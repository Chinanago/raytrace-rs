//! Values derived from @digital-go-jp/design-tokens 2.1.0 (MIT License,
//! © Digital Agency, Government of Japan) and the DADS guidelines.
//! This is a local style guide for raytrace-rs, not an official Digital Agency product.

use eframe::egui::NumExt as _;
use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Stroke, Vec2};

// Keep the small source palette here, separate from roles used by the GUI.
mod primitive {
    use super::Color32;
    pub const WHITE: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BLACK: Color32 = Color32::from_rgb(0, 0, 0);
    pub const GRAY_50: Color32 = Color32::from_rgb(242, 242, 242);
    pub const GRAY_100: Color32 = Color32::from_rgb(230, 230, 230);
    pub const GRAY_420: Color32 = Color32::from_rgb(148, 148, 148);
    pub const GRAY_536: Color32 = Color32::from_rgb(118, 118, 118);
    pub const GRAY_600: Color32 = Color32::from_rgb(102, 102, 102);
    pub const GRAY_800: Color32 = Color32::from_rgb(51, 51, 51);
    pub const BLUE_50: Color32 = Color32::from_rgb(232, 241, 254);
    pub const BLUE_100: Color32 = Color32::from_rgb(217, 230, 255);
    pub const BLUE_900: Color32 = Color32::from_rgb(0, 23, 193);
    pub const BLUE_1000: Color32 = Color32::from_rgb(0, 17, 143);
    pub const BLUE_1200: Color32 = Color32::from_rgb(0, 0, 96);
    pub const GREEN_600: Color32 = Color32::from_rgb(37, 157, 99);
    pub const GREEN_800: Color32 = Color32::from_rgb(25, 122, 75);
    pub const RED_50: Color32 = Color32::from_rgb(253, 238, 238);
    pub const RED_800: Color32 = Color32::from_rgb(236, 0, 0);
    pub const RED_900: Color32 = Color32::from_rgb(206, 0, 0);
    pub const ORANGE_50: Color32 = Color32::from_rgb(255, 238, 226);
    pub const ORANGE_600: Color32 = Color32::from_rgb(251, 91, 1);
    pub const ORANGE_800: Color32 = Color32::from_rgb(199, 71, 0);
    pub const YELLOW_50: Color32 = Color32::from_rgb(251, 245, 224);
    pub const YELLOW_300: Color32 = Color32::from_rgb(255, 212, 61);
}

pub const TEXT: Color32 = primitive::GRAY_800;
pub const TEXT_MUTED: Color32 = primitive::GRAY_600;
// Disabled text is exempt from WCAG text contrast. egui additionally fades disabled widgets.
#[allow(dead_code)] // semantic disabled role; egui controls its own fade
pub const TEXT_DISABLED: Color32 = primitive::GRAY_420;
pub const TEXT_ON_PRIMARY: Color32 = primitive::WHITE;
pub const BG_CANVAS: Color32 = primitive::WHITE;
pub const BG_PANEL: Color32 = primitive::GRAY_50;
pub const BG_RAISED: Color32 = primitive::WHITE;
pub const PRIMARY: Color32 = primitive::BLUE_900;
pub const PRIMARY_HOVER: Color32 = primitive::BLUE_1000;
pub const PRIMARY_ACTIVE: Color32 = primitive::BLUE_1200;
pub const PRIMARY_SUBTLE: Color32 = primitive::BLUE_50;
pub const PRIMARY_SUBTLE_HOVER: Color32 = primitive::BLUE_100;
pub const BORDER_CONTROL: Color32 = primitive::GRAY_536;
pub const BORDER_DIVIDER: Color32 = primitive::GRAY_420;
pub const GRID: Color32 = primitive::GRAY_100;
pub const LINK: Color32 = primitive::BLUE_1000;
pub const WARNING: Color32 = primitive::ORANGE_800;
pub const ERROR: Color32 = primitive::RED_800;
pub const SUCCESS: Color32 = primitive::GREEN_800;
pub const FOCUS_RING: Color32 = primitive::YELLOW_300;
pub const FOCUS_INNER: Color32 = primitive::BLACK;

// Reserved semantic roles, ready for future status icons/banners. DADS orange-600
// is only 2.84:1 on gray-50: use the panel-safe WARNING_PANEL_ICON there.
#[allow(dead_code)]
pub mod status {
    use super::*;
    pub const SUCCESS_ICON: Color32 = primitive::GREEN_600;
    pub const ERROR_ICON: Color32 = primitive::RED_900;
    pub const WARNING_ICON: Color32 = primitive::ORANGE_600;
    pub const WARNING_PANEL_ICON: Color32 = primitive::ORANGE_800;
    pub const ERROR_SUBTLE: Color32 = primitive::RED_50;
    pub const WARNING_SUBTLE: Color32 = primitive::ORANGE_50;
    pub const CAUTION_SUBTLE: Color32 = primitive::YELLOW_50;
}

pub const SIZE_CAPTION: f32 = 14.0;
pub const SIZE_BODY: f32 = 14.0;
pub const SIZE_BODY_LARGE: f32 = 16.0;
pub const SIZE_HEADING_S: f32 = 16.0;
pub const SIZE_HEADING_M: f32 = 18.0;
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;
pub const CONTROL_HEIGHT: f32 = SPACE_8;
pub const TOOLBAR_HEIGHT: f32 = CONTROL_HEIGHT + SPACE_2 * 2.0;
// gray-420 is below 3:1 on gray-50; use the DADS gray-536 fallback.
pub const TAB_BORDER: Color32 = primitive::GRAY_536;
pub const TAB_HOVER_BAR: Color32 = primitive::GRAY_420;
pub const TAB_HOVER_FILL: Color32 = primitive::GRAY_50;
pub const TAB_BAR_HEIGHT: f32 = 6.0;
pub const TAB_HEIGHT: f32 = 40.0;
pub const TAB_PADDING_X: f32 = SPACE_4;
pub const TAB_PADDING_Y: f32 = 7.0;
pub const TAB_LINE_WIDTH: f32 = 1.0;
pub const TAB_UNDERLINE_OFFSET: f32 = 3.0;
pub const PRIMARY_BUTTON_WIDTH: f32 = 118.0;
pub const STATUS_HEIGHT: f32 = 38.0;
pub const STATUS_PADDING_Y: f32 = 5.0;
pub const WINDOW_CHROME_HEIGHT: f32 = CONTROL_HEIGHT + SPACE_4 * 4.0 + 2.0;

// Preserve the 0.31 checkbox geometry, including its leading icon and radius.
pub fn checkbox(ui: &mut egui::Ui, checked: &mut bool, label: &str) -> egui::Response {
    let spacing = ui.spacing();
    let extra = Vec2::new(spacing.icon_width + spacing.icon_spacing, 0.0);
    let galley = (!label.is_empty()).then(|| {
        egui::WidgetText::from(label).into_galley(
            ui,
            None,
            ui.available_width() - extra.x,
            egui::TextStyle::Button,
        )
    });
    let size = galley
        .as_ref()
        .map_or(Vec2::new(spacing.icon_width, 0.0), |g| {
            (extra + g.size()).at_least(spacing.interact_size)
        });
    let size = size.at_least(Vec2::splat(spacing.interact_size.y));
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *checked, label)
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let (small, big) = ui.spacing().icon_rectangles(rect);
        ui.painter().rect(
            big.expand(visuals.expansion),
            visuals.corner_radius,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        if *checked {
            ui.painter().add(egui::Shape::line(
                vec![
                    egui::pos2(small.left(), small.center().y),
                    egui::pos2(small.center().x, small.bottom()),
                    egui::pos2(small.right(), small.top()),
                ],
                visuals.fg_stroke,
            ));
        }
        if let Some(galley) = galley {
            let pos = egui::pos2(
                rect.left() + extra.x,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(pos, galley, visuals.text_color());
        }
    }
    response
}

pub fn name_edit(ui: &mut egui::Ui, name: &mut String, width: f32) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        ui.add(
            egui::TextEdit::singleline(name)
                .margin(egui::Margin::symmetric(4, 2))
                // TextEdit now takes an outer width instead of a text-area width.
                .desired_width(width + SPACE_2),
        )
    })
    .inner
}

pub fn toolbar_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_PANEL)
        .inner_margin(egui::Margin::symmetric(SPACE_4 as i8, 0))
}

pub fn sidebar_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_PANEL)
        .inner_margin(SPACE_4)
        // Panels now reserve their separator outside the frame.
        .outer_margin(egui::Margin {
            right: -1,
            ..Default::default()
        })
}

pub fn history_button(ui: &mut egui::Ui, enabled: bool, glyph: &str) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let response = ui.add(egui::Button::new("").min_size(Vec2::splat(CONTROL_HEIGHT)));
        // Hack's arrow ink occupies only a small fraction of its line box. Paint it
        // independently so the visible arrow is body-sized without raising the row.
        ui.painter().text(
            response.rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(SPACE_8),
            ui.style().interact(&response).text_color(),
        );
        response
    })
    .inner
}

pub const RADIUS_CONTROL: u8 = 6;
pub const RADIUS_PANEL: u8 = 8;

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}
pub fn heading(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(bold_font(SIZE_HEADING_S))
        .color(TEXT)
}
pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SIZE_CAPTION).color(TEXT_MUTED)
}

pub fn window_title(ctx: &egui::Context, text: &str) -> egui::Atoms<'static> {
    let font = bold_font(SIZE_HEADING_M);
    let height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
    let mut atoms =
        egui::Atoms::new(RichText::new(text.to_owned()).line_height(Some(CONTROL_HEIGHT)));
    // Balance the close button so the title stays centered on the full window.
    atoms.push_left(egui::Atom::custom(
        egui::IdSalt::new("title_balance"),
        Vec2::new(height + SPACE_2, 0.0),
    ));
    atoms
}

/// Data colors are independent of interface roles. All opaque line/marker colors
/// have >=3:1 contrast on white. The heatmap and the translucent ray bundle are
/// density displays, not UI ink: their overlap carries the information.
pub mod viz {
    use super::Color32;
    pub const MARKER: Color32 = SERIES[0];
    pub const MARKER_MUTED: Color32 = super::TEXT_MUTED;
    pub const HALF_COLOR: Color32 = Color32::from_rgb(188, 48, 92);
    /// Ray segment colour; opacity follows the relative branch weight `t` in [0, 1].
    pub fn ray(t: f64) -> Color32 {
        // 0.31 premultiplied in linear space before encoding back to sRGB.
        let alpha = (25.0 + 155.0 * t.sqrt()) as u8;
        let premultiply = |value| {
            eframe::egui::ecolor::gamma_u8_from_linear_f32(
                eframe::egui::ecolor::linear_f32_from_gamma_u8(value) * f32::from(alpha) / 255.0,
            )
        };
        Color32::from_rgba_premultiplied(premultiply(224), premultiply(135), premultiply(48), alpha)
    }
    pub const GEOMETRY: Color32 = Color32::from_rgb(104, 139, 172);
    pub const ROI: Color32 = Color32::from_rgb(46, 139, 87);
    // Orange darkened from (203,137,38), also used as legend text (>=4.5:1).
    // Red (201,78,75), green (49,143,101), cyan (45,146,166) also darkened
    // for >=4.5:1 legend text; blue/purple retain their original colors.
    pub const SERIES: [Color32; 6] = [
        Color32::from_rgb(40, 90, 180),
        Color32::from_rgb(200, 77, 74),
        Color32::from_rgb(36, 121, 83),
        Color32::from_rgb(149, 89, 183),
        Color32::from_rgb(159, 101, 20),
        Color32::from_rgb(32, 119, 137),
    ];
    pub fn heatmap(fraction: f64, square_root: bool) -> Color32 {
        let [r, g, b] = raytrace_io::export::rgb(fraction, square_root);
        Color32::from_rgb(r, g, b)
    }
}

pub fn install(ctx: &egui::Context) {
    // Pin the light theme before the first native input event.
    ctx.set_theme(egui::Theme::Light);
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "Noto Sans JP",
            include_bytes!("../../assets/fonts/NotoSansJP-Regular.otf").as_slice(),
        ),
        (
            "Noto Sans JP Bold",
            include_bytes!("../../assets/fonts/NotoSansJP-Bold.otf").as_slice(),
        ),
        (
            "Noto Sans Mono",
            include_bytes!("../../assets/fonts/NotoSansMono-Regular.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    let mut defaults = fonts.families[&FontFamily::Proportional].clone();
    // Undo/redo arrows are in egui's bundled Hack, but not its proportional list.
    defaults.push("Hack".into());
    let mut bold = vec!["Noto Sans JP Bold".into(), "Noto Sans JP".into()];
    bold.extend(defaults);
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "Noto Sans JP".into());
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .push("Hack".into());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .splice(0..0, ["Noto Sans Mono".into(), "Noto Sans JP".into()]);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    for text_style in [
        egui::TextStyle::Small,
        egui::TextStyle::Body,
        egui::TextStyle::Button,
    ] {
        style
            .text_styles
            .insert(text_style, FontId::proportional(SIZE_BODY));
    }
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(SIZE_BODY));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, bold_font(SIZE_HEADING_M));
    style.spacing.item_spacing = Vec2::splat(SPACE_2);
    style.spacing.button_padding = Vec2::new(SPACE_3, SPACE_3 / 2.0);
    style.spacing.interact_size = Vec2::new(SPACE_6, SPACE_8);
    style.spacing.window_margin = egui::Margin::same(SPACE_4 as i8);
    style.spacing.scroll.fade.strength = 0.0;
    let mut visuals = egui::Visuals::light();
    visuals.text_options.font_hinting = false;
    let widget = |fill, border, text| egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0_f32, border),
        fg_stroke: Stroke::new(1.0_f32, text),
        corner_radius: RADIUS_CONTROL.into(),
        expansion: 0.0,
    };
    visuals.widgets.noninteractive = widget(BG_PANEL, BORDER_DIVIDER, TEXT);
    visuals.widgets.inactive = widget(BG_CANVAS, BORDER_CONTROL, TEXT);
    visuals.widgets.hovered = widget(PRIMARY_SUBTLE, PRIMARY, PRIMARY);
    visuals.widgets.active = widget(PRIMARY_SUBTLE_HOVER, PRIMARY, PRIMARY);
    visuals.widgets.open = widget(PRIMARY_SUBTLE, PRIMARY, PRIMARY);
    visuals.selection = egui::style::Selection {
        bg_fill: PRIMARY_SUBTLE,
        stroke: Stroke::new(1.0_f32, PRIMARY),
    };
    visuals.hyperlink_color = LINK;
    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = BG_RAISED;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER_DIVIDER);
    visuals.faint_bg_color = BG_CANVAS;
    visuals.extreme_bg_color = BG_CANVAS;
    visuals.code_bg_color = BG_CANVAS;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    visuals.window_corner_radius = RADIUS_PANEL.into();
    visuals.menu_corner_radius = RADIUS_PANEL.into();
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 12,
        spread: 2,
        color: Color32::from_black_alpha(26),
    };
    visuals.popup_shadow = visuals.window_shadow;
    style.visuals = visuals;
    ctx.set_style_of(egui::Theme::Light, style);

    // Paint the two DADS focus layers after widgets.
    // read_response only knows the interaction rect:
    // partly scrolled/clipped widgets get a ring around their visible rect. Pointer
    // focus also receives the ring; native OS dialogs are outside egui's control.
    ctx.on_end_pass(
        "DADS focus ring",
        std::sync::Arc::new(|ctx| {
            let focused = ctx.memory(|m| m.focused());
            if let Some(response) = focused.and_then(|id| ctx.read_response(id))
                && response.enabled()
                && response.interact_rect.is_positive()
            {
                let painter = ctx.layer_painter(response.layer_id);
                let rect = response.interact_rect;
                painter.rect_stroke(
                    rect.expand(SPACE_1 - 1.0),
                    RADIUS_PANEL,
                    Stroke::new(3.0_f32, FOCUS_RING),
                    egui::StrokeKind::Outside,
                );
                painter.rect_stroke(
                    rect.expand(1.0),
                    RADIUS_CONTROL,
                    Stroke::new(2.0_f32, FOCUS_INNER),
                    egui::StrokeKind::Outside,
                );
            }
        }),
    );
}

pub fn primary_scope(
    ui: &mut egui::Ui,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.bg_fill = PRIMARY;
        visuals.widgets.inactive.weak_bg_fill = PRIMARY;
        visuals.widgets.hovered.bg_fill = PRIMARY_HOVER;
        visuals.widgets.hovered.weak_bg_fill = PRIMARY_HOVER;
        visuals.widgets.active.bg_fill = PRIMARY_ACTIVE;
        visuals.widgets.active.weak_bg_fill = PRIMARY_ACTIVE;
        add(ui)
    })
    .inner
}

pub fn component_button(ui: &mut egui::Ui, label: String, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let response = ui
        .with_layout(
            egui::Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Min),
            |ui| {
                ui.add(
                    egui::Button::new(RichText::new(label).color(if selected {
                        PRIMARY
                    } else {
                        TEXT
                    }))
                    .min_size(Vec2::new(width, SPACE_8))
                    .selected(selected)
                    .corner_radius(egui::CornerRadius::ZERO)
                    .stroke(if selected {
                        ui.visuals().selection.stroke
                    } else {
                        Stroke::NONE
                    })
                    .frame(selected),
                )
            },
        )
        .inner;
    if selected {
        ui.painter().vline(
            response.rect.left() + 1.0,
            response.rect.y_range(),
            Stroke::new(2.0_f32, PRIMARY),
        );
    }
    response
}

/// Paint the panel's top border, leaving the selected folder ear open.
pub fn tab_baseline(ui: &egui::Ui, row: egui::Rect, selected: egui::Rect) {
    let y = row.bottom() - TAB_LINE_WIDTH / 2.0;
    let stroke = Stroke::new(TAB_LINE_WIDTH, TAB_BORDER);
    ui.painter().hline(row.left()..=selected.left(), y, stroke);
    ui.painter()
        .hline(selected.right()..=row.right(), y, stroke);
}

pub fn tab<T: PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    target: T,
    label: &str,
    selected_fill: Color32,
) -> egui::Response {
    let selected = *value == target;
    let font = if selected {
        bold_font(SIZE_BODY)
    } else {
        FontId::proportional(SIZE_BODY)
    };
    let galley = ui.painter().layout_no_wrap(label.into(), font, TEXT);
    let mut response = ui.allocate_response(
        Vec2::new(galley.size().x + TAB_PADDING_X * 2.0, TAB_HEIGHT),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), selected, label)
    });
    let rect = response.rect;
    let hover = response.hovered() && !selected;
    ui.painter().rect_filled(
        rect,
        0,
        if selected {
            selected_fill
        } else if hover {
            TAB_HOVER_FILL
        } else {
            BG_CANVAS
        },
    );
    let stroke = Stroke::new(TAB_LINE_WIDTH, TAB_BORDER);
    let border = rect.shrink(TAB_LINE_WIDTH / 2.0);
    ui.painter().vline(border.left(), border.y_range(), stroke);
    ui.painter().vline(border.right(), border.y_range(), stroke);
    ui.painter().hline(border.x_range(), border.top(), stroke);
    if !selected {
        ui.painter()
            .hline(border.x_range(), border.bottom(), stroke);
    }
    if selected || hover {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.min + Vec2::splat(TAB_LINE_WIDTH),
                Vec2::new(rect.width() - TAB_LINE_WIDTH * 2.0, TAB_BAR_HEIGHT),
            ),
            0,
            if selected { PRIMARY } else { TAB_HOVER_BAR },
        );
    }
    let text_pos = rect.min + Vec2::new(TAB_PADDING_X, TAB_BAR_HEIGHT + TAB_PADDING_Y);
    if hover {
        ui.painter().hline(
            text_pos.x..=text_pos.x + galley.size().x,
            text_pos.y + galley.size().y + TAB_UNDERLINE_OFFSET,
            Stroke::new(TAB_LINE_WIDTH, TEXT),
        );
    }
    ui.painter().galley(text_pos, galley, TEXT);
    if response.clicked() && !selected {
        *value = target;
        response.mark_changed();
        // Repaint the entire row in this frame so a click cannot leave two bold
        // labels (the previously selected ear may already have been painted).
        ui.ctx().request_discard("tab selection changed");
    }
    response
}

/// DADS-style inline notification; status colour is reserved for the icon.
pub fn notification(ui: &mut egui::Ui, error: bool, message: &str) -> bool {
    let (background, icon) = if error {
        (status::ERROR_SUBTLE, status::ERROR_ICON)
    } else {
        (status::WARNING_SUBTLE, status::WARNING_PANEL_ICON)
    };
    egui::Frame::new()
        .fill(background)
        .corner_radius(RADIUS_PANEL)
        .inner_margin(SPACE_3)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if error { "✕" } else { "!" })
                        .color(icon)
                        .font(bold_font(SIZE_BODY_LARGE)),
                );
                ui.label(RichText::new(message).color(TEXT));
                ui.button("確認").clicked()
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    // WCAG 2.x sRGB relative luminance (IEC 61966-2-1 transfer function).
    fn luminance(color: Color32) -> f64 {
        let linear = |v: u8| {
            let c = f64::from(v) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn check(foreground: Color32, background: Color32, minimum: f64) {
        let ratio = contrast(foreground, background);
        assert!(
            ratio >= minimum,
            "{foreground:?} on {background:?}: {ratio:.3}:1 < {minimum}:1"
        );
    }

    #[test]
    fn wcag_reference_values() {
        assert!((contrast(primitive::BLACK, primitive::WHITE) - 21.0).abs() < 1e-12);
        assert_eq!(contrast(TEXT, TEXT), 1.0);
        assert!((luminance(primitive::GRAY_536) - 0.181164).abs() < 1e-6);
    }

    #[test]
    fn text_role_contrast_on_actual_surfaces() {
        for bg in [
            BG_CANVAS,
            BG_PANEL,
            BG_RAISED,
            PRIMARY_SUBTLE,
            PRIMARY_SUBTLE_HOVER,
            status::ERROR_SUBTLE,
            status::WARNING_SUBTLE,
            status::CAUTION_SUBTLE,
        ] {
            for fg in [TEXT, TEXT_MUTED, LINK, PRIMARY] {
                check(fg, bg, 4.5);
            }
        }
        // Warnings are on the white workspace / report windows only. ERROR is
        // egui's error text on white inputs/windows. Neither is panel/banner ink:
        // exact DADS red-800/orange-800 fail 4.5:1 on gray-50 and subtle fills.
        for bg in [BG_CANVAS, BG_RAISED] {
            for fg in [SUCCESS, ERROR, WARNING] {
                check(fg, bg, 4.5);
            }
        }
        for bg in [PRIMARY, PRIMARY_HOVER, PRIMARY_ACTIVE] {
            check(TEXT_ON_PRIMARY, bg, 4.5);
        }
        for color in viz::SERIES {
            check(color, BG_CANVAS, 4.5);
        } // legend text
        check(viz::HALF_COLOR, BG_CANVAS, 4.5); // FWHM caption
    }

    #[test]
    fn boundaries_icons_and_data_contrast() {
        for bg in [BG_PANEL, BG_CANVAS] {
            for fg in [
                BORDER_CONTROL,
                status::SUCCESS_ICON,
                status::ERROR_ICON,
                status::WARNING_PANEL_ICON,
                PRIMARY,
            ] {
                check(fg, bg, 3.0);
            }
        }
        // Exact orange-600 is permitted on white only (2.84:1 on gray-50).
        check(status::WARNING_ICON, BG_CANVAS, 3.0);
        for color in [
            viz::HALF_COLOR,
            viz::GEOMETRY,
            viz::ROI,
            viz::MARKER,
            viz::MARKER_MUTED,
        ]
        .into_iter()
        .chain(viz::SERIES)
        {
            check(color, BG_CANVAS, 3.0);
        }
        check(status::ERROR_ICON, status::ERROR_SUBTLE, 3.0);
        check(status::WARNING_PANEL_ICON, status::WARNING_SUBTLE, 3.0);
        check(FOCUS_RING, FOCUS_INNER, 3.0);
    }

    #[test]
    fn tab_contrast_on_toolbar_and_window_surfaces() {
        assert!(contrast(primitive::GRAY_420, BG_PANEL) < 3.0);
        for bg in [BG_CANVAS, TAB_HOVER_FILL] {
            check(TEXT, bg, 4.5);
            check(PRIMARY, bg, 3.0);
            check(TAB_BORDER, bg, 3.0);
        }
    }

    #[test]
    fn folder_tabs_have_one_bold_label_top_bar_and_open_baseline() {
        let ctx = egui::Context::default();
        install(&ctx);
        for active in 0..3 {
            let mut value = active;
            let mut rects = Vec::new();
            let output = crate::gui::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = -TAB_LINE_WIDTH;
                        for (target, label) in ["First", "Second", "Third"].into_iter().enumerate()
                        {
                            rects.push(tab(ui, &mut value, target, label, BG_RAISED).rect);
                        }
                    });
                    let row = egui::Rect::from_min_max(
                        row.response.rect.min,
                        egui::pos2(ui.max_rect().right(), row.response.rect.bottom()),
                    );
                    tab_baseline(ui, row, rects[active]);
                });
            });
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(text)
                        if ["First", "Second", "Third"]
                            .contains(&text.galley.job.text.as_str()) =>
                    {
                        Some(text)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(labels.len(), 3);
            let bold: Vec<_> = labels
                .iter()
                .enumerate()
                .filter(|(_, text)| {
                    text.galley.job.sections[0].format.font_id == bold_font(SIZE_BODY)
                })
                .map(|(i, _)| i)
                .collect();
            assert_eq!(bold, [active]);
            assert_eq!(rects[0].right() - rects[1].left(), TAB_LINE_WIDTH);
            assert!(rects.iter().all(|rect| rect.height() == TAB_HEIGHT));
            let selected = rects[active];
            let label_bottom = labels[active].pos.y + labels[active].galley.size().y;
            let mut bars = 0;
            for shape in &output.shapes {
                match &shape.shape {
                    egui::Shape::LineSegment { points, .. } => {
                        // Includes both a former blue underline and an accidental
                        // baseline/bottom border across the selected opening.
                        let horizontal = points[0].y == points[1].y;
                        let overlaps_label = points[0].x.min(points[1].x)
                            < selected.right() - TAB_PADDING_X
                            && points[0].x.max(points[1].x) > selected.left() + TAB_PADDING_X;
                        assert!(
                            !(horizontal
                                && overlaps_label
                                && points[0].y >= label_bottom
                                && points[0].y <= selected.bottom())
                        );
                    }
                    egui::Shape::Rect(rect) if rect.fill == PRIMARY => {
                        bars += 1;
                        assert_eq!(rect.rect.height(), TAB_BAR_HEIGHT);
                        assert!(rect.rect.bottom() < labels[active].pos.y);
                        assert_eq!(rect.corner_radius, egui::CornerRadius::ZERO);
                    }
                    _ => {}
                }
            }
            assert_eq!(bars, 1);
        }
    }

    #[test]
    fn only_unselected_hover_has_an_underline_and_gray_bar() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut rect = egui::Rect::NOTHING;
        for hovered in [false, true] {
            let input = egui::RawInput {
                events: if hovered {
                    vec![egui::Event::PointerMoved(rect.center())]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            let output = crate::gui::test_support::run_ui(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    rect = tab(ui, &mut false, true, "Hover", BG_RAISED).rect;
                });
            });
            if hovered {
                let text = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(text) => Some(text),
                        _ => None,
                    })
                    .unwrap();
                assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::LineSegment { points, stroke } if stroke.color == TEXT && points[0].y == text.pos.y + text.galley.size().y + TAB_UNDERLINE_OFFSET)));
                assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == TAB_HOVER_BAR && r.rect.height() == TAB_BAR_HEIGHT)));
            }
        }
    }

    #[test]
    fn source_uses_color_and_typography_tokens() {
        fn walk(dir: &std::path::Path) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().unwrap() != "theme.rs"
                {
                    let source = std::fs::read_to_string(&path).unwrap();
                    for pattern in ["Color32::from_rgb", "Color32::from_gray"] {
                        assert!(!source.contains(pattern), "{}: {pattern}", path.display());
                    }
                    // Disallow ALL literal font sizes, even >=14, to preserve tokens.
                    for pattern in ["FontId::proportional(", "FontId::monospace(", ".size("] {
                        for tail in source.split(pattern).skip(1) {
                            assert!(
                                !tail.trim_start().starts_with(|c: char| c.is_ascii_digit()),
                                "{}: literal after {pattern}",
                                path.display()
                            );
                        }
                    }
                }
            }
        }
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    }

    #[test]
    fn installed_fonts_sizes_and_keyboard_focus_ring() {
        let ctx = egui::Context::default();
        install(&ctx);
        let style = ctx.style_of(egui::Theme::Light);
        assert!(style.text_styles.values().all(|f| f.size >= SIZE_CAPTION));
        assert_eq!(
            style.text_styles[&egui::TextStyle::Heading],
            bold_font(SIZE_HEADING_M)
        );
        let output = crate::gui::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                for font in [
                    FontId::proportional(SIZE_BODY),
                    FontId::monospace(SIZE_BODY),
                    bold_font(SIZE_BODY),
                ] {
                    assert!(ui.fonts_mut(|f| f.has_glyphs(&font, "日本語 ▶ ↶ ↷ ✓")));
                }
                let response = ui.button("Focus");
                response.request_focus();
                ui.label(heading("日本語 ▶ ↶ ↷ ✓"));
            });
        });
        let strokes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(rect) => Some(rect.stroke.color),
                _ => None,
            })
            .collect();
        assert!(strokes.contains(&FOCUS_RING));
        assert!(strokes.contains(&FOCUS_INNER));
    }
}
