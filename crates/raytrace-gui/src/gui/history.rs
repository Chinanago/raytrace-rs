use super::*;
use std::collections::VecDeque;

#[derive(Clone)]
pub(super) struct Snapshot {
    explicit_record: bool,
    design: Design,
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
    next: u64,
    saved: Option<u64>,
}
impl History {
    pub fn begin(&mut self) {
        if !self.pending {
            self.pending = true;
            self.next += 1;
            self.revision = self.next;
            self.future.clear();
        }
    }
    pub fn can_undo(&self) -> bool {
        self.pending || !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    fn trim(&mut self) {
        let mut bytes: usize = self.past.iter().map(|s| design_bytes(&s.design)).sum();
        while self.past.len() > 1 && (self.past.len() > 64 || bytes > 128 * 1024 * 1024) {
            if let Some(s) = self.past.pop_front() {
                bytes = bytes.saturating_sub(design_bytes(&s.design));
            }
        }
    }
}
// Geometry vectors dominate CAD snapshot size. No serialization or triangle walk is needed.
fn coating_bytes(c: &raytrace_core::coating::Coating) -> usize {
    let size = |m: &raytrace_core::coating::CoatingModel| match m {
        raytrace_core::coating::CoatingModel::Table {
            wavelengths_um,
            angles_deg,
            coefficients,
            ..
        } => (wavelengths_um.len() + angles_deg.len()) * 8 + coefficients.len() * 16,
        _ => 0,
    };
    c.name.len()
        + c.zemax_definition.as_ref().map_or(0, |s| s.len())
        + size(&c.front)
        + c.back.as_ref().map_or(0, size)
}
fn scene_bytes(scene: &Scene) -> usize {
    scene.sources.len() * size_of::<Source>()
        + scene
            .sources
            .iter()
            .filter_map(|s| s.spectrum.as_ref())
            .map(|s| s.samples.len() * 16)
            .sum::<usize>()
        + scene.materials.len() * 1024
        + scene
            .surfaces
            .iter()
            .map(|s| {
                size_of::<Surface>()
                    + s.name.len()
                    + s.coating.as_ref().map_or(0, coating_bytes)
                    + match &s.shape {
                        Shape::TriangleMesh {
                            vertices,
                            triangles,
                            normals,
                        } => {
                            (vertices.len() + normals.len()) * size_of::<V3>()
                                + triangles.len() * 12
                        }
                        Shape::EvenAsphere { coefficients, .. }
                        | Shape::FlatEvenAsphere { coefficients } => coefficients.len() * 8,
                        _ => 0,
                    }
            })
            .sum::<usize>()
}
// The provenance is opaque JSON; arrays are assumed to hold similar entries so that a
// large mesh costs one step per nesting level instead of one per triangle.
fn value_bytes(v: &serde_json::Value) -> usize {
    use serde_json::Value;
    match v {
        Value::String(s) => s.len(),
        Value::Array(a) => a
            .first()
            .map_or(0, |first| a.len() * (value_bytes(first) + 16)),
        Value::Object(o) => o.iter().map(|(k, v)| k.len() + value_bytes(v) + 16).sum(),
        _ => 16,
    }
}
fn design_bytes(d: &Design) -> usize {
    d.parts
        .iter()
        .map(|p| {
            scene_bytes(&p.scene)
                + p.provenance
                    .as_ref()
                    .map_or(0, |o| value_bytes(&o.document))
                + p.rod
                    .as_ref()
                    .map_or(0, |r| r.coatings.values().map(coating_bytes).sum::<usize>())
        })
        .sum()
}
impl App {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            explicit_record: self.viewing_explicit_record,
            design: self.design.clone(),
            preview: self.preview_limit,
            component: self.component,
            surface: self.selected,
            source: self.source,
            source_selected: self.source_selected,
            revision: self.history.revision,
        }
    }
    pub(super) fn reset_history(&mut self, saved: bool) {
        self.history = History::default();
        self.history.saved = saved.then_some(0);
        self.history.baseline = Some(self.snapshot());
    }
    pub(super) fn commit_edit(&mut self) {
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
        self.design = state.design.clone();
        self.recompile();
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
        let original = serde_json::to_value(&app.design).unwrap();
        let scene = app.design.trace_scene().unwrap();
        let p = scene.clone().prepare().unwrap();
        let rays = p.generate_rays().unwrap();
        app.result = Some(ResultView {
            revision: 0,
            calculation: RunContext::capture(&app.design, &scene, None, 1, TraceOptions::default()),
            scene,
            report: trace(&p, &rays, TraceOptions::default()),
            seconds: 0.0,
            texture: None,
            screen: None,
            roi_cache: None,
        });
        for _ in 0..3 {
            app.design.occurrences[2].pose.translation_mm.0[2] += 1.0;
            app.refresh_components();
        }
        assert!(app.stale);
        app.commit_edit();
        assert_eq!(app.history.past.len(), 1);
        app.undo();
        assert_eq!(serde_json::to_value(&app.design).unwrap(), original);
        assert!(!app.stale);
        assert!(!app.dirty);
        app.redo();
        assert!(app.stale);
        app.undo();
        app.design.occurrences[1].enabled = false;
        app.refresh_components();
        app.commit_edit();
        assert!(!app.history.can_redo());
        assert_ne!(app.history.revision, 1);
    }
}
