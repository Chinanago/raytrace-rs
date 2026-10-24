//! Values derived from @digital-go-jp/design-tokens 2.1.0 (MIT License,
//! © Digital Agency, Government of Japan) and the DADS guidelines.
//! This is a local style guide for raytrace-rs, not an official Digital Agency product.

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
pub const SIZE_TITLE: f32 = 20.0;
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;
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
        Color32::from_rgba_unmultiplied(224, 135, 48, (25.0 + 155.0 * t.sqrt()) as u8)
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
    // set_style targets the currently active theme. Pin light before installing:
    // the first native input event may otherwise switch from dark to light defaults.
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

    let mut style = (*ctx.style()).clone();
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
    let mut visuals = egui::Visuals::light();
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
    ctx.set_style(style);

    // egui 0.31 exposes one focus stroke, shared with active widgets. Paint two
    // layers after widgets instead. read_response only knows the interaction rect:
    // partly scrolled/clipped widgets get a ring around their visible rect. Pointer
    // focus also receives the ring; native OS dialogs are outside egui's control.
    ctx.on_end_pass(
        "DADS focus ring",
        std::sync::Arc::new(|ctx| {
            let focused = ctx.memory(|m| m.focused());
            if let Some(response) = focused.and_then(|id| ctx.read_response(id)) {
                if response.enabled() && response.interact_rect.is_positive() {
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

pub fn tab<T: PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    target: T,
    label: &str,
) -> egui::Response {
    let selected = *value == target;
    let mut response = ui.selectable_label(
        selected,
        RichText::new(label).color(if selected { PRIMARY } else { TEXT }),
    );
    if selected {
        ui.painter().hline(
            response.rect.x_range(),
            response.rect.bottom(),
            Stroke::new(2.0_f32, PRIMARY),
        );
    }
    if response.clicked() && !selected {
        *value = target;
        response.mark_changed();
    }
    response
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
        check(FOCUS_RING, FOCUS_INNER, 3.0);
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
        let style = ctx.style();
        assert!(style.text_styles.values().all(|f| f.size >= SIZE_CAPTION));
        assert_eq!(
            style.text_styles[&egui::TextStyle::Heading],
            bold_font(SIZE_HEADING_M)
        );
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for font in [
                    FontId::proportional(SIZE_BODY),
                    FontId::monospace(SIZE_BODY),
                    bold_font(SIZE_BODY),
                ] {
                    assert!(ui.fonts(|f| f.has_glyphs(&font, "日本語 ▶ ↶ ↷ ✓")));
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
