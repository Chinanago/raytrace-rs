//! The session: the open document, its revision, the undo history and the change log.
//!
//! The document is an immutable value shared by reference. A change never edits it in place:
//! the command builds a new [`Design`], the session validates it, and only then does it
//! replace the document, keep the old one for undo, clear the redo history and bump the
//! revision. A change that fails leaves the document, the history and the revision as they
//! were. Opening or creating a document starts a new history.
//!
//! Changes that carry the same `group` in a row share one undo step, and the session keeps
//! its history within [`HistoryLimits`]. The committed changes are recorded as
//! [`LogEntry`]s, so the same document can be rebuilt by running them in a new session.
use crate::{
    engine::{DocSummary, EngineError, LogEntry, RunStatus, SessionLog, SessionStatus},
    footprint,
};
use raytrace_io::{
    design::{self, Design},
    record::RunRecord,
};
use std::{path::PathBuf, sync::Arc};

/// How much undo history a session keeps. The oldest steps are dropped first; the newest
/// step is always kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryLimits {
    /// Most undo steps.
    pub steps: usize,
    /// Most estimated bytes of the documents held by the undo steps.
    pub bytes: usize,
}
impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            steps: 64,
            bytes: 128 * 1024 * 1024,
        }
    }
}

/// One undo or redo step: the document that undo (or redo) brings back, and the log entries
/// of the change(s) it undoes.
#[derive(Clone, Debug)]
struct Step {
    design: Arc<Design>,
    /// Estimated size of `design`.
    bytes: usize,
    entries: Vec<LogEntry>,
}
impl Step {
    fn new(design: Arc<Design>, entries: Vec<LogEntry>) -> Self {
        Self {
            bytes: footprint::design_bytes(&design),
            design,
            entries,
        }
    }
    /// Add a change to the step. A `doc.set` that rewrites the place of the previous entry
    /// replaces it, so a long drag leaves one entry instead of one per mouse move.
    fn record(&mut self, entry: LogEntry) {
        if let Some(last) = self.entries.last_mut()
            && entry.command == "doc.set"
            && last.command == entry.command
            && last.params.get("path") == entry.params.get("path")
        {
            *last = entry;
        } else {
            self.entries.push(entry);
        }
    }
}

/// The command that is running, set by the registry around a change command.
#[derive(Clone, Debug)]
struct Pending {
    entry: LogEntry,
    group: Option<String>,
}

/// The latest calculation record of a session and the document revision it was computed from.
#[derive(Clone, Debug)]
pub struct LatestRun {
    pub record: Arc<RunRecord>,
    /// The session revision of the document the record was computed from. `None` when the
    /// record does not come from the session document: a record opened from a file, a replay,
    /// or a trace of a `model` given to the command.
    pub revision: Option<u64>,
}

/// A document under edit. Create one per front end (the CLI makes one per process, or per
/// `run` stream), then call commands with [`Session::execute`] or [`Session::call`].
#[derive(Clone, Debug, Default)]
pub struct Session {
    design: Option<Arc<Design>>,
    path: Option<PathBuf>,
    revision: u64,
    /// The document as it was opened or last saved.
    saved: Option<Arc<Design>>,
    dirty: bool,
    undo: Vec<Step>,
    redo: Vec<Step>,
    limits: HistoryLimits,
    /// The `doc.open` or `doc.new` that started the document.
    base: Option<LogEntry>,
    /// Entries of changes that no longer have an undo step (dropped by the limits, or made
    /// before the session had a document to keep).
    older: Vec<LogEntry>,
    /// The group of the last committed change, while it is the last thing that happened.
    open_group: Option<String>,
    pending: Option<Pending>,
    catalogs: Vec<PathBuf>,
    /// The latest calculation record; only this one is kept.
    run: Option<LatestRun>,
}

impl Session {
    /// A session without a document, at revision 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The open document.
    pub fn design(&self) -> Option<&Arc<Design>> {
        self.design.as_ref()
    }
    /// The open document, or the `no_document` error.
    pub fn require_design(&self) -> Result<&Arc<Design>, EngineError> {
        self.design.as_ref().ok_or_else(EngineError::no_document)
    }
    /// The file the document was opened from or last saved to.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
    /// Increases with every change of the document, including undo and redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// The document differs from the saved file.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// AGF catalog folders used when a command imports a ZMX without its own.
    pub fn catalogs(&self) -> &[PathBuf] {
        &self.catalogs
    }

    /// The latest calculation record (`analysis.trace`, `run.replay` or `run.open`), which the
    /// `analysis.*` commands use when they are given no `run`.
    pub fn latest_run(&self) -> Option<&LatestRun> {
        self.run.as_ref()
    }
    /// Keep `record` as the latest calculation record, replacing the previous one.
    /// `revision` is the revision of the document it was computed from, if any.
    pub fn set_latest_run(&mut self, record: Arc<RunRecord>, revision: Option<u64>) {
        self.run = Some(LatestRun { record, revision });
    }
    /// The latest record was computed from a document that has changed since.
    pub fn run_is_stale(&self) -> bool {
        self.run
            .as_ref()
            .and_then(|run| run.revision)
            .is_some_and(|revision| revision != self.revision)
    }

    /// Reject a command that was prepared for another revision.
    pub fn check_revision(&self, expected: Option<u64>) -> Result<(), EngineError> {
        match expected {
            Some(expected) if expected != self.revision => Err(EngineError::new(
                "revision",
                format!(
                    "文書の版が一致しません（指定: {expected}、現在: {}）。session.status で現在の版を確認してください",
                    self.revision
                ),
            )),
            _ => Ok(()),
        }
    }

    /// The limits of the undo history.
    pub fn history_limits(&self) -> HistoryLimits {
        self.limits
    }
    /// Change the limits of the undo history; steps over the new limits are dropped now.
    pub fn set_history_limits(&mut self, limits: HistoryLimits) {
        self.limits = limits;
        self.trim();
    }
    /// Estimated bytes held by the undo steps.
    pub fn undo_bytes(&self) -> usize {
        self.undo.iter().map(|step| step.bytes).sum()
    }

    /// The changes that lead from the opened document to the current one.
    pub fn log(&self) -> SessionLog {
        SessionLog {
            revision: self.revision,
            base: self.base.clone(),
            entries: self
                .older
                .iter()
                .chain(self.undo.iter().flat_map(|step| &step.entries))
                .cloned()
                .collect(),
        }
    }

    /// Record the command that is about to run, so that the change it commits is logged.
    /// Called by the registry around every change command.
    pub(crate) fn begin_command(&mut self, entry: LogEntry, group: Option<String>) {
        self.pending = Some(Pending { entry, group });
    }
    /// The command is over; whatever it did not commit is not logged.
    pub(crate) fn end_command(&mut self) {
        self.pending = None;
    }

    /// Replace the document by an opened or new one and start a new history and log.
    /// The document starts without unsaved changes.
    pub fn open_design(
        &mut self,
        design: Design,
        path: Option<PathBuf>,
        catalogs: Vec<PathBuf>,
    ) -> Result<(), EngineError> {
        design::check(&design)?;
        let design = Arc::new(design);
        self.saved = Some(design.clone());
        self.design = Some(design);
        self.path = path;
        self.update_dirty();
        if !catalogs.is_empty() {
            self.catalogs = catalogs;
        }
        self.undo.clear();
        self.redo.clear();
        self.older.clear();
        self.base = self.pending.take().map(|p| p.entry);
        self.open_group = None;
        self.revision += 1;
        Ok(())
    }

    /// Take over the document, history and log of `other`: a session that opened the
    /// document, or edited a copy of this one ([`Clone`]), on another thread. The revision
    /// keeps rising; the latest calculation record of `other` comes along.
    pub fn adopt(&mut self, other: Session) {
        let revision = (self.revision + 1).max(other.revision);
        let catalogs = std::mem::take(&mut self.catalogs);
        let other_revision = other.revision;
        *self = other;
        if self.catalogs.is_empty() {
            self.catalogs = catalogs;
        }
        self.revision = revision;
        // A record computed from the document that is adopted stays current.
        if let Some(run) = &mut self.run
            && run.revision == Some(other_revision)
        {
            run.revision = Some(revision);
        }
    }

    /// Change the document. `build` receives the current document and returns the new one (and
    /// a value for the caller); the new document is validated before anything changes.
    pub fn edit<T>(
        &mut self,
        expected_revision: Option<u64>,
        build: impl FnOnce(&Design) -> Result<(Design, T), EngineError>,
    ) -> Result<T, EngineError> {
        self.check_revision(expected_revision)?;
        let current = self.require_design()?.clone();
        let (next, value) = build(&current)?;
        self.commit(next)?;
        Ok(value)
    }

    /// Replace the whole document as one undoable change. A session without a document
    /// starts one.
    pub fn replace(
        &mut self,
        expected_revision: Option<u64>,
        design: Design,
    ) -> Result<(), EngineError> {
        self.check_revision(expected_revision)?;
        self.commit(design)
    }

    fn commit(&mut self, next: Design) -> Result<(), EngineError> {
        design::check(&next)?;
        let (entry, group) = match self.pending.take() {
            Some(Pending { entry, group }) => (Some(entry), group),
            None => (None, None),
        };
        let joins_last_step = group.is_some() && self.open_group == group && !self.undo.is_empty();
        if joins_last_step {
            if let (Some(step), Some(entry)) = (self.undo.last_mut(), entry) {
                step.record(entry);
            }
        } else if let Some(old) = self.design.take() {
            self.undo.push(Step::new(old, entry.into_iter().collect()));
            self.trim();
        } else {
            self.older.extend(entry);
        }
        self.open_group = if self.undo.is_empty() { None } else { group };
        self.redo.clear();
        self.design = Some(Arc::new(next));
        self.update_dirty();
        self.revision += 1;
        Ok(())
    }

    /// Drop the oldest undo steps while the history is over its limits. Their log entries
    /// stay in the log.
    fn trim(&mut self) {
        let mut bytes = self.undo_bytes();
        let mut dropped = 0;
        while self.undo.len() - dropped > 1
            && (self.undo.len() - dropped > self.limits.steps || bytes > self.limits.bytes)
        {
            bytes = bytes.saturating_sub(self.undo[dropped].bytes);
            dropped += 1;
        }
        for step in self.undo.drain(..dropped) {
            self.older.extend(step.entries);
        }
    }

    /// Bring back the document before the last change.
    pub fn undo(&mut self, expected_revision: Option<u64>) -> Result<(), EngineError> {
        self.check_revision(expected_revision)?;
        let Some(previous) = self.undo.pop() else {
            return Err(EngineError::new("history", "取り消せる操作がありません"));
        };
        self.swap(previous, true);
        Ok(())
    }

    /// Bring back the change that was undone last.
    pub fn redo(&mut self, expected_revision: Option<u64>) -> Result<(), EngineError> {
        self.check_revision(expected_revision)?;
        let Some(next) = self.redo.pop() else {
            return Err(EngineError::new("history", "やり直せる操作がありません"));
        };
        self.swap(next, false);
        Ok(())
    }

    /// Exchange the document with the one of `target`. The step that goes to the other
    /// stack holds the document that was current and the same log entries.
    fn swap(&mut self, target: Step, undoing: bool) {
        let current = self.design.replace(target.design);
        if let Some(current) = current {
            let step = Step::new(current, target.entries);
            if undoing {
                self.redo.push(step);
            } else {
                self.undo.push(step);
            }
        }
        self.open_group = None;
        self.update_dirty();
        self.revision += 1;
    }

    /// The document differs from the saved one unless it is the very same value.
    fn update_dirty(&mut self) {
        self.dirty = match (&self.design, &self.saved) {
            (Some(design), Some(saved)) => !Arc::ptr_eq(design, saved),
            (Some(_), None) => true,
            (None, _) => false,
        };
    }

    /// The document was written to `path`.
    pub fn mark_saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.saved = self.design.clone();
        self.update_dirty();
    }

    /// The state of the session for `session.status`.
    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            revision: self.revision,
            has_document: self.design.is_some(),
            path: self.path.clone(),
            dirty: self.dirty,
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            undo_depth: self.undo.len(),
            redo_depth: self.redo.len(),
            undo_bytes: self.undo_bytes(),
            catalogs: self.catalogs.clone(),
            document: self.design.as_deref().map(DocSummary::of),
            run: self.run.as_ref().map(|run| RunStatus {
                revision: run.revision,
                stale: self.run_is_stale(),
                detectors: run.record.report.detectors.len(),
            }),
        }
    }
}
