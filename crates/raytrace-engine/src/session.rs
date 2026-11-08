//! The session: the open document, its revision and the undo history.
//!
//! The document is an immutable value shared by reference. A change never edits it in place:
//! the command builds a new [`Design`], the session validates it, and only then does it
//! replace the document, keep the old one for undo, clear the redo history and bump the
//! revision. A change that fails leaves the document, the history and the revision as they
//! were. Opening or creating a document starts a new history.
use crate::engine::{DocSummary, EngineError, SessionStatus};
use raytrace_io::design::{self, Design};
use std::{path::PathBuf, sync::Arc};

/// The document state that undo and redo bring back. Documents are shared, so a snapshot
/// costs a reference count.
#[derive(Clone, Debug)]
struct Snapshot {
    design: Arc<Design>,
}

/// A document under edit. Create one per front end (the CLI makes one per process, or per
/// `run` stream), then call commands with [`Session::execute`] or [`Session::call`].
#[derive(Debug, Default)]
pub struct Session {
    design: Option<Arc<Design>>,
    path: Option<PathBuf>,
    revision: u64,
    /// The document as it was opened or last saved.
    saved: Option<Arc<Design>>,
    dirty: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    catalogs: Vec<PathBuf>,
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

    /// Replace the document by an opened or new one and start a new history.
    pub fn open_design(
        &mut self,
        design: Design,
        path: Option<PathBuf>,
        catalogs: Vec<PathBuf>,
    ) -> Result<(), EngineError> {
        design::check(&design)?;
        let design = Arc::new(design);
        self.saved = path.is_some().then(|| design.clone());
        self.design = Some(design);
        self.path = path;
        self.update_dirty();
        if !catalogs.is_empty() {
            self.catalogs = catalogs;
        }
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        Ok(())
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
        if let Some(old) = self.design.take() {
            self.undo.push(Snapshot { design: old });
        }
        self.redo.clear();
        self.design = Some(Arc::new(next));
        self.update_dirty();
        self.revision += 1;
        Ok(())
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

    fn swap(&mut self, target: Snapshot, undoing: bool) {
        let current = self
            .design
            .replace(target.design)
            .map(|design| Snapshot { design });
        if let Some(current) = current {
            if undoing {
                self.redo.push(current);
            } else {
                self.undo.push(current);
            }
        }
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
            catalogs: self.catalogs.clone(),
            document: self.design.as_deref().map(DocSummary::of),
        }
    }
}
