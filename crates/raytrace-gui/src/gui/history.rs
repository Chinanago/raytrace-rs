use super::*;
use std::collections::VecDeque;

#[derive(Clone)]
pub(super) struct Snapshot {
    explicit_record: bool,
    preview: Option<usize>,
    component: usize,
    surface: usize,
    source: usize,
    source_selected: bool,
    revision: u64,
}
#[derive(Default)]
pub(super) struct History {
    baseline: Option<Snapshot>,
    past: VecDeque<Snapshot>,
    future: Vec<Snapshot>,
    pending: bool,
    pub revision: u64,
    saved: Option<u64>,
}
impl History {
    pub fn begin(&mut self, revision: u64) {
        self.revision = revision;
        if !self.pending {
            self.pending = true;
            self.future.clear();
        }
    }
    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        self.pending || !self.past.is_empty()
    }
    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    fn trim(&mut self) {
        while self.past.len() > 64 {
            self.past.pop_front();
        }
    }
}
impl App {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            explicit_record: self.viewing_explicit_record,
            preview: self.preview_limit,
            component: self.component,
            surface: self.selected,
            source: self.source,
            source_selected: self.source_selected,
            revision: self.history.revision,
        }
    }
    pub(super) fn reset_history(&mut self, saved: bool) {
        self.engine
            .reset_project(self.project.clone())
            .expect("valid editor project");
        self.engine.set_catalogs(self.catalogs.clone());
        self.project = self.engine.project().clone();
        self.reset_presentation_history(saved);
    }
    pub(super) fn reset_presentation_history(&mut self, saved: bool) {
        self.history = History::default();
        self.history.revision = self.engine.revision();
        self.history.saved = saved.then_some(self.engine.revision());
        self.history.baseline = Some(self.snapshot());
    }
    pub(super) fn commit_edit(&mut self) {
        self.engine.end_gesture();
        if self.history.pending {
            if let Some(previous) = self.history.baseline.take() {
                self.history.past.push_back(previous);
            }
            self.history.baseline = Some(self.snapshot());
            self.history.pending = false;
            self.history.trim();
        } else if let Some(s) = &mut self.history.baseline {
            s.component = self.component;
            s.surface = self.selected;
            s.source = self.source;
            s.source_selected = self.source_selected;
        }
    }
    fn restore_snapshot(&mut self, state: &Snapshot) {
        self.viewing_explicit_record = state.explicit_record;
        self.project = self.engine.project().clone();
        self.preview_limit = state.preview;
        self.component = state.component;
        self.selected = state.surface;
        self.source = state.source;
        self.source_selected = state.source_selected;
        self.history.revision = state.revision;
        self.dirty = self.history.saved != Some(state.revision);
        self.stale = self
            .result
            .as_ref()
            .is_some_and(|r| r.revision != state.revision);
        self.error = None;
        self.status = if self.stale {
            "配置を復元しました・計算すると結果が更新されます"
        } else {
            "配置を復元しました"
        }
        .into();
    }
    pub(super) fn undo(&mut self) {
        self.commit_edit();
        if !self.engine.can_undo() {
            return;
        }
        if let Err(e) = self.engine.apply(raytrace_engine::Request {
            expected_revision: None,
            command: raytrace_engine::Command::Undo {},
        }) {
            self.error = Some(e.to_string());
            return;
        }
        if let Some(previous) = self.history.past.pop_back() {
            if let Some(current) = self.history.baseline.take() {
                self.history.future.push(current);
            }
            self.restore_snapshot(&previous);
            self.history.baseline = Some(previous);
        }
    }
    pub(super) fn redo(&mut self) {
        self.commit_edit();
        if !self.engine.can_redo() {
            return;
        }
        if let Err(e) = self.engine.apply(raytrace_engine::Request {
            expected_revision: None,
            command: raytrace_engine::Command::Redo {},
        }) {
            self.error = Some(e.to_string());
            return;
        }
        if let Some(next) = self.history.future.pop() {
            if let Some(current) = self.history.baseline.take() {
                self.history.past.push_back(current);
            }
            self.restore_snapshot(&next);
            self.history.baseline = Some(next);
        }
    }
    pub(super) fn mark_saved(&mut self) {
        self.commit_edit();
        self.history.saved = Some(self.history.revision);
        self.dirty = false;
    }
    pub(super) fn history_shortcuts(&mut self, ctx: &egui::Context) {
        if self.job.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let redo = ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Y,
            ))
        });
        if redo {
            self.redo();
        } else if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Z,
            ))
        }) {
            self.undo();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raytrace_core::trace::{TraceOptions, trace};
    #[test]
    fn edits_coalesce_until_release_and_undo_restores_the_matching_result_revision() {
        let mut app = App::demo();
        let original = serde_json::to_value(&app.project).unwrap();
        let scene = app.project.trace_scene().unwrap();
        let p = scene.clone().prepare().unwrap();
        let rays = p.generate_rays().unwrap();
        app.result = Some(ResultView {
            revision: 0,
            calculation: RunContext::capture(
                &app.project,
                &scene,
                None,
                1,
                TraceOptions::default(),
            ),
            scene,
            report: trace(&p, &rays, TraceOptions::default()),
            seconds: 0.0,
            texture: None,
            screen: None,
            roi_cache: None,
        });
        for _ in 0..3 {
            app.project.assembly.as_mut().unwrap().components[2]
                .pose
                .translation_mm
                .0[2] += 1.0;
            app.refresh_components();
        }
        assert!(app.stale);
        app.commit_edit();
        assert_eq!(app.history.past.len(), 1);
        app.undo();
        assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
        assert!(!app.stale);
        assert!(!app.dirty);
        app.redo();
        assert!(app.stale);
        app.undo();
        app.project.assembly.as_mut().unwrap().components[1].enabled = false;
        app.refresh_components();
        app.commit_edit();
        assert!(!app.history.can_redo());
        assert_ne!(app.history.revision, 1);
    }
}
