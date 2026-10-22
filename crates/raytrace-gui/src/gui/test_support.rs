use super::*;

pub(super) fn frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(1050.0, 740.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            app.toolbar(ctx);
            app.sidebar(ctx);
            app.content(ctx);
            app.details_window(ctx);
        },
    )
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
