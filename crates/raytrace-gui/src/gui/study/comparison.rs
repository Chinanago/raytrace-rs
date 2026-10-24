use super::*;
use crate::gui::detector::{draw_detector_map, draw_profile, screen_color};

impl App {
    pub(in crate::gui) fn load_comparison(&mut self, ctx: &egui::Context) {
        let request = (|| -> Result<Vec<(PathBuf, String, usize)>, String> {
            let m = self.study.manifest.as_ref().ok_or("走査結果がありません")?;
            let dir = self.study.directory.as_ref().ok_or("保存先がありません")?;
            self.study
                .pair
                .iter()
                .enumerate()
                .map(|(slot, &i)| {
                    let c = m.cases.get(i).ok_or("比較するケースを選んでください")?;
                    let file = c
                        .run_file
                        .as_ref()
                        .ok_or("完了したケースを選んでください")?;
                    Ok((
                        dir.join(file),
                        case_label(c, &m.plan),
                        self.study.comparison_detectors[slot],
                    ))
                })
                .collect()
        })();
        let request = match request {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<Comparison>, String> {
                let mut images = Vec::new();
                for (path, label, detector) in request {
                    let r = RunRecord::open(&path).map_err(|e| e.to_string())?;
                    let map = r
                        .report
                        .detectors
                        .get(detector)
                        .cloned()
                        .ok_or("スクリーンが見つかりません")?;
                    images.push(Comparison {
                        label,
                        reference_plane: r.calculation.scene.reference_plane,
                        map: map.clone(),
                        reference: r.report.budget.launched,
                        analysis: ScreenAnalysis::from_detector(&map).map_err(|e| e.to_string())?,
                        texture: None,
                    });
                }
                if images[0].reference_plane != images[1].reference_plane {
                    return Err("比較する基準面を揃えてください".into());
                }
                Ok(images)
            })();
            let _ = tx.send(Job::Compared(result));
            context.request_repaint();
        });
    }
    pub(in crate::gui) fn comparison_window(&mut self, ctx: &egui::Context) {
        if !self.study.comparison_open {
            return;
        }
        let mut open = true;
        egui::Window::new("スクリーン比較")
            .max_height((ctx.screen_rect().height() - 40.0).max(300.0))
            .open(&mut open)
            .default_width(900.0)
            .default_height(720.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("共通の上端");
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.study.comparison_max)
                                .range(1e-10..=100.0)
                                .speed(0.01)
                                .suffix(" %"),
                        )
                        .changed()
                    {
                        for c in &mut self.study.comparisons {
                            c.texture = None;
                        }
                    }
                });
                let display = ScreenDisplay {
                    normalization: ScreenNormalization::Reference,
                    maximum_percent: Some(self.study.comparison_max),
                    ..ScreenDisplay::default()
                };
                egui::ScrollArea::vertical()
                    .id_salt("comparison_images")
                    .max_height((ctx.screen_rect().height() - 130.0).max(200.0))
                    .show(ui, |ui| {
                        ui.columns(2, |cols| {
                            for (i, c) in self.study.comparisons.iter_mut().take(2).enumerate() {
                                let ui = &mut cols[i];
                                ui.label(egui::RichText::new(&c.label).strong());
                                if c.texture.is_none() {
                                    let max = display
                                        .maximum_weight(c.analysis.peak_weight, c.reference)
                                        .max(1e-300);
                                    let pixels = c
                                        .map
                                        .weights
                                        .chunks(c.map.pixels[0])
                                        .rev()
                                        .flatten()
                                        .map(|w| screen_color(w / max, display.scale))
                                        .collect();
                                    c.texture = Some(ctx.load_texture(
                                        format!("comparison_{i}"),
                                        egui::ColorImage {
                                            size: c.map.pixels,
                                            pixels,
                                        },
                                        egui::TextureOptions::NEAREST,
                                    ));
                                }
                                draw_detector_map(
                                    ui,
                                    &c.map,
                                    &c.analysis,
                                    c.texture.as_ref().unwrap(),
                                    c.reference,
                                    330.0,
                                    &display,
                                );
                                for (axis, profile, extent) in [
                                    ("X", c.analysis.x_profile.as_ref(), c.map.half_extent_mm[0]),
                                    ("Y", c.analysis.y_profile.as_ref(), c.map.half_extent_mm[1]),
                                ] {
                                    if let Some(p) = profile {
                                        draw_profile(
                                            ui,
                                            p,
                                            axis,
                                            extent,
                                            c.analysis.peak_weight,
                                            &display,
                                            c.reference,
                                        );
                                    }
                                }
                                ui.label(
                                    egui::RichText::new(format!(
                                        "基準面：{} = 100%",
                                        c.reference_plane
                                    ))
                                    .size(10.0)
                                    .color(MUTED),
                                );
                            }
                        });
                    });
            });
        self.study.comparison_open = open;
    }
}
