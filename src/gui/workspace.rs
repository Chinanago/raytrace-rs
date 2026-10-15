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
                        [(ui.available_width() - 420.0).clamp(60.0, 360.0), 24.0],
                        egui::Label::new(egui::RichText::new(title).size(12.0).color(MUTED))
                            .truncate(),
                    )
                    .on_hover_text(self.path.as_ref().map_or_else(
                        || "保存先：未設定".into(),
                        |p| format!("保存先：{}", p.display()),
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.job.is_none() || self.control.is_some(),
                                egui::Button::new(
                                    egui::RichText::new(if self.control.is_some() {
                                        "■  中止"
                                    } else if self.viewing_explicit_record {
                                        "モデル光源で計算"
                                    } else {
                                        "▶  計算する"
                                    })
                                    .color(Color32::WHITE),
                                )
                                .fill(BLUE)
                                .min_size(Vec2::new(118.0, 32.0)),
                            )
                            .clicked()
                        {
                            if let Some(control) = &self.control {
                                control.cancel.store(true, Ordering::Relaxed);
                            } else {
                                if self.viewing_explicit_record {
                                    self.viewing_explicit_record = false;
                                    self.changed();
                                }
                                self.run(ctx);
                            }
                        }
                        if ui.button("計算設定").clicked() {
                            self.details = Some(Details::Calculation);
                        }
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            if ui
                                .add_enabled(self.history.can_redo(), egui::Button::new("↷"))
                                .on_hover_text("やり直す · Ctrl+Shift+Z / Ctrl+Y")
                                .clicked()
                            {
                                self.redo();
                            }
                            if ui
                                .add_enabled(self.history.can_undo(), egui::Button::new("↶"))
                                .on_hover_text("元に戻す · Ctrl+Z")
                                .clicked()
                            {
                                self.undo();
                            }
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
                        if self.study.manifest.is_some() && ui.small_button("走査結果").clicked()
                        {
                            self.study.open = true;
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
        if ui.button("パラメーター走査…").clicked() {
            self.configure_study();
            ui.close_menu();
        }
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
        ui.menu_button("最近開いたファイル", |ui| {
            for path in self.preferences.recent_files.clone() {
                let label = path.file_name().unwrap_or_default().to_string_lossy();
                if ui
                    .button(label)
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    self.load(path, ctx);
                    ui.close_menu();
                }
            }
        });
        if ui.button("保存…").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("optics.raytrace.json")
                .add_filter("Raytrace", &["json"])
                .save_file()
            {
                match self.project.save(&path) {
                    Ok(()) => {
                        self.mark_saved();
                        self.path = Some(path);
                        self.remember_path();
                        self.status = "プロジェクトを保存しました".into();
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if ui
            .add_enabled(self.result.is_some(), egui::Button::new("計算結果を保存…"))
            .on_hover_text("計算時の部品・材料・光源条件と画素データを保存します")
            .clicked()
        {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("calculation.raytrace-run.json")
                .add_filter("Raytrace calculation", &["json"])
                .save_file()
            {
                let r = self.result.as_ref().unwrap();
                let mut report = r.report.clone();
                // Viewer paths use reindexed sample IDs; archive the actual calculation.
                report.paths.clear();
                match r.calculation.save(&report, r.seconds, &path) {
                    Ok(()) => self.status = "計算時の条件と結果を保存しました".into(),
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if ui
            .add_enabled(
                self.result
                    .as_ref()
                    .is_some_and(|r| !r.report.detectors.is_empty()),
                egui::Button::new("スクリーンを書き出す…"),
            )
            .clicked()
        {
            ui.close_menu();
            if let Some(parent) = rfd::FileDialog::new()
                .set_title("スクリーンの出力先")
                .pick_folder()
            {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                self.export_current_screen(&parent.join(format!("screen_{stamp}")), ctx);
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
        let display = crate::export::DisplaySettings {
            peak_normalized: self.screen_display.normalization == ScreenNormalization::Peak,
            square_root: self.screen_display.scale == ScreenScale::SquareRoot,
            maximum_percent: self.screen_display.maximum_percent,
        };
        let conditions = serde_json::json!({"calculation_software_version":r.calculation.software_version,"model":r.scene.name,"wavelengths_um":r.calculation.launch_wavelengths_um,
            "input_rays":r.calculation.input_count(),"sources":r.scene.sources,"trace_settings":r.scene.settings,"display":display});
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.status = "スクリーンを書き出し中…".into();
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                crate::export::write_screen(&dir, &map, &plane, reference, display)?;
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

    fn replace_project(&mut self, project: Project, preview: Option<usize>, component: usize) {
        self.project = project;
        self.viewing_explicit_record = false;
        self.path = None;
        self.loading_path = None;
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
        self.camera = LayoutCamera::default();
        self.reset_history(true);
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
        let mut duplicate = false;
        let mut study = false;
        egui::ScrollArea::vertical()
            .id_salt("part_inspector")
            .show(ui, |ui| {
                let c = &mut assembly.components[self.component];
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(kind_label(c.kind)).strong().color(INK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("操作", |ui| {
                            if ui.button("この部品の値を走査…").clicked() {
                                study = true;
                                ui.close_menu();
                            }
                            if ui.button("部品を複製").clicked() {
                                duplicate = true;
                                ui.close_menu();
                            }
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
        } else if duplicate {
            let mut copy = assembly.components[self.component].clone();
            copy.name = format!("{} コピー", copy.name);
            assembly.components.insert(self.component + 1, copy);
            self.component += 1;
            self.source = 0;
            changed = true;
        }
        if changed {
            self.refresh_components();
        }
        if study {
            self.configure_study();
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
            if ui.small_button("選択へ").clicked() {
                self.camera.request = Some(CameraRequest::Selected);
            }
            if ui.small_button("全体").clicked() {
                self.camera.request = Some(CameraRequest::All);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("表示", |ui| {
                    ui.checkbox(&mut self.equal_scale, "縦横を同じ縮尺にする");
                    if ui.button("拡大").clicked() {
                        self.camera.request = Some(CameraRequest::Zoom(0.5));
                        ui.close_menu();
                    }
                    if ui.button("縮小").clicked() {
                        self.camera.request = Some(CameraRequest::Zoom(2.0));
                        ui.close_menu();
                    }
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
            if self.project.assembly.is_some() {
                self.component
            } else {
                self.selected
            },
            self.job.is_none(),
            self.equal_scale,
            &mut self.camera,
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
        self.details = if self.details.is_some() {
            open.then_some(page)
        } else {
            None
        };
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
                self.changed();
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
                let rect = egui::Rect::from_min_size(t.pos, t.galley.size()).intersect(s.clip_rect);
                rect.is_positive().then_some(rect)
            }
            _ => None,
        })
    }

    #[test]
    fn zoom_and_background_pan_change_only_the_view_and_all_restores_auto_fit() {
        let mut app = App::demo();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let original = serde_json::to_value(&app.project).unwrap();
        let p = Pos2::new(850.0, 250.0);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, 100.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let zoomed = serde_json::to_value(&app.camera).unwrap();
        assert!(zoomed["views"][0].is_object());
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p + Vec2::new(50.0, 20.0))],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: p + Vec2::new(50.0, 20.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let panned = serde_json::to_value(&app.camera).unwrap();
        assert_ne!(zoomed["views"][0]["center"], panned["views"][0]["center"]);
        assert_eq!(original, serde_json::to_value(&app.project).unwrap());
        assert!(!app.history.can_undo());
        app.camera.request = Some(CameraRequest::All);
        frame(&mut app, &ctx, vec![]);
        assert!(serde_json::to_value(&app.camera).unwrap()["views"][0].is_null());
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
