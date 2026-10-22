use super::controls::{number_row, section_label};
use super::*;

impl App {
    pub(super) fn details_window(&mut self, ctx: &egui::Context) {
        let Some(mut page) = self.details else {
            return;
        };
        let mut open = true;
        egui::Window::new("設定とレポート")
            .open(&mut open)
            .collapsible(false)
            .default_width(540.0)
            .default_height(500.0)
            .min_width(410.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut page, Details::Calculation, "計算条件");
                    ui.selectable_value(&mut page, Details::Import, "読込レポート");
                    ui.selectable_value(&mut page, Details::Budget, "光量収支");
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("details_content")
                    .show(ui, |ui| match page {
                        Details::Calculation => self.calculation_controls(ui),
                        Details::Import => self.import_report(ui, ctx),
                        Details::Budget => self.power_budget(ui),
                    });
            });
        self.details = if self.details.is_some() {
            open.then_some(page)
        } else {
            None
        };
    }

    fn import_report(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            let mut changed = false;
            section_label(ui, "計算に使う条件");
            if self.project.issues.iter().any(|i| i.code == "coating") || self.project.scene.surfaces.iter().any(|s| s.coating.is_some()) {
                changed |= ui.checkbox(&mut self.project.allow_uncoated, "未コートFresnelで計算").changed();
                ui.label(egui::RichText::new("元のコーティング名を保持し、界面の反射率は未コートとして計算します。").size(11.0).color(MUTED));
            }
            let mut mono = self.project.monochromatic_override_um.is_some();
            if ui.checkbox(&mut mono, "全光源の計算波長を単色で指定").changed() {
                self.project.monochromatic_override_um = mono.then_some(0.465);
                changed = true;
            }
            if let Some(value) = self.project.monochromatic_override_um.as_mut() {
                let mut nm = *value * 1000.0;
                if number_row(ui, "計算波長", &mut nm, " nm", 1.0, 100.0..=20000.0) {
                    *value = nm / 1000.0;
                    changed = true;
                }
            }
            if changed { self.changed(); }
            if let Some(r) = &self.result {
                if !r.calculation.pupils.is_empty() {
                    egui::CollapsingHeader::new(if self.stale { "前回の計算結果の入射瞳" } else { "この計算結果の入射瞳" }).show(ui, |ui| {
                        for pupil in &r.calculation.pupils {
                            ui.label(format!("光源 {}", pupil.source_index + 1));
                            if let Some(g) = &pupil.geometry {
                                let p = g.entrance_pose.translation_mm.0;
                                ui.label(format!("入射瞳中心：X {:.6} / Y {:.6} / Z {:.6} mm", p[0], p[1], p[2]));
                                ui.label(format!("入射瞳半径：{:.6} mm · STOP基準半径：{:.6} mm", g.entrance_radius_mm, g.target_stop_radius_mm));
                                let mode = match g.aiming { raytrace_core::illumination::RayAiming::Off => "近軸瞳へ照射", raytrace_core::illumination::RayAiming::Paraxial => "近軸半径のSTOPへ照準", raytrace_core::illumination::RayAiming::Real => "実光線半径のSTOPへ照準" };
                                ui.label(format!("主波長 {:.1} nm · {mode}", g.primary_wavelength_um * 1000.));
                            }
                            if let Some(e) = &pupil.error { ui.colored_label(Color32::from_rgb(173, 93, 20), e); }
                        }
                    });
                }
            }
            section_label(ui, "読込の確認");
            let report = self.project.compatibility_report();
            ui.label(egui::RichText::new(if report.traceable { "指定した条件で計算できます" } else { "計算前に確認が必要な項目があります" })
                .color(if report.traceable { BLUE } else { Color32::from_rgb(173, 93, 20) }));
            for component in report.components {
                egui::CollapsingHeader::new(format!("{}{}", component.name, if component.enabled { "" } else { "（使用停止）" }))
                    .id_salt(("compatibility", component.component)).default_open(true).show(ui, |ui| {
                    for entry in &component.entries {
                        ui.label(egui::RichText::new(format!("{} · {}", entry.status.label(), entry.code)).size(11.0).color(
                            if entry.status.blocks() && component.enabled { Color32::from_rgb(173, 93, 20) } else { MUTED }));
                        ui.label(egui::RichText::new(&entry.message).size(12.0));
                    }
                    egui::CollapsingHeader::new("計算波長・屈折率・吸収").id_salt(("optics", component.component)).show(ui, |ui| {
                        for material in &component.materials {
                            ui.label(egui::RichText::new(&material.name).strong().size(11.0));
                            for sample in &material.samples {
                                ui.label(egui::RichText::new(format!("{:.2} nm · n={} · 吸収係数 {} /mm", sample.wavelength_um * 1000.0,
                                    sample.refractive_index.map_or_else(|| "範囲外".into(), |v| format!("{v:.7}")),
                                    sample.absorption_per_mm.map_or_else(|| "範囲外".into(), |v| format!("{v:.6}")))).size(11.0).color(MUTED));
                            }
                        }
                    });
                });
            }
            section_label(ui, "材料・コーティングカタログ");
            if ui.button("カタログフォルダーを追加…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.catalogs.push(dir);
                    self.status = "材料カタログを追加しました。次のファイル読込から使います".into();
                }
            }
            for dir in &self.catalogs { ui.label(egui::RichText::new(dir.display().to_string()).size(11.0).color(MUTED)); }
            if let Some(path) = self.path.clone() {
                if ui.button("現在のファイルを読み直す").on_hover_text("配置・編集をファイルの内容へ戻し、追加した材料カタログを使って読み直します").clicked() {
                    self.load(path, ctx);
                }
            }
        });
    }

    fn power_budget(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.result else {
            ui.label("計算後に光量収支を表示します。");
            return;
        };
        if self.stale {
            ui.colored_label(Color32::from_rgb(173, 93, 20), "前回の計算結果");
        }
        let b = &r.report.budget;
        let reference = b.launched.max(1e-300);
        ui.add_space(8.0);
        ui.label(format!("基準面：{} = 100%", r.scene.reference_plane));
        ui.add_space(10.0);
        egui::Grid::new("budget")
            .num_columns(2)
            .spacing([30.0, 10.0])
            .show(ui, |ui| {
                for (label, value) in [
                    ("スクリーン合計", b.detected),
                    ("追跡領域外", b.escaped),
                    ("幾何学的な枠外・外れ", b.geometric_miss),
                    (
                        "逐次追跡で追わない界面反射",
                        b.sequential_reflection_not_followed,
                    ),
                    ("全反射（シーケンシャル）", b.sequential_tir),
                    ("界面での吸収", b.surface_absorbed),
                    ("材料内部での吸収", b.bulk_absorbed),
                    ("分岐・深さの打切り", b.branch_cutoff + b.interaction_limit),
                    ("未解決の交差", b.numerical_failure),
                    ("媒質の不整合", b.medium_mismatch),
                    ("合計", b.accounted()),
                ] {
                    ui.label(label);
                    ui.label(format!("{:.4}%", value * 100.0 / reference));
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(format!(
                "{} 本 · {:.2} ms · {} 面",
                r.calculation.input_count(),
                r.seconds * 1000.0,
                r.scene.surfaces.len()
            ))
            .size(11.0)
            .color(MUTED),
        );
        section_label(ui, "追跡の診断");
        let fallbacks = r
            .report
            .surfaces
            .iter()
            .map(|s| s.normal_fallbacks)
            .sum::<u64>();
        if fallbacks > 0 {
            egui::CollapsingHeader::new(format!("CAD法線の補正：{fallbacks}回")).show(ui,|ui|{
                ui.label("補間法線の反射・屈折が三角形の境界と食い違う場合、三角形の幾何法線を使用します。CADの分割精度を変えた収束確認の対象です。");
                for s in r.report.surfaces.iter().filter(|s|s.normal_fallbacks>0) {ui.label(format!("{}：{}回",s.name,s.normal_fallbacks));}
            });
        }
        if r.report.diagnostics.is_empty() {
            ui.label(
                egui::RichText::new("未解決の光線・打切りはありません")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        let mut locate = None;
        for (i, d) in r.report.diagnostics.iter().enumerate() {
            let name = d
                .surface
                .and_then(|s| r.scene.surfaces.get(s))
                .map(|s| s.name.as_str())
                .unwrap_or("面の特定なし");
            let reason = match d.reason.as_str() {
                "sequential_tir" => "全反射（シーケンシャル）",
                "medium_mismatch" => "媒質の不整合",
                "numerical_failure" => "未解決の交差",
                "branch_cutoff" => "弱い分岐の打切り",
                "interaction_limit" => "回数上限の打切り",
                _ => &d.reason,
            };
            egui::CollapsingHeader::new(format!(
                "{reason} · {:.6}% · {name}",
                d.weight * 100.0 / reference
            ))
            .id_salt(("diagnostic", i))
            .show(ui, |ui| {
                ui.label(format!(
                    "終端分岐数：{}（発射IDの重複を含む）",
                    d.branch_count
                ));
                ui.label(format!(
                    "代表光線 ID {} · 深さ {}",
                    d.example.ray.id, d.example.depth
                ));
                ui.label(format!("最終位置 [mm]：{:?}", d.example.ray.position_mm.0));
                ui.label(format!(
                    "方向：{:?} · 波長 {:.3} nm",
                    d.example.ray.direction.0,
                    d.example.ray.wavelength_um * 1000.0
                ));
                if let Some(expected) = d.expected_medium {
                    ui.label(format!(
                        "入射媒質 {} · 境界が要求する媒質 {}",
                        d.actual_medium, expected
                    ));
                }
                if let Some(h) = &d.intersection {
                    ui.label(format!("交差探索の開始位置 [mm]：{:?}", h.origin_mm.0));
                    ui.label(format!("交点までの距離：{:.9e} mm", h.distance_mm));
                    ui.label(format!(
                        "幾何法線：{:?} · 光学法線：{:?}",
                        h.geometric_normal.0, h.normal.0
                    ));
                }
                if d.reason == "numerical_failure" {
                    ui.label("位置は交差探索を開始した点です。");
                }
                if ["branch_cutoff", "interaction_limit"].contains(&d.reason.as_str()) {
                    ui.label("面は直前に相互作用した面です。");
                }
                if ui
                    .add_enabled(
                        !self.stale && d.surface.is_some(),
                        egui::Button::new("配置で確認"),
                    )
                    .clicked()
                {
                    locate = d.surface;
                }
            });
        }
        if r.report.diagnostic_overflow.branch_count > 0 {
            ui.label(format!(
                "代表例の保存上限を超えた診断：{} 分岐 · {:.6}%",
                r.report.diagnostic_overflow.branch_count,
                r.report.diagnostic_overflow.weight * 100.0 / reference
            ));
        }
        if let Some(surface) = locate {
            self.details = None;
            self.view = View::Layout;
            if self.project.assembly.is_some() {
                if let Some(index) = r.scene.surfaces[surface]
                    .name
                    .split(':')
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
                    .and_then(|i| i.checked_sub(1))
                {
                    self.component = index;
                    self.source = 0;
                }
            } else {
                self.selected = surface;
                self.source_selected = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::test_support::*;
    use raytrace_core::trace::{trace, TraceOptions};
    #[test]
    fn diagnostic_navigation_selects_the_component_without_changing_the_model() {
        let ctx = egui::Context::default();
        ctx.style_mut(|s| s.animation_time = 0.0);
        let mut app = App::demo();
        for source in &mut app.project.assembly.as_mut().unwrap().components[0]
            .project
            .scene
            .sources
        {
            source.medium = 1;
            source.count = 4;
        }
        app.project.refresh_assembly().unwrap();
        let scene = app.project.trace_scene().unwrap();
        let prepared = scene.clone().prepare().unwrap();
        let rays = prepared.generate_rays().unwrap();
        let report = trace(&prepared, &rays, TraceOptions::default());
        assert_eq!(report.diagnostics[0].reason, "medium_mismatch");
        let label = format!(
            "媒質の不整合 · {:.6}% · {}",
            report.diagnostics[0].weight * 100.0 / report.budget.launched,
            scene.surfaces[report.diagnostics[0].surface.unwrap()].name
        );
        let before = serde_json::to_value(&app.project).unwrap();
        app.result = Some(ResultView {
            revision: app.history.revision,
            calculation: RunContext::capture(
                &app.project,
                &scene,
                None,
                1,
                TraceOptions::default(),
            ),
            scene,
            report,
            seconds: 0.1,
            texture: None,
            screen: None,
            roi_cache: None,
        });
        app.details = Some(Details::Budget);
        frame(&mut app, &ctx, vec![]);
        let out = frame(&mut app, &ctx, vec![]);
        let header = text_rect(&out, &label).expect("diagnostic group").center();
        click(&mut app, &ctx, header);
        let mut out = frame(&mut app, &ctx, vec![]);
        if text_rect(&out, "配置で確認").is_none() {
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(header),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -600.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            for _ in 0..30 {
                out = frame(&mut app, &ctx, vec![]);
            }
        }
        let button = text_rect(&out, "配置で確認")
            .expect("navigation button")
            .center();
        click(&mut app, &ctx, button);
        assert_eq!(app.view, View::Layout);
        assert_eq!(app.component, 1);
        assert_eq!(app.details, None);
        assert_eq!(before, serde_json::to_value(&app.project).unwrap());
    }
}
