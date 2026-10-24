use super::components::short_name;
use super::*;
use raytrace_core::screen::Profile;

impl App {
    pub(super) fn export_current_screen(&mut self, dir: &std::path::Path, ctx: &egui::Context) {
        let Some(r) = &self.result else {
            return;
        };
        let Some(map) = r.report.detectors.get(self.detector).cloned() else {
            self.error = Some("出力するスクリーンがありません".into());
            return;
        };
        let dir = dir.to_path_buf();
        let reference = r.report.budget.launched;
        let plane = r.scene.reference_plane.clone();
        let cursor = self.screen_display.cursor_mm;
        let roi = self.screen_display.roi;
        let display = raytrace_io::export::DisplaySettings {
            peak_normalized: self.screen_display.normalization == ScreenNormalization::Peak,
            square_root: self.screen_display.scale == ScreenScale::SquareRoot,
            maximum_percent: self.screen_display.maximum_percent,
        };
        let conditions = serde_json::json!({"calculation_software_version":r.calculation.software_version,"model":r.scene.name,"wavelengths_um":r.calculation.launch_wavelengths_um,
            "input_rays":r.calculation.input_count(),"sources":r.scene.sources,"pupils":r.calculation.pupils,"trace_settings":r.scene.settings,"display":display});
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.status = "スクリーンを書き出し中…".into();
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                raytrace_io::export::write_screen(&dir, &map, &plane, reference, display)
                    .map_err(|e| e.to_string())?;
                raytrace_io::export::write_postprocessing(&dir, &map, reference, cursor, roi)
                    .map_err(|e| e.to_string())?;
                std::fs::write(
                    dir.join("conditions.json"),
                    serde_json::to_vec_pretty(&conditions).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                Ok(dir)
            })();
            let _ = tx.send(Job::Exported(result));
            context.request_repaint();
        });
    }

    pub(super) fn screen_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(r) = self.result.as_mut() else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.label(
                    egui::RichText::new("計算するとスクリーンの分布が表示されます")
                        .size(17.0)
                        .color(MUTED),
                );
            });
            return;
        };
        if r.report.detectors.is_empty() {
            ui.label("光学系にスクリーンを追加して計算してください。");
            return;
        }
        ui.horizontal(|ui| {
            let old = self.detector;
            self.detector = self.detector.min(r.report.detectors.len() - 1);
            egui::ComboBox::from_id_salt("detector")
                .width(230.0)
                .selected_text(short_name(&r.report.detectors[self.detector].name, 26))
                .show_ui(ui, |ui| {
                    for (i, d) in r.report.detectors.iter().enumerate() {
                        ui.selectable_value(&mut self.detector, i, &d.name);
                    }
                });
            if old != self.detector {
                r.texture = None;
                r.screen = None;
                r.roi_cache = None;
                self.screen_display.cursor_mm = None;
                self.screen_display.roi = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("光量の内訳").clicked() {
                    self.details = Some(Details::Budget);
                }
            });
        });
        ui.add_space(8.0);
        let height = ui.available_height().max(280.0);
        egui::ScrollArea::vertical()
            .id_salt("screen_workspace")
            .show(ui, |ui| {
                draw_detector(ui, ctx, r, self.detector, height, &mut self.screen_display);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("基準面：{} = 100%", r.scene.reference_plane))
                        .size(11.0)
                        .color(MUTED),
                );
                if self.stale {
                    ui.label(egui::RichText::new("前回の計算結果を表示中").color(MUTED));
                }
            });
    }
}

fn draw_detector(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    result: &mut ResultView,
    index: usize,
    height: f32,
    display: &mut ScreenDisplay,
) {
    let old_display = (
        display.scale,
        display.normalization,
        display.maximum_percent,
    );
    let old_cursor = display.cursor_mm;
    if let Some(c) = &mut display.cursor_mm {
        for (a, v) in c.iter_mut().enumerate() {
            *v = v.clamp(
                -result.report.detectors[index].half_extent_mm[a],
                result.report.detectors[index].half_extent_mm[a],
            );
        }
    }
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("screen_normalization")
            .width(125.0)
            .selected_text(match display.normalization {
                ScreenNormalization::Peak => "ピーク = 1",
                ScreenNormalization::Reference => "基準光量比 [%]",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut display.normalization, ScreenNormalization::Peak, "ピーク = 1");
                ui.selectable_value(&mut display.normalization, ScreenNormalization::Reference, "基準光量比 [%]");
            });
        egui::ComboBox::from_id_salt("screen_scale")
            .width(65.0)
            .selected_text(match display.scale {
                ScreenScale::Linear => "線形",
                ScreenScale::SquareRoot => "平方根",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut display.scale, ScreenScale::Linear, "線形");
                ui.selectable_value(&mut display.scale, ScreenScale::SquareRoot, "平方根");
            });
        ui.checkbox(&mut display.half_maximum, "半値幅")
            .on_hover_text("最大画素を通るX・Y断面の半値交点を表示します。複数の山がある場合は主ピーク周囲の幅です。");
        ui.checkbox(&mut display.profiles, "断面");
        ui.menu_button("表示範囲",|ui| {
            let mut fixed=display.maximum_percent.is_some();
            if ui.checkbox(&mut fixed,"基準光量比の上端を固定").changed() {display.maximum_percent=fixed.then_some(1.0);display.normalization=ScreenNormalization::Reference;}
            if let Some(v)=&mut display.maximum_percent {ui.add(egui::DragValue::new(v).range(1e-10..=100.0).speed(0.01).suffix(" %"));}
        });
        ui.menu_button("断面位置・ROI",|ui| {
            let map=&result.report.detectors[index];
            let [hx,hy]=map.half_extent_mm;
            let mut cursor=display.cursor_mm.is_some();
            if ui.checkbox(&mut cursor,"断面位置を指定").changed() {display.cursor_mm=cursor.then_some([0.,0.]);display.profiles=true;}
            if let Some(c)=&mut display.cursor_mm {ui.horizontal(|ui|{ui.label("X");ui.add(egui::DragValue::new(&mut c[0]).range(-hx..=hx).speed(hx/100.).suffix(" mm"));ui.label("Y");ui.add(egui::DragValue::new(&mut c[1]).range(-hy..=hy).speed(hy/100.).suffix(" mm"));});ui.label("最も近い画素列を表示します。");}
            let mut roi=display.roi.is_some();
            if ui.checkbox(&mut roi,"ROIの光量を表示").changed() {display.roi=roi.then_some(raytrace_core::screen::ScreenRoi{minimum_mm:[-hx/2.,-hy/2.],maximum_mm:[hx/2.,hy/2.]});}
            if let Some(r)=&mut display.roi {
                for a in 0..2 {ui.horizontal(|ui|{ui.label(["X","Y"][a]);ui.add(egui::DragValue::new(&mut r.minimum_mm[a]).speed(map.half_extent_mm[a]/100.).suffix(" mm"));ui.label("〜");ui.add(egui::DragValue::new(&mut r.maximum_mm[a]).speed(map.half_extent_mm[a]/100.).suffix(" mm"));});}
            }
        });
    });
    if old_cursor != display.cursor_mm {
        result.screen = None;
    }
    if old_display
        != (
            display.scale,
            display.normalization,
            display.maximum_percent,
        )
    {
        result.texture = None;
    }
    let map = &result.report.detectors[index];
    if result.screen.is_none() {
        match ScreenAnalysis::at(map, display.cursor_mm) {
            Ok(analysis) => result.screen = Some(analysis),
            Err(error) => {
                ui.label(error.to_string());
                return;
            }
        }
    }
    let analysis = result.screen.as_ref().unwrap();
    let peak = analysis.peak_weight;
    let reference = result.report.budget.launched.max(1e-300);
    let maximum = display.maximum_weight(peak, reference);
    if result.texture.is_none() {
        let pixels = map
            .weights
            .chunks(map.pixels[0])
            .rev()
            .flatten()
            .map(|w| screen_color(w / maximum.max(1e-300), display.scale))
            .collect();
        result.texture = Some(ctx.load_texture(
            "detector",
            egui::ColorImage {
                size: map.pixels,
                pixels,
            },
            egui::TextureOptions::NEAREST,
        ));
    }
    let texture = result.texture.as_ref().unwrap();
    if display.profiles {
        ui.columns(2, |cols| {
            draw_detector_map(
                &mut cols[0],
                map,
                analysis,
                texture,
                reference,
                height,
                display,
            );
            let ui = &mut cols[1];
            if let Some([x, y]) = analysis.slice_position_mm {
                ui.label(format!("断面位置 X={x:.4}, Y={y:.4} mm"));
                for (axis, profile, half) in [
                    ("X", analysis.x_profile.as_ref(), map.half_extent_mm[0]),
                    ("Y", analysis.y_profile.as_ref(), map.half_extent_mm[1]),
                ] {
                    if let Some(profile) = profile {
                        draw_profile(ui, profile, axis, half, peak, display, reference);
                    } else {
                        ui.label(format!("{axis}断面：受光なし"));
                    }
                    ui.add_space(10.0);
                }
                ui.label(
                    egui::RichText::new(
                        "赤破線：各断面の最大値の50%とその交点。画素中心の値を線形補間。",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            } else {
                ui.label("受光がありません。");
            }
        });
    } else {
        draw_detector_map(ui, map, analysis, texture, reference, height, display);
    }
    if let Some(roi) = display.roi {
        if result.roi_cache.as_ref().is_none_or(|(r, _)| *r != roi) {
            result.roi_cache = Some((
                roi,
                roi.integrate(map, reference).map_err(|e| e.to_string()),
            ));
        }
        match &result.roi_cache.as_ref().unwrap().1 {
            Ok(r) => {
                ui.label(format!(
                    "ROI {:.5}%（基準光量比） · 面積 {:.4} mm² · 平均 {:.5} %/mm²",
                    r.percent_of_reference, r.area_mm2, r.percent_of_reference_per_mm2
                ));
                ui.label(
                    egui::RichText::new(
                        "ROI端の画素は重なる面積で按分。画素内の光量密度を一定とする後処理です。",
                    )
                    .size(10.)
                    .color(MUTED),
                );
            }
            Err(e) => {
                ui.label(e);
            }
        }
    }
}

pub(super) fn draw_detector_map(
    ui: &mut egui::Ui,
    map: &raytrace_core::trace::DetectorMap,
    analysis: &ScreenAnalysis,
    texture: &egui::TextureHandle,
    reference: f64,
    height: f32,
    display: &ScreenDisplay,
) {
    let peak = analysis.peak_weight;
    let maximum = display.maximum_weight(peak, reference);
    let [hx, hy] = map.half_extent_mm;
    let aspect = (hx / hy) as f32;
    let width = (ui.available_width() - 90.0)
        .min((height - 150.0).max(80.0) * aspect)
        .max(30.0);
    let size = Vec2::new(width, width / aspect);
    let (area, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), size.y + 14.0),
        egui::Sense::hover(),
    );
    let rect = egui::Rect::from_min_size(
        Pos2::new(area.center().x - (size.x + 80.0) * 0.5, area.top() + 7.0),
        size,
    );
    let painter = ui.painter_at(area);
    painter.image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    let colorbar = egui::Rect::from_min_size(
        Pos2::new(rect.right() + 12.0, rect.top()),
        Vec2::new(10.0, size.y),
    );
    for i in 0..64 {
        let fraction = i as f64 / 63.0;
        painter.rect_filled(
            egui::Rect::from_min_max(
                Pos2::new(
                    colorbar.left(),
                    colorbar.bottom() - size.y * (i + 1) as f32 / 64.0,
                ),
                Pos2::new(
                    colorbar.right(),
                    colorbar.bottom() - size.y * i as f32 / 64.0,
                ),
            ),
            0.0,
            screen_color(if maximum > 0.0 { fraction } else { 0.0 }, display.scale),
        );
    }
    for fraction in [0.0, 0.5, 1.0] {
        let value = match display.normalization {
            ScreenNormalization::Peak => {
                if peak > 0.0 {
                    fraction
                } else {
                    0.0
                }
            }
            ScreenNormalization::Reference => fraction * 100.0 * maximum / reference,
        };
        let label = match display.normalization {
            ScreenNormalization::Peak => format!("{value:.1}"),
            ScreenNormalization::Reference => format!("{value:.3}%"),
        };
        painter.text(
            Pos2::new(
                colorbar.right() + 5.0,
                colorbar.bottom() - size.y * fraction as f32,
            ),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(9.0),
            MUTED,
        );
    }
    if display.half_maximum {
        draw_half_width_guides(&painter, rect, analysis, [hx, hy]);
    }
    if let Some(roi) = display.roi {
        if roi
            .minimum_mm
            .iter()
            .chain(&roi.maximum_mm)
            .all(|v| v.is_finite())
            && (0..2).all(|a| roi.minimum_mm[a] < roi.maximum_mm[a])
        {
            let pos = |v: [f64; 2]| {
                Pos2::new(
                    rect.left() + ((v[0].clamp(-hx, hx) + hx) / (2. * hx)) as f32 * rect.width(),
                    rect.bottom() - ((v[1].clamp(-hy, hy) + hy) / (2. * hy)) as f32 * rect.height(),
                )
            };
            painter.rect_stroke(
                egui::Rect::from_two_pos(pos(roi.minimum_mm), pos(roi.maximum_mm)),
                0.,
                Stroke::new(1.5_f32, Color32::from_rgb(46, 139, 87)),
                egui::StrokeKind::Inside,
            );
        }
    }
    ui.label(
        egui::RichText::new(format!(
            "X ±{hx:.3} / Y ±{hy:.3} mm  ·  画素 {:.4} × {:.4} mm",
            analysis.pixel_pitch_mm[0], analysis.pixel_pitch_mm[1],
        ))
        .size(10.0)
        .color(MUTED),
    );
    ui.label(
        egui::RichText::new(format!(
            "このスクリーン {:.3}%  ·  最大画素 {:.5}%（基準光量比）",
            100.0 * map.weights.iter().sum::<f64>() / reference,
            100.0 * peak / reference,
        ))
        .size(11.0)
        .color(MUTED),
    );
    if display.half_maximum {
        let width = |axis: &str, p: &Option<Profile>| match p {
            None => format!("{axis}: 受光なし"),
            Some(p) => match p.half_maximum.width_mm() {
                Some(v) => format!("{axis}: {v:.4} mm"),
                None => format!("{axis}: 半値交点不足"),
            },
        };
        ui.label(egui::RichText::new(format!(
            "FWHM  {}  /  {}",
            width("X", &analysis.x_profile), width("Y", &analysis.y_profile)
        )).size(11.0).color(HALF_COLOR))
            .on_hover_text("画素中心の値を線形補間した幅です。平滑化は行いません。半値の交点が両側で得られると幅を表示します。");
        if [&analysis.x_profile, &analysis.y_profile]
            .iter()
            .any(|p| p.as_ref().is_some_and(|p| p.half_maximum.multiple_lobes))
        {
            ui.label(
                egui::RichText::new("複数の山：主ピーク周囲の半値幅")
                    .size(10.0)
                    .color(MUTED),
            );
        }
    }
}

pub(super) fn screen_color(fraction: f64, scale: ScreenScale) -> Color32 {
    let [r, g, b] = raytrace_io::export::rgb(fraction, scale == ScreenScale::SquareRoot);
    Color32::from_rgb(r, g, b)
}

fn dashed_line(painter: &egui::Painter, a: Pos2, b: Pos2) {
    let distance = a.distance(b);
    if distance <= 0.0 {
        return;
    }
    let direction = (b - a) / distance;
    for i in 0..(distance / 7.0).ceil() as usize {
        let start = i as f32 * 7.0;
        painter.line_segment(
            [
                a + direction * start,
                a + direction * (start + 4.0).min(distance),
            ],
            Stroke::new(1.0_f32, HALF_COLOR),
        );
    }
}

fn draw_half_width_guides(
    painter: &egui::Painter,
    rect: egui::Rect,
    a: &ScreenAnalysis,
    half: [f64; 2],
) {
    if let Some(p) = &a.x_profile {
        for value in [p.half_maximum.lower_mm, p.half_maximum.upper_mm]
            .into_iter()
            .flatten()
        {
            let x = rect.left() + ((value + half[0]) / (2.0 * half[0])) as f32 * rect.width();
            dashed_line(
                painter,
                Pos2::new(x, rect.top()),
                Pos2::new(x, rect.bottom()),
            );
        }
    }
    if let Some(p) = &a.y_profile {
        for value in [p.half_maximum.lower_mm, p.half_maximum.upper_mm]
            .into_iter()
            .flatten()
        {
            let y = rect.bottom() - ((value + half[1]) / (2.0 * half[1])) as f32 * rect.height();
            dashed_line(
                painter,
                Pos2::new(rect.left(), y),
                Pos2::new(rect.right(), y),
            );
        }
    }
    if let Some([x, y]) = a.slice_position_mm {
        let p = Pos2::new(
            rect.left() + ((x + half[0]) / (2.0 * half[0])) as f32 * rect.width(),
            rect.bottom() - ((y + half[1]) / (2.0 * half[1])) as f32 * rect.height(),
        );
        painter.line_segment(
            [p - Vec2::new(4.0, 0.0), p + Vec2::new(4.0, 0.0)],
            Stroke::new(1.0_f32, HALF_COLOR),
        );
        painter.line_segment(
            [p - Vec2::new(0.0, 4.0), p + Vec2::new(0.0, 4.0)],
            Stroke::new(1.0_f32, HALF_COLOR),
        );
    }
}

pub(super) fn draw_profile(
    ui: &mut egui::Ui,
    profile: &Profile,
    axis: &str,
    half_extent: f64,
    peak: f64,
    display: &ScreenDisplay,
    reference: f64,
) {
    let maximum = display.maximum_weight(peak, reference).max(1e-300);
    let top = match display.normalization {
        ScreenNormalization::Peak => "1.0".into(),
        ScreenNormalization::Reference => format!("{:.5}%", maximum * 100.0 / reference),
    };
    ui.label(format!("{axis}断面 · 上端 {top}"));
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(248, 250, 253));
    let inner = rect.shrink2(Vec2::new(10.0, 16.0));
    let x =
        |v: f64| inner.left() + ((v + half_extent) / (2.0 * half_extent)) as f32 * inner.width();
    let y = |v: f64| {
        let fraction = v / maximum;
        let t = match display.scale {
            ScreenScale::Linear => fraction,
            ScreenScale::SquareRoot => fraction.sqrt(),
        };
        inner.bottom() - t as f32 * inner.height()
    };
    painter.with_clip_rect(inner).add(egui::Shape::line(
        profile
            .weights
            .iter()
            .enumerate()
            .map(|(i, w)| Pos2::new(x(profile.coordinate_mm(i)), y(*w)))
            .collect(),
        Stroke::new(1.5_f32, BLUE),
    ));
    if display.half_maximum {
        dashed_line(
            &painter,
            Pos2::new(inner.left(), y(profile.peak_weight() * 0.5)),
            Pos2::new(inner.right(), y(profile.peak_weight() * 0.5)),
        );
        for value in [profile.half_maximum.lower_mm, profile.half_maximum.upper_mm]
            .into_iter()
            .flatten()
        {
            dashed_line(
                &painter,
                Pos2::new(x(value), inner.top()),
                Pos2::new(x(value), inner.bottom()),
            );
        }
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom()),
        egui::Align2::CENTER_BOTTOM,
        format!("{axis} [{:.3} … {half_extent:.3}] mm", -half_extent),
        egui::FontId::proportional(10.0),
        MUTED,
    );
}
