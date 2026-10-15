//! The layout workspace keeps placement visible; reports and tracing controls open on demand.
use super::*;
use crate::math::Pose;

impl App {
    pub(super) fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(62.0)
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(16.0))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("●").color(BLUE).size(19.0));
                    ui.label(
                        egui::RichText::new("Raytrace")
                            .color(INK)
                            .size(22.0)
                            .strong(),
                    );
                    ui.add_space(12.0);
                    let title = format!(
                        "{}{}",
                        self.project.scene.name,
                        if self.dirty { " •" } else { "" }
                    );
                    ui.add_sized(
                        [(ui.available_width() - 360.0).clamp(60.0, 360.0), 24.0],
                        egui::Label::new(egui::RichText::new(title).size(12.0).color(MUTED))
                            .truncate(),
                    )
                    .on_hover_text(&self.project.scene.name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.job.is_none(),
                                egui::Button::new(
                                    egui::RichText::new("▶  計算する").color(Color32::WHITE),
                                )
                                .fill(BLUE)
                                .min_size(Vec2::new(118.0, 32.0)),
                            )
                            .clicked()
                        {
                            self.run(ctx);
                        }
                        if ui.button("計算設定").clicked() {
                            self.details = Some(Details::Calculation);
                        }
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            ui.menu_button("ファイル", |ui| self.file_menu(ui, ctx));
                        });
                    });
                });
            });
        egui::TopBottomPanel::top("workspace_tabs")
            .exact_height(43.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(18, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.view, View::Layout, "光学系");
                    ui.selectable_value(&mut self.view, View::Screen, "スクリーン");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("読込レポート").clicked() {
                            self.details = Some(Details::Import);
                        }
                        if !self.project.issues.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("{} 件", self.project.issues.len()))
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        }
                    });
                });
            });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("新規の光学系").clicked() {
            self.replace_project(Project::new_assembly(), Some(256), 0);
            self.changed();
            self.status = "部品を追加して光学系を組み立てます".into();
            ui.close_menu();
        }
        if ui.button("開く…").clicked() {
            ui.close_menu();
            if let Some(path) = optics_dialog().pick_file() {
                self.load(path, ctx);
            }
        }
        if ui.button("保存…").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("optics.raytrace.json")
                .add_filter("Raytrace", &["json"])
                .save_file()
            {
                match self.project.save(&path) {
                    Ok(()) => {
                        self.dirty = false;
                        self.status = "プロジェクトを保存しました".into();
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if ui
            .add_enabled(
                self.project.assembly.is_none(),
                egui::Button::new("ZMX書出し…"),
            )
            .on_hover_text("組立光学系はJSONプロジェクトで保存します")
            .clicked()
        {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("optics_edited.zmx")
                .add_filter("Zemax", &["zmx"])
                .save_file()
            {
                match zemax::export(&self.project, &path) {
                    Ok(()) => self.status = "ZMXを書き出しました".into(),
                    Err(e) => self.error = Some(e),
                }
            }
        }
        ui.separator();
        ui.menu_button("サンプルを開く", |ui| {
            if ui.button("光源＋レンズ＋ロッド").clicked() {
                self.replace_project(assembly::demo_project().expect("assembly demo"), None, 2);
                self.run(ctx);
                ui.close_menu();
            }
            for (name, text) in [
                (
                    "レンズ",
                    include_str!("../../examples/sequential_lens.json"),
                ),
                ("ロッド", include_str!("../../examples/integrator_rod.json")),
                ("ミラー", include_str!("../../examples/folded_mirror.json")),
            ] {
                if ui.button(name).clicked() {
                    self.replace_project(
                        Project::new(serde_json::from_str(text).unwrap()),
                        None,
                        0,
                    );
                    self.run(ctx);
                    ui.close_menu();
                }
            }
        });
    }

    fn replace_project(&mut self, project: Project, preview: Option<usize>, component: usize) {
        self.project = project;
        self.path = None;
        self.result = None;
        self.component = component;
        self.source = 0;
        self.selected = 0;
        self.source_selected = false;
        self.detector = 0;
        self.preview_limit = preview;
        self.dirty = false;
        self.stale = false;
        self.view = View::Layout;
        self.error = None;
    }

    fn add_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        for (label, kind) in [
            ("光源のZMX…", ComponentKind::Source),
            ("レンズのZMX…", ComponentKind::Optic),
        ] {
            if ui.button(label).clicked() {
                ui.close_menu();
                if let Some(path) = optics_dialog().pick_file() {
                    self.add_file(path, kind, ctx);
                }
            }
        }
        ui.separator();
        if ui.button("ロッド").clicked() {
            let demo: Scene =
                serde_json::from_str(include_str!("../../examples/integrator_rod.json")).unwrap();
            self.append_component(Component::rod(demo.materials[1].clone(), "Silica".into()));
            ui.close_menu();
        }
        if ui.button("スクリーン").clicked() {
            self.append_component(Component::detector());
            ui.close_menu();
        }
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        if self.view != View::Layout {
            return;
        }
        egui::SidePanel::left("parts")
            .exact_width(264.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(248, 250, 252))
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("部品").strong().size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            ui.menu_button("＋ 追加", |ui| self.add_menu(ui, ctx));
                        });
                    });
                });
                ui.add_space(8.0);
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    if self.project.assembly.is_some() {
                        self.components(ui);
                    } else {
                        self.surfaces(ui);
                    }
                });
            });
    }

    fn components(&mut self, ui: &mut egui::Ui) {
        let materials: Vec<_> = self
            .project
            .material_names
            .iter()
            .cloned()
            .zip(self.project.scene.materials.iter().cloned())
            .collect();
        let wavelength = self.wavelength();
        let mono = self.project.monochromatic_override_um;
        let assembly = self.project.assembly.as_mut().unwrap();
        if assembly.components.is_empty() {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("光源と光学部品を追加すると、ここに一覧が表示されます。")
                    .color(MUTED),
            );
            return;
        }
        self.component = self.component.min(assembly.components.len() - 1);
        let mut changed = false;
        egui::ScrollArea::vertical()
            .id_salt("part_list")
            .max_height((ui.available_height() * 0.32).clamp(100.0, 240.0))
            .show(ui, |ui| {
                for (i, c) in assembly.components.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= ui
                            .checkbox(&mut c.enabled, "")
                            .on_hover_text("この部品を使用")
                            .changed();
                        let text = format!("{:02}  {}", i + 1, short_name(&c.name, 15));
                        if ui
                            .add(
                                egui::Button::new(text)
                                    .min_size(Vec2::new(ui.available_width(), 26.0))
                                    .selected(self.component == i)
                                    .frame(self.component == i),
                            )
                            .on_hover_text(format!(
                                "{}\n{} · Z = {:.3} mm",
                                c.name,
                                kind_label(c.kind),
                                c.pose.translation_mm.0[2]
                            ))
                            .clicked()
                        {
                            self.component = i;
                            self.source = 0;
                        }
                    });
                }
            });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(10.0);
        let mut remove = false;
        egui::ScrollArea::vertical()
            .id_salt("part_inspector")
            .show(ui, |ui| {
                let c = &mut assembly.components[self.component];
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(kind_label(c.kind)).strong().color(INK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("操作", |ui| {
                            if ui.button("部品を削除").clicked() {
                                remove = true;
                                ui.close_menu();
                            }
                        });
                    });
                });
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut c.name).desired_width(f32::INFINITY))
                    .changed();
                ui.add_space(8.0);
                changed |= pose_controls(ui, &mut c.pose);
                ui.label(
                    egui::RichText::new(match c.kind {
                        ComponentKind::Source => "基準：発光配列の中心・最初の発射面",
                        ComponentKind::Optic => "基準：最初の光学面の頂点",
                        ComponentKind::Rod => "基準：ロッド入口の中心",
                        _ => "部品全体の位置と傾き",
                    })
                    .size(10.0)
                    .color(MUTED),
                );
                if let Some(r) = &mut c.rod {
                    section_label(ui, "寸法");
                    for (name, value) in [
                        ("幅 X", &mut r.width_mm),
                        ("幅 Y", &mut r.height_mm),
                        ("長さ", &mut r.length_mm),
                    ] {
                        changed |= number_row(ui, name, value, " mm", 0.1, 0.0001..=1e9);
                    }
                    section_label(ui, "材料");
                    egui::ComboBox::from_id_salt("rod_material")
                        .width(ui.available_width() - 8.0)
                        .selected_text(short_name(
                            c.project
                                .material_names
                                .get(1)
                                .map(String::as_str)
                                .unwrap_or("材料"),
                            21,
                        ))
                        .show_ui(ui, |ui| {
                            for (name, material) in &materials {
                                if ui.selectable_label(false, name).clicked() {
                                    c.project.scene.materials[1] = material.clone();
                                    c.project.material_names[1] =
                                        name.rsplit(" / ").next().unwrap_or(name).into();
                                    changed = true;
                                }
                            }
                        });
                    index_label(ui, &c.project.scene.materials[1], wavelength);
                } else if c.kind == ComponentKind::Detector {
                    if let Some(s) = c.project.scene.surfaces.first_mut() {
                        section_label(ui, "観測面");
                        changed |= detector_controls(ui, s);
                    }
                } else {
                    if !c.project.scene.sources.is_empty() {
                        section_label(ui, "発光条件");
                        changed |= source_controls(
                            ui,
                            &mut c.project.scene.sources,
                            &mut self.source,
                            true,
                            mono,
                        );
                    }
                    if !c.project.scene.surfaces.is_empty() {
                        egui::CollapsingHeader::new("読込部品の材料").show(ui, |ui| {
                            for (name, material) in c
                                .project
                                .material_names
                                .iter()
                                .zip(&c.project.scene.materials)
                                .skip(1)
                            {
                                ui.label(egui::RichText::new(name).size(11.0));
                                index_label(ui, material, wavelength);
                            }
                        });
                    }
                }
            });
        if remove {
            assembly.components.remove(self.component);
            self.component = self.component.saturating_sub(1);
            self.source = 0;
            changed = true;
        }
        if changed {
            self.refresh_components();
        }
    }

    // Standalone ZMX/JSON scenes retain their native source and surface editors.
    fn surfaces(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("surface_list")
            .max_height((ui.available_height() * 0.36).clamp(100.0, 260.0))
            .show(ui, |ui| {
                for (i, s) in self.project.scene.sources.iter().enumerate() {
                    if ui
                        .add(
                            egui::Button::new(format!(
                                "光源 {} · {:.0} nm",
                                i + 1,
                                s.wavelength_um * 1000.0
                            ))
                            .min_size(Vec2::new(ui.available_width(), 25.0))
                            .selected(self.source_selected && self.source == i)
                            .frame(self.source_selected && self.source == i),
                        )
                        .clicked()
                    {
                        self.source = i;
                        self.source_selected = true;
                    }
                }
                for (i, s) in self.project.scene.surfaces.iter().enumerate() {
                    if ui
                        .add(
                            egui::Button::new(short_name(&s.name, 20))
                                .min_size(Vec2::new(ui.available_width(), 25.0))
                                .selected(!self.source_selected && self.selected == i)
                                .frame(!self.source_selected && self.selected == i),
                        )
                        .on_hover_text(format!("{} · {}", s.name, shape_name(&s.shape)))
                        .clicked()
                    {
                        self.selected = i;
                        self.source_selected = false;
                    }
                }
            });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(10.0);
        let wavelength = self.wavelength();
        let mut changed = false;
        egui::ScrollArea::vertical()
            .id_salt("surface_inspector")
            .show(ui, |ui| {
                if self.source_selected {
                    changed |= source_controls(
                        ui,
                        &mut self.project.scene.sources,
                        &mut self.source,
                        false,
                        self.project.monochromatic_override_um,
                    );
                    return;
                }
                if self.project.scene.surfaces.is_empty() {
                    ui.label("光学面がありません。読込レポートを確認してください。");
                    return;
                }
                self.selected = self.selected.min(self.project.scene.surfaces.len() - 1);
                let s = &mut self.project.scene.surfaces[self.selected];
                ui.label(egui::RichText::new(&s.name).strong());
                ui.label(
                    egui::RichText::new(shape_name(&s.shape))
                        .size(11.0)
                        .color(MUTED),
                );
                changed |= pose_controls(ui, &mut s.pose);
                match &mut s.shape {
                    Shape::Sphere { radius_mm } | Shape::EvenAsphere { radius_mm, .. } => {
                        changed |= number_row(ui, "曲率半径", radius_mm, " mm", 0.1, -1e12..=1e12);
                    }
                    Shape::TriangleMesh { triangles, .. } => {
                        ui.label(
                            egui::RichText::new(format!("CAD · {} triangles", triangles.len()))
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                    _ => (),
                }
                if let Shape::EvenAsphere { conic, .. } = &mut s.shape {
                    changed |= number_row(ui, "円錐定数", conic, "", 0.001, -1e12..=1e12);
                }
                if matches!(s.interaction, Interaction::Detector { .. }) {
                    section_label(ui, "観測面");
                    changed |= detector_controls(ui, s);
                }
                if let Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    ..
                } = &mut s.interaction
                {
                    section_label(ui, "材料");
                    for (label, value) in [("法線 − 側", minus_medium), ("法線 ＋ 側", plus_medium)]
                    {
                        ui.label(egui::RichText::new(label).size(11.0).color(MUTED));
                        egui::ComboBox::from_id_salt(label)
                            .width(ui.available_width() - 8.0)
                            .selected_text(short_name(
                                self.project
                                    .material_names
                                    .get(*value)
                                    .map(String::as_str)
                                    .unwrap_or("?"),
                                20,
                            ))
                            .show_ui(ui, |ui| {
                                for (i, name) in self.project.material_names.iter().enumerate() {
                                    changed |= ui.selectable_value(value, i, name).changed();
                                }
                            });
                        if let Some(m) = self.project.scene.materials.get(*value) {
                            index_label(ui, m, wavelength);
                        }
                    }
                }
            });
        if changed {
            self.changed();
        }
    }

    fn wavelength(&self) -> f64 {
        self.project
            .monochromatic_override_um
            .or_else(|| self.project.scene.sources.first().map(|s| s.wavelength_um))
            .unwrap_or(0.55)
    }

    pub(super) fn content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(20.0))
            .show(ctx, |ui| {
                if self.stale {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            Color32::from_rgb(173, 93, 20),
                            "配置・条件を変更しました。計算すると結果が更新されます。",
                        );
                    });
                    ui.add_space(6.0);
                }
                match self.view {
                    View::Layout => self.layout_view(ui),
                    View::Screen => self.screen_view(ui, ctx),
                }
            });
    }

    fn layout_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("projection")
                .width(116.0)
                .selected_text(["Z–X 側面", "Z–Y 上面", "X–Y 正面"][self.projection])
                .show_ui(ui, |ui| {
                    for (i, t) in ["Z–X 側面", "Z–Y 上面", "X–Y 正面"].iter().enumerate()
                    {
                        ui.selectable_value(&mut self.projection, i, *t);
                    }
                });
            ui.checkbox(&mut self.show_rays, "光線");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("表示", |ui| {
                    ui.checkbox(&mut self.equal_scale, "縦横を同じ縮尺にする");
                });
                if self.job.is_some() {
                    ui.spinner();
                }
            });
        });
        ui.add_space(10.0);
        let markers: Vec<_> = self
            .project
            .assembly
            .as_ref()
            .map(|a| {
                a.components
                    .iter()
                    .enumerate()
                    .map(|(i, c)| PartMarker {
                        index: i,
                        label: format!("{} {}", i + 1, kind_label(c.kind)),
                        position: c.pose.translation_mm,
                        enabled: c.enabled,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Geometry always comes from the editor; rays belong to a completed revision.
        let paths = visible_paths(self.result.as_ref(), self.stale, self.show_rays);
        let placement = draw_paths(
            ui,
            &self.project.scene,
            paths,
            self.projection,
            (ui.available_height() - 30.0).max(120.0),
            &markers,
            self.component,
            self.job.is_none(),
            self.equal_scale,
        );
        if let Some((index, delta)) = placement {
            if self.component != index {
                self.source = 0;
            }
            self.component = index;
            if delta.norm() > 0.0 {
                if let Some(c) = self
                    .project
                    .assembly
                    .as_mut()
                    .and_then(|a| a.components.get_mut(index))
                {
                    c.pose.translation_mm = c.pose.translation_mm + delta;
                }
                self.refresh_components();
            }
        }
    }

    fn screen_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
        self.details = open.then_some(page);
    }

    fn calculation_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            section_label(ui, "光線数");
            let mut preview = self.preview_limit.is_some();
            let old_preview = preview;
            ui.horizontal(|ui| {
                ui.radio_value(&mut preview, true, "プレビュー");
                ui.radio_value(&mut preview, false, "指定本数で計算");
            });
            if old_preview != preview {
                self.preview_limit = preview.then_some(256);
                self.stale = true;
            }
            ui.label(
                egui::RichText::new(
                    "プレビューは各光源256本まで。光路図は合計64本までのサンプルです。",
                )
                .size(11.0)
                .color(MUTED),
            );
            let mut changed = false;
            if let Some(first) = self.project.scene.sources.first() {
                let mut count = first.count;
                let mut sampling = first.sampling;
                let mixed = self
                    .project
                    .scene
                    .sources
                    .iter()
                    .any(|s| s.count != count || s.sampling != sampling);
                ui.add_space(10.0);
                ui.label("各光源の本数をまとめて設定");
                if ui
                    .add(
                        egui::DragValue::new(&mut count)
                            .range(1..=2_000_000)
                            .clamp_existing_to_range(false)
                            .speed(256),
                    )
                    .changed()
                {
                    edit_sources(&mut self.project, |s| s.count = count);
                    changed = true;
                }
                ui.label("サンプリング");
                if sampling_control(ui, &mut sampling) {
                    edit_sources(&mut self.project, |s| s.sampling = sampling);
                    changed = true;
                }
                if mixed {
                    ui.label(
                        egui::RichText::new(
                            "光源ごとに設定が異なります。上の値を変更すると全光源へ適用します。",
                        )
                        .size(11.0)
                        .color(MUTED),
                    );
                }
            } else {
                ui.label(egui::RichText::new("光源を追加すると本数を設定できます。").color(MUTED));
            }
            section_label(ui, "追跡方式");
            if self.project.assembly.is_some() {
                ui.label("非シーケンシャル");
                ui.label(
                    egui::RichText::new("組立光学系は、光線が次に当たる部品を探索して追跡します。")
                        .size(11.0)
                        .color(MUTED),
                );
            } else {
                ui.horizontal(|ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.project.scene.mode,
                            Mode::Sequential,
                            "シーケンシャル",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.project.scene.mode,
                            Mode::NonSequential,
                            "非シーケンシャル",
                        )
                        .changed();
                });
            }
            ui.add_space(12.0);
            egui::CollapsingHeader::new("追跡の詳細").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("計算スレッド");
                    ui.add(egui::DragValue::new(&mut self.threads).range(1..=64));
                });
                ui.horizontal(|ui| {
                    ui.label("最大相互作用回数");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.project.scene.settings.max_depth)
                                .range(1..=1000)
                                .clamp_existing_to_range(false),
                        )
                        .changed();
                });
            });
            if changed {
                self.refresh_components();
            }
        });
    }

    fn import_report(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            let mut changed = false;
            section_label(ui, "計算に使う条件");
            if self.project.issues.iter().any(|i| i.code == "coating") {
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
            section_label(ui, "読込の確認");
            if self.project.issues.is_empty() {
                ui.label("読込時の確認項目はありません。");
            } else {
                let blocked = self.project.trace_scene().is_err();
                ui.label(egui::RichText::new(if blocked { "計算前に確認が必要な項目があります" } else { "指定した条件で計算できます" })
                    .color(if blocked { Color32::from_rgb(173, 93, 20) } else { BLUE }));
                for issue in &self.project.issues {
                    ui.add_space(7.0);
                    ui.label(egui::RichText::new(&issue.code).strong().size(11.0));
                    ui.label(egui::RichText::new(&issue.message).size(12.0).color(MUTED));
                }
            }
            section_label(ui, "材料カタログ");
            if ui.button("AGFフォルダーを追加…").clicked() {
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

    fn power_budget(&self, ui: &mut egui::Ui) {
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
                r.scene.sources.iter().map(|s| s.count).sum::<usize>(),
                r.seconds * 1000.0,
                r.scene.surfaces.len()
            ))
            .size(11.0)
            .color(MUTED),
        );
    }
}

fn optics_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Optics", &["zmx", "ZMX", "json"])
}

fn kind_label(kind: ComponentKind) -> &'static str {
    if kind == ComponentKind::Detector {
        "スクリーン"
    } else {
        kind.label()
    }
}

fn short_name(name: &str, count: usize) -> String {
    if name.chars().count() > count {
        format!("{}…", name.chars().take(count).collect::<String>())
    } else {
        name.into()
    }
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(label).strong().size(12.0).color(INK));
}

fn number_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    unit: &str,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [60.0, 20.0],
            egui::Label::new(egui::RichText::new(label).size(12.0)),
        );
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .clamp_existing_to_range(false)
                .speed(speed)
                .max_decimals(5)
                .suffix(unit),
        )
        .changed()
    })
    .inner
}

fn pose_controls(ui: &mut egui::Ui, pose: &mut Pose) -> bool {
    let mut changed = false;
    ui.label(egui::RichText::new("位置").strong().size(12.0));
    for a in 0..3 {
        changed |= number_row(
            ui,
            ["X", "Y", "Z"][a],
            &mut pose.translation_mm.0[a],
            " mm",
            0.1,
            -1e12..=1e12,
        );
    }
    egui::CollapsingHeader::new("傾き").show(ui, |ui| {
        for a in 0..3 {
            changed |= number_row(
                ui,
                ["θx", "θy", "θz"][a],
                &mut pose.rotation_deg.0[a],
                " °",
                0.1,
                -36000.0..=36000.0,
            );
        }
    });
    changed
}

fn index_label(ui: &mut egui::Ui, material: &Material, wavelength: f64) {
    if let Some(n) = material.index(wavelength) {
        ui.label(
            egui::RichText::new(format!("n({:.1} nm) = {:.7}", wavelength * 1000.0, n))
                .size(11.0)
                .color(MUTED),
        );
    }
}

fn detector_controls(ui: &mut egui::Ui, surface: &mut Surface) -> bool {
    let mut changed = false;
    if let Aperture::Rectangle {
        half_width_mm,
        half_height_mm,
    } = &mut surface.aperture
    {
        for (label, half) in [("幅 X", half_width_mm), ("幅 Y", half_height_mm)] {
            let mut full = *half * 2.0;
            if number_row(ui, label, &mut full, " mm", 0.5, 0.0001..=1e9) {
                *half = full / 2.0;
                changed = true;
            }
        }
    }
    if let Interaction::Detector { pixels } = &mut surface.interaction {
        for (axis, value) in pixels.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 20.0], egui::Label::new(["画素 X", "画素 Y"][axis]));
                changed |= ui
                    .add(
                        egui::DragValue::new(value)
                            .range(1..=1024)
                            .clamp_existing_to_range(false),
                    )
                    .changed();
            });
        }
    }
    changed
}

fn sampling_control(ui: &mut egui::Ui, sampling: &mut Sampling) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("sampling")
        .selected_text(match sampling {
            Sampling::Sobol => "Sobol（低偏差列）",
            Sampling::Pseudorandom => "疑似乱数",
        })
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(sampling, Sampling::Sobol, "Sobol（低偏差列）")
                .changed();
            changed |= ui
                .selectable_value(sampling, Sampling::Pseudorandom, "疑似乱数")
                .changed();
        });
    changed
}

fn source_controls(
    ui: &mut egui::Ui,
    sources: &mut [Source],
    index: &mut usize,
    local: bool,
    mono: Option<f64>,
) -> bool {
    if sources.is_empty() {
        return false;
    }
    *index = (*index).min(sources.len() - 1);
    if sources.len() > 1 {
        egui::ComboBox::from_id_salt("source")
            .selected_text(format!("光源 {} / {}", *index + 1, sources.len()))
            .show_ui(ui, |ui| {
                for (i, s) in sources.iter().enumerate() {
                    ui.selectable_value(
                        index,
                        i,
                        format!("光源 {} · {:.0} nm", i + 1, s.wavelength_um * 1000.0),
                    );
                }
            });
    }
    let mut changed = false;
    let mut apply = false;
    let s = &mut sources[*index];
    ui.horizontal(|ui| {
        ui.add_sized([60.0, 20.0], egui::Label::new("光線数"));
        changed |= ui
            .add(
                egui::DragValue::new(&mut s.count)
                    .range(1..=2_000_000)
                    .clamp_existing_to_range(false)
                    .speed(256),
            )
            .changed();
    });
    let mut nm = s.wavelength_um * 1000.0;
    if number_row(ui, "波長", &mut nm, " nm", 1.0, 100.0..=20000.0) {
        s.wavelength_um = nm / 1000.0;
        changed = true;
    }
    if let Some(value) = mono {
        ui.label(
            egui::RichText::new(format!(
                "単色指定中：計算には {:.1} nm を使用",
                value * 1000.0
            ))
            .size(11.0)
            .color(MUTED),
        );
    }
    egui::CollapsingHeader::new("分布・サンプリング").show(ui, |ui| {
        changed |= sampling_control(ui, &mut s.sampling);
        if s.sampling == Sampling::Sobol && !s.count.is_power_of_two() {
            ui.label(
                egui::RichText::new("Sobol推奨：256 / 1024 / 4096…本")
                    .size(11.0)
                    .color(MUTED),
            );
        }
        ui.horizontal(|ui| {
            ui.label("seed");
            changed |= ui.add(egui::DragValue::new(&mut s.seed)).changed();
        });
        changed |= number_row(ui, "光量重み", &mut s.total_weight, "", 0.01, 0.0..=1e12);
        if let Some(d) = s.diode.as_mut() {
            for a in 0..2 {
                changed |= number_row(
                    ui,
                    ["X 発散", "Y 発散"][a],
                    &mut d.divergence_deg[a],
                    " °",
                    0.1,
                    0.0..=179.0,
                );
            }
        } else {
            changed |= number_row(ui, "円板半径", &mut s.disk_radius_mm, " mm", 0.1, 0.0..=1e9);
            changed |= number_row(
                ui,
                "円錐半角",
                &mut s.cone_half_angle_deg,
                " °",
                0.1,
                0.0..=180.0,
            );
        }
        egui::CollapsingHeader::new(if local {
            "部品内の発光位置"
        } else {
            "発光位置"
        })
        .show(ui, |ui| {
            for a in 0..3 {
                changed |= number_row(
                    ui,
                    ["X", "Y", "Z"][a],
                    &mut s.origin_mm.0[a],
                    " mm",
                    0.1,
                    -1e12..=1e12,
                );
            }
        });
        if ui
            .small_button(if local {
                "本数・方式を部品の全光源へ"
            } else {
                "本数・方式をモデルの全光源へ"
            })
            .clicked()
        {
            apply = true;
        }
    });
    if apply {
        let count = s.count;
        let sampling = s.sampling;
        for source in sources {
            source.count = count;
            source.sampling = sampling;
        }
        changed = true;
    }
    changed
}

fn edit_sources(project: &mut Project, edit: impl Fn(&mut Source)) {
    if let Some(a) = &mut project.assembly {
        for c in &mut a.components {
            for s in &mut c.project.scene.sources {
                edit(s);
            }
        }
    } else {
        for s in &mut project.scene.sources {
            edit(s);
        }
    }
}

fn visible_paths(
    result: Option<&ResultView>,
    stale: bool,
    show_rays: bool,
) -> &[crate::trace::PathEvent] {
    if stale || !show_rays {
        &[]
    } else {
        result
            .map(|r| r.report.paths.as_slice())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
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

    fn text_rect(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == text => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
            }
            _ => None,
        })
    }

    fn click(app: &mut App, ctx: &egui::Context, p: Pos2) -> egui::FullOutput {
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

    #[test]
    fn workspace_switches_and_part_selection_preserve_the_optical_system() {
        let ctx = egui::Context::default();
        let mut app = App::demo();
        let a = app.project.assembly.as_mut().unwrap();
        a.components[0].project.scene.sources[0].count = 3_000_000;
        if let Interaction::Detector { pixels } =
            &mut a.components[3].project.scene.surfaces[0].interaction
        {
            *pixels = [4096, 2048];
        }
        app.project.refresh_assembly().unwrap();
        let before = serde_json::to_value(&app.project).unwrap();
        frame(&mut app, &ctx, vec![]);
        let layout = frame(&mut app, &ctx, vec![]);
        assert!(text_rect(&layout, "寸法").is_some());
        assert!(text_rect(&layout, "追跡方式").is_none());
        let screen_tab = text_rect(&layout, "スクリーン").unwrap().center();
        let screen = click(&mut app, &ctx, screen_tab);
        assert_eq!(app.view, View::Screen);
        assert!(text_rect(&screen, "寸法").is_none());
        assert!(text_rect(&screen, "計算するとスクリーンの分布が表示されます").is_some());
        let layout_tab = text_rect(&screen, "光学系").unwrap().center();
        let layout = click(&mut app, &ctx, layout_tab);
        let source = text_rect(&layout, "01  円板光源").unwrap().center();
        let selected = click(&mut app, &ctx, source);
        assert_eq!(app.component, 0);
        assert!(text_rect(&selected, "発光条件").is_some());
        let detector = text_rect(&selected, "04  スクリーン").unwrap().center();
        let detector = click(&mut app, &ctx, detector);
        assert!(text_rect(&detector, "観測面").is_some());
        assert_eq!(before, serde_json::to_value(&app.project).unwrap());
        assert!(!app.dirty);
    }

    #[test]
    fn global_source_controls_keep_component_local_coordinates_and_disabled_sources() {
        let mut project = assembly::demo_project().unwrap();
        let a = project.assembly.as_mut().unwrap();
        let mut disabled = a.components[0].clone();
        disabled.enabled = false;
        a.components.push(disabled);
        a.components[0].pose.translation_mm = V3([2.0, -3.0, 8.0]);
        a.components[0].pose.rotation_deg = V3([7.0, 19.0, 31.0]);
        project.refresh_assembly().unwrap();
        let before = project.scene.sources.clone();
        edit_sources(&mut project, |s| {
            s.count = 1024;
            s.sampling = Sampling::Pseudorandom;
        });
        project.refresh_assembly().unwrap();
        for (old, current) in before.iter().zip(&project.scene.sources) {
            assert_eq!(current.count, 1024);
            assert_eq!(current.sampling, Sampling::Pseudorandom);
            assert_eq!(current.origin_mm, old.origin_mm);
            assert_eq!(current.direction, old.direction);
            assert_eq!(current.medium, old.medium);
            assert_eq!(current.wavelength_um, old.wavelength_um);
        }
        assert!(project
            .assembly
            .as_ref()
            .unwrap()
            .components
            .last()
            .unwrap()
            .project
            .scene
            .sources
            .iter()
            .all(|s| s.count == 1024));
    }
}
