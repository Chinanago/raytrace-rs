use super::*;
use raytrace_io::record::{RUN_FORMAT, RunContext};

pub(super) fn run_ui(
    ctx: &egui::Context,
    input: egui::RawInput,
    ui: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, ui);
    // These tests inspect shapes without uploading textures to a renderer.
    output.textures_delta.clear();
    output
}

pub(super) fn frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let output = run_ui(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(1050.0, 740.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            app.toolbar(ui);
            app.sidebar(ui);
            let workspace = ui.available_rect_before_wrap();
            app.content(ui);
            app.details_window(ui.ctx(), workspace);
        },
    );
    app.finish_frame(ctx);
    output
}

pub(super) fn text_rect(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
    output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.job.text == text => {
            let rect = egui::Rect::from_min_size(t.pos, t.galley.size()).intersect(s.clip_rect);
            rect.is_positive().then_some(rect)
        }
        _ => None,
    })
}

pub(super) fn click(app: &mut App, ctx: &egui::Context, p: Pos2) -> egui::FullOutput {
    frame(
        app,
        ctx,
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
    frame(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
}

/// Replace the document with an edited copy as a new document: no history, nothing unsaved.
/// The tests use it to set up the document they then look at.
pub(super) fn set_document(app: &mut App, edit: impl FnOnce(&mut Design)) {
    let mut design = app.design().clone();
    edit(&mut design);
    assert!(app.start_document(Some(design)), "{:?}", app.error);
}

pub(super) fn calculate_demo(app: &mut App) {
    use raytrace_core::trace::{TraceOptions, trace};
    let mut scene = app.design().trace_scene().unwrap();
    for source in &mut scene.sources {
        source.count = 64;
    }
    let prepared = scene.clone().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    app.result = Some(ResultView::of_record(
        app.session.design().unwrap().clone(),
        app.preview_limit,
        RunRecord {
            format: RUN_FORMAT.into(),
            calculation: RunContext::capture(
                app.design(),
                &scene,
                None,
                1,
                TraceOptions::default(),
            ),
            trace_seconds: 0.0,
            report: trace(&prepared, &rays, TraceOptions::default()).unwrap(),
        },
    ));
}

pub(super) fn visible_text(output: &egui::FullOutput) -> Vec<&str> {
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if egui::Rect::from_min_size(t.pos, t.galley.size()).intersects(s.clip_rect) =>
            {
                Some(t.galley.job.text.as_str())
            }
            _ => None,
        })
        .collect()
}
