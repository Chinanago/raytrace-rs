//! Editing the document. The GUI never writes to the `Design`: every edit is a command run
//! on the session (`doc.set`, `component.*`, `session.undo`/`redo`), so the session owns the
//! undo history and the change log that an agent can read with `session.log`.
//!
//! The widgets edit a small working copy of the part they show (a pose, a rod, the sources of
//! a part). After the widgets ran, [`Edits::diff`] compares the copy with the document and
//! turns every changed value into one `doc.set`. All the changes of one gesture (a drag, the
//! typing in one field, one click) carry the same `group`, which makes them one undo step.
use super::*;
use raytrace_engine::{
    EngineError,
    engine::{DocNewRequest, DocSetRequest, HistoryRequest},
};
use serde::Serialize;
use serde_json::Value;
use std::sync::{OnceLock, Weak};

/// Which part, surface and source the sidebar shows. It is remembered for every document
/// the user leaves, so that undo brings the selection back with the document.
#[derive(Clone, Copy)]
pub(super) struct Selection {
    component: usize,
    surface: usize,
    source: usize,
    source_selected: bool,
}

/// The document of a session, or an empty one while the session has none.
pub(super) fn current(session: &Session) -> &Design {
    static EMPTY: OnceLock<Design> = OnceLock::new();
    session
        .design()
        .map_or_else(|| EMPTY.get_or_init(Design::empty), |design| &**design)
}

/// The `doc.set` calls an editing pass wants to make, collected while the document is
/// borrowed for reading.
#[derive(Default)]
pub(super) struct Edits(Vec<(String, Value)>);

impl Edits {
    /// Set the value at `path` (a JSON Pointer into the design).
    pub fn set<T: Serialize>(&mut self, path: impl Into<String>, value: &T) {
        if let Ok(value) = serde_json::to_value(value) {
            self.0.push((path.into(), value));
        }
    }
    /// Set every value at which `after` differs from `before`. `base` is the JSON Pointer of
    /// the place where `before` is stored in the design.
    pub fn diff<T: Serialize>(&mut self, base: &str, before: &T, after: &T) {
        if let (Ok(before), Ok(after)) = (serde_json::to_value(before), serde_json::to_value(after))
        {
            changes(base, &before, &after, &mut self.0);
        }
    }
    /// Add the edits of `other` after these.
    pub fn extend(&mut self, other: Edits) {
        self.0.extend(other.0);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// The smallest set of `(pointer, value)` that turns `before` into `after`: objects and
/// arrays of the same shape are compared member by member, anything else is set whole.
fn changes(path: &str, before: &Value, after: &Value, out: &mut Vec<(String, Value)>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(a), Value::Object(b))
            if a.len() == b.len() && a.keys().all(|key| b.contains_key(key)) =>
        {
            for (key, value) in b {
                if let Some(old) = a.get(key) {
                    changes(&format!("{path}/{}", escape(key)), old, value, out);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (old, value)) in a.iter().zip(b).enumerate() {
                changes(&format!("{path}/{index}"), old, value, out);
            }
        }
        _ => out.push((path.to_string(), after.clone())),
    }
}

impl App {
    /// The document the session holds.
    pub(super) fn design(&self) -> &Design {
        current(&self.session)
    }
    pub(super) fn is_dirty(&self) -> bool {
        self.session.is_dirty()
    }
    pub(super) fn can_undo(&self) -> bool {
        self.session.can_undo()
    }
    pub(super) fn can_redo(&self) -> bool {
        self.session.can_redo()
    }
    /// The result on screen was calculated from another document (or with another ray
    /// limit) than the one the session holds now. Undo back to the calculated document and
    /// the result is current again.
    pub(super) fn is_stale(&self) -> bool {
        self.result.as_ref().is_some_and(|r| {
            r.preview_limit != self.preview_limit
                || self
                    .session
                    .design()
                    .is_none_or(|design| !Arc::ptr_eq(design, &r.design))
        })
    }

    /// Compile the document again when the session moved on. A document the session accepted
    /// compiles, so a failure here only reports the reason and keeps the previous scene.
    pub(super) fn sync_compiled(&mut self) {
        let revision = self.session.revision();
        if self.compiled_revision == Some(revision) {
            return;
        }
        self.compiled_revision = Some(revision);
        match self.design().compile() {
            Ok(compiled) => self.compiled = compiled,
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The `group` of the changes of the current gesture.
    pub(super) fn group(&mut self) -> String {
        self.gesture_used = true;
        format!("gui:{}", self.gesture)
    }
    /// The gesture is over: the next change starts a new undo step.
    pub(super) fn end_gesture(&mut self) {
        if self.gesture_used {
            self.gesture += 1;
            self.gesture_used = false;
        }
    }

    /// A frame is over. With no mouse button held and no text field focused the gesture that
    /// made the changes of this frame is finished.
    pub(super) fn finish_frame(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input() {
            self.end_gesture();
        }
    }

    /// Run a command on the session and show its error. The compiled scene follows.
    pub(super) fn execute<In: 'static, Out: 'static>(
        &mut self,
        handle: raytrace_engine::Cmd<In, Out>,
        input: In,
    ) -> Option<Out> {
        self.remember_selection();
        let result: Result<Out, EngineError> = self.session.run(handle, input);
        self.sync_compiled();
        match result {
            Ok(out) => Some(out),
            Err(e) => {
                self.error = Some(e.message);
                None
            }
        }
    }

    /// Apply the collected edits as `doc.set` commands of the current gesture.
    pub(super) fn apply(&mut self, edits: Edits) -> bool {
        if edits.is_empty() {
            return false;
        }
        let group = self.group();
        for (path, value) in edits.0 {
            let request = DocSetRequest {
                path,
                value,
                group: Some(group.clone()),
                expected_revision: None,
            };
            if self.execute(commands::DOC_SET, request).is_none() {
                return false;
            }
        }
        true
    }

    /// Start a new document (empty, or the given design) with a new history.
    pub(super) fn start_document(&mut self, design: Option<Design>) -> bool {
        self.end_gesture();
        let request = DocNewRequest {
            design: design.map(Box::new),
            ..DocNewRequest::default()
        };
        let started = self.execute(commands::DOC_NEW, request).is_some();
        self.selections.clear();
        started
    }

    fn selection(&self) -> Selection {
        Selection {
            component: self.component,
            surface: self.selected,
            source: self.source,
            source_selected: self.source_selected,
        }
    }
    fn remember_selection(&mut self) {
        let Some(design) = self.session.design() else {
            return;
        };
        let weak = Arc::downgrade(design);
        let selection = self.selection();
        if self.selections.len() > 64 {
            self.selections.retain(|(w, _)| w.strong_count() > 0);
        }
        match self
            .selections
            .iter_mut()
            .find(|(w, _)| Weak::ptr_eq(w, &weak))
        {
            Some(entry) => entry.1 = selection,
            None => self.selections.push((weak, selection)),
        }
    }
    fn restore_selection(&mut self) {
        let Some(design) = self.session.design() else {
            return;
        };
        let weak = Arc::downgrade(design);
        if let Some((_, s)) = self.selections.iter().find(|(w, _)| Weak::ptr_eq(w, &weak)) {
            self.component = s.component;
            self.selected = s.surface;
            self.source = s.source;
            self.source_selected = s.source_selected;
        }
    }

    pub(super) fn undo(&mut self) {
        self.step_history(true);
    }
    pub(super) fn redo(&mut self) {
        self.step_history(false);
    }
    fn step_history(&mut self, undo: bool) {
        self.end_gesture();
        self.remember_selection();
        let result = if undo {
            self.session
                .run(commands::SESSION_UNDO, HistoryRequest::default())
        } else {
            self.session
                .run(commands::SESSION_REDO, HistoryRequest::default())
        };
        // Nothing to undo or redo leaves everything as it was.
        if result.is_ok() {
            self.sync_compiled();
            self.restore_selection();
            self.error = None;
            self.status = if self.is_stale() {
                "配置を復元しました・計算すると結果が更新されます"
            } else {
                "配置を復元しました"
            }
            .into();
        }
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
    use crate::gui::test_support::*;
    use raytrace_engine::Context as EngineContext;
    use serde_json::json;

    #[test]
    fn changes_name_the_smallest_places_that_differ() {
        let before = json!({"a": [1, 2, 3], "b": {"x": 1.0, "y": 2.0}, "c": "s", "d/e": 4});
        let mut after = before.clone();
        after["a"][1] = json!(9);
        after["b"]["y"] = json!(3.0);
        after["d/e"] = json!(5);
        let mut out = Vec::new();
        changes("/p", &before, &after, &mut out);
        assert_eq!(
            out,
            vec![
                ("/p/a/1".to_string(), json!(9)),
                ("/p/b/y".to_string(), json!(3.0)),
                ("/p/d~1e".to_string(), json!(5)),
            ]
        );
        // Another shape is set whole.
        let mut out = Vec::new();
        changes(
            "/p",
            &json!({"a": [1, 2]}),
            &json!({"a": [1, 2, 3]}),
            &mut out,
        );
        assert_eq!(out, vec![("/p/a".to_string(), json!([1, 2, 3]))]);
        let mut out = Vec::new();
        changes("/p", &json!({"a": 1}), &json!({"b": 1}), &mut out);
        assert_eq!(out, vec![("/p".to_string(), json!({"b": 1}))]);
        let mut out = Vec::new();
        changes("/p", &json!({"a": 1}), &json!({"a": 1}), &mut out);
        assert!(out.is_empty());
    }

    fn move_z(app: &mut App, index: usize, by: f64) {
        let o = &app.design().occurrences[index];
        let mut edits = Edits::default();
        edits.set(
            format!("/occurrences/@{}/pose/translation_mm/2", o.id.as_str()),
            &(o.pose.translation_mm.0[2] + by),
        );
        assert!(app.apply(edits), "{:?}", app.error);
    }

    fn wait(app: &mut App, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.job.is_some() {
            assert!(Instant::now() < deadline, "background job timed out");
            std::thread::sleep(Duration::from_millis(5));
            app.poll(ctx);
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    /// The document that running the log of `session` on a new session gives.
    fn replayed(session: &Session) -> Session {
        let log = session.log();
        let mut fresh = Session::new();
        let control = TraceControl::default();
        for entry in log.base.iter().chain(&log.entries) {
            let response = fresh.execute(
                &entry.command,
                entry.params.clone(),
                &mut EngineContext::new(&control),
            );
            assert!(response.ok, "{}: {:?}", entry.command, response.error);
        }
        fresh
    }

    fn design_json(session: &Session) -> String {
        serde_json::to_string_pretty(session.design().unwrap().as_ref()).unwrap()
    }

    #[test]
    fn edits_of_one_gesture_are_one_undo_step_and_undo_restores_the_matching_result() {
        let mut app = App::demo();
        let original = serde_json::to_value(app.design()).unwrap();
        calculate_demo(&mut app);
        assert!(!app.is_stale());
        for _ in 0..3 {
            move_z(&mut app, 2, 1.0);
        }
        assert!(app.is_stale());
        assert!(app.is_dirty());
        app.end_gesture();
        assert_eq!(app.session.status().undo_depth, 1);
        app.undo();
        assert_eq!(serde_json::to_value(app.design()).unwrap(), original);
        assert!(!app.is_stale(), "the result belongs to this document again");
        assert!(!app.is_dirty());
        app.redo();
        assert!(app.is_stale());
        app.undo();
        // A new edit clears what could be redone.
        let mut edits = Edits::default();
        edits.set(
            format!(
                "/occurrences/@{}/enabled",
                app.design().occurrences[1].id.as_str()
            ),
            &false,
        );
        assert!(app.apply(edits));
        app.end_gesture();
        assert!(!app.can_redo());
        assert!(app.can_undo());
    }

    #[test]
    fn a_second_gesture_is_a_second_undo_step() {
        let mut app = App::demo();
        move_z(&mut app, 2, 1.0);
        move_z(&mut app, 2, 1.0);
        app.end_gesture();
        move_z(&mut app, 2, 1.0);
        app.end_gesture();
        assert_eq!(app.session.status().undo_depth, 2);
        app.undo();
        assert_eq!(app.session.status().undo_depth, 1);
        assert_eq!(app.session.status().redo_depth, 1);
    }

    #[test]
    fn undo_brings_back_the_selection_that_went_with_the_document() {
        let mut app = App::demo();
        app.component = 1;
        move_z(&mut app, 1, 2.0);
        app.end_gesture();
        app.component = 3;
        app.undo();
        assert_eq!(app.component, 1);
        assert!(app.status.contains("復元"));
    }

    #[test]
    fn a_failed_edit_leaves_the_document_alone_and_reports_it() {
        let mut app = App::demo();
        let before = serde_json::to_value(app.design()).unwrap();
        let mut edits = Edits::default();
        edits.set("/occurrences/@nope/enabled", &false);
        assert!(!app.apply(edits));
        assert!(app.error.is_some());
        assert_eq!(serde_json::to_value(app.design()).unwrap(), before);
        assert!(!app.can_undo());
    }

    #[test]
    fn dragging_a_marker_is_one_undo_step_and_the_log_replays_to_the_same_design() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let start = serde_json::to_value(app.design()).unwrap();
        let out = frame(&mut app, &ctx, vec![]);
        let p = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Circle(c) if c.radius == 6.0 => Some(c.center),
                _ => None,
            })
            .expect("a part marker");
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let mut at = p;
        for _ in 0..4 {
            at += Vec2::new(12.0, 0.0);
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        }
        assert!(app.is_dirty());
        assert_eq!(
            app.session.status().undo_depth,
            1,
            "the drag is in progress"
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let dragged = serde_json::to_value(app.design()).unwrap();
        assert_ne!(dragged, start);
        let status = app.session.status();
        assert_eq!(status.undo_depth, 1, "a drag is one undo step");
        assert!(status.revision > 2, "every mouse move was a change");
        let log = app.session.log();
        assert_eq!(log.base.as_ref().unwrap().command, "doc.new");
        assert_eq!(log.entries.len(), 1, "the moves of a drag are one entry");
        assert_eq!(log.entries[0].command, "doc.set");
        assert!(
            log.entries[0].params["path"]
                .as_str()
                .unwrap()
                .ends_with("/pose/translation_mm")
        );
        // A second drag is a second step.
        move_z(&mut app, 2, 5.0);
        app.end_gesture();
        assert_eq!(app.session.status().undo_depth, 2);
        app.undo();
        assert_eq!(serde_json::to_value(app.design()).unwrap(), dragged);
        app.undo();
        assert_eq!(serde_json::to_value(app.design()).unwrap(), start);
        assert!(!app.is_dirty());
    }

    #[test]
    fn gui_edits_replay_from_the_session_log_to_an_identical_design() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        // Move a part, change the size of the rod, add a screen, switch a part off.
        move_z(&mut app, 1, 3.5);
        move_z(&mut app, 1, 1.25);
        app.end_gesture();
        let rod = app.design().occurrences[2].clone();
        let part = rod.part.as_str().to_string();
        let mut edits = Edits::default();
        edits.set(format!("/parts/@{part}/rod/width_mm"), &7.5);
        edits.set(format!("/parts/@{part}/rod/length_mm"), &61.0);
        assert!(app.apply(edits), "{:?}", app.error);
        app.end_gesture();
        app.add_generated(PartKind::Detector);
        assert_eq!(app.design().occurrences.len(), 5);
        app.component = 1;
        let mut edits = Edits::default();
        edits.set(
            format!(
                "/occurrences/@{}/enabled",
                app.design().occurrences[1].id.as_str()
            ),
            &false,
        );
        assert!(app.apply(edits));
        app.end_gesture();
        // An undone edit is not in the log.
        move_z(&mut app, 0, 9.0);
        app.end_gesture();
        app.undo();
        // An undo-redo round trip keeps its entries.
        app.undo();
        app.redo();
        let status = app.session.status();
        assert_eq!(status.undo_depth, 4);
        let log = app.session.log();
        assert_eq!(log.entries.len(), 6, "{:?}", log.entries);
        assert!(
            log.entries
                .iter()
                .all(|e| e.command == "doc.set" || e.command == "component.add")
        );
        assert!(
            log.entries
                .iter()
                .all(|e| e.params.get("expected_revision").is_none())
        );

        let fresh = replayed(&app.session);
        assert_eq!(design_json(&fresh), design_json(&app.session));
        assert_eq!(fresh.log().entries, log.entries);
        // Everything before is undone one step at a time to the starting document.
        for _ in 0..4 {
            app.undo();
        }
        assert!(!app.can_undo());
        assert!(!app.is_dirty());
        assert_eq!(
            design_json(&replayed(&app.session)),
            design_json(&app.session)
        );
    }

    #[test]
    fn adding_a_screen_places_it_behind_the_system_in_one_undo_step() {
        let mut app = App::demo();
        let end = app.next_z();
        app.add_generated(PartKind::Detector);
        let added = app.design().occurrences.last().unwrap().clone();
        assert_eq!(added.pose.translation_mm.0[2], end);
        assert_eq!(app.component, app.design().occurrences.len() - 1);
        assert_eq!(app.session.status().undo_depth, 1);
        app.undo();
        assert_eq!(app.design().occurrences.len(), 4);
    }

    #[test]
    fn opening_a_file_starts_a_new_history_and_adding_a_part_is_an_undoable_step() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        move_z(&mut app, 2, 1.0);
        app.end_gesture();
        let lens = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/sequential_lens.json"
        ));
        app.load(lens.clone(), &ctx);
        wait(&mut app, &ctx);
        assert!(!app.can_undo(), "opening clears the history");
        assert!(!app.is_dirty());
        assert_eq!(app.session.path(), Some(lens.as_path()));
        assert_eq!(app.session.log().base.unwrap().command, "doc.open");
        assert!(app.result.is_some() || app.job.is_some());
        wait(&mut app, &ctx);

        // A part read from a file is added by a copy of the session on a worker.
        app.add_file(lens, PartKind::Optic, &ctx);
        wait(&mut app, &ctx);
        assert_eq!(app.design().occurrences.len(), 2);
        assert!(app.is_dirty());
        assert_eq!(app.session.status().undo_depth, 1);
        let log = app.session.log();
        assert_eq!(log.entries[0].command, "component.add");
        assert_eq!(log.entries.len(), 2, "the position joins the addition");
        assert_eq!(
            design_json(&replayed(&app.session)),
            design_json(&app.session)
        );
        app.undo();
        assert_eq!(app.design().occurrences.len(), 1);
    }

    fn drag(app: &mut App, ctx: &egui::Context, from: Pos2, by: Vec2) {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let mut at = from;
        for _ in 0..5 {
            at += by / 5.0;
            frame(app, ctx, vec![egui::Event::PointerMoved(at)]);
        }
        frame(
            app,
            ctx,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(app, ctx, vec![]);
    }

    #[test]
    fn dragging_and_typing_in_the_inspector_fields_edit_through_commands() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let rod = app.design().occurrences[2].clone();
        let z = rod.pose.translation_mm.0[2];
        // Drag the Z field of the rod's position.
        let out = frame(&mut app, &ctx, vec![]);
        let field = text_rect(&out, &format!("{z:?}"))
            .expect("Z field")
            .center();
        drag(&mut app, &ctx, field, Vec2::new(40.0, 0.0));
        let dragged = app.design().occurrences[2].pose.translation_mm.0[2];
        assert!(dragged > z, "{dragged} {z}");
        assert_eq!(app.session.status().undo_depth, 1, "one drag, one step");
        let log = app.session.log();
        assert_eq!(log.entries.len(), 1);
        assert_eq!(
            log.entries[0].params["path"],
            format!("/occurrences/@{}/pose/translation_mm/2", rod.id.as_str())
        );

        // Click the length of the rod and type a value.
        let out = frame(&mut app, &ctx, vec![]);
        let length = text_rect(&out, "30.0").expect("length field").center();
        click(&mut app, &ctx, length);
        frame(&mut app, &ctx, vec![egui::Event::Text("45".into())]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let part = app.design().part(&rod.part).unwrap();
        assert_eq!(part.rod.as_ref().unwrap().length_mm, 45.0);
        assert_eq!(app.session.status().undo_depth, 2);
        let fresh = replayed(&app.session);
        assert_eq!(design_json(&fresh), design_json(&app.session));
        app.undo();
        assert_eq!(
            app.design()
                .part(&rod.part)
                .unwrap()
                .rod
                .as_ref()
                .unwrap()
                .length_mm,
            30.0
        );
    }

    #[test]
    fn a_single_system_is_edited_through_the_paths_of_its_part() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        let scene: Scene =
            serde_json::from_str(include_str!("../../../../examples/sequential_lens.json"))
                .unwrap();
        assert!(app.start_document(Some(Design::from_scene(scene))));
        assert!(!app.design().is_assembly());
        let (index, radius) = app
            .design()
            .lone_system()
            .unwrap()
            .scene
            .surfaces
            .iter()
            .enumerate()
            .find_map(|(i, s)| match s.shape {
                Shape::Sphere { radius_mm } => Some((i, radius_mm)),
                _ => None,
            })
            .expect("a spherical surface");
        app.selected = index;
        app.source_selected = false;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let out = frame(&mut app, &ctx, vec![]);
        let field = text_rect(&out, &format!("{radius:?}"))
            .expect("radius field")
            .center();
        drag(&mut app, &ctx, field, Vec2::new(30.0, 0.0));
        let Shape::Sphere { radius_mm } =
            app.design().lone_system().unwrap().scene.surfaces[index].shape
        else {
            panic!("still a sphere");
        };
        assert_ne!(radius_mm, radius);
        let log = app.session.log();
        assert_eq!(log.entries.len(), 1);
        assert_eq!(
            log.entries[0].params["path"],
            format!("/parts/@system-1/scene/surfaces/{index}/shape/radius_mm")
        );
        assert_eq!(
            design_json(&replayed(&app.session)),
            design_json(&app.session)
        );
        app.undo();
        assert!(!app.can_undo());
    }
}
