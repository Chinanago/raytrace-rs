use super::controls::{number_row, section_label, source_controls};
use super::*;
use raytrace_core::math::Pose;

impl App {
    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        if self.view != View::Layout {
            return;
        }
        egui::SidePanel::left("parts")
            .exact_width(340.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(BG_PANEL).inner_margin(16.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("部品")
                            .family(egui::FontFamily::Name("bold".into()))
                            .size(SIZE_HEADING_S),
                    );
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
                    .color(TEXT_MUTED),
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
                        if theme::component_button(ui, text, self.component == i)
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
                    ui.label(
                        egui::RichText::new(kind_label(c.kind))
                            .family(egui::FontFamily::Name("bold".into()))
                            .color(TEXT),
                    );
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
                    .size(SIZE_CAPTION)
                    .color(TEXT_MUTED),
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
                                ui.label(egui::RichText::new(name).size(SIZE_CAPTION));
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
                            egui::Button::new(s.spectrum.as_ref().map_or_else(
                                || format!("光源 {} · {:.0} nm", i + 1, s.wavelength_um * 1000.),
                                |sp| format!("光源 {} · {}波長", i + 1, sp.samples.len()),
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
                ui.label(
                    egui::RichText::new(&s.name).family(egui::FontFamily::Name("bold".into())),
                );
                ui.label(
                    egui::RichText::new(shape_name(&s.shape))
                        .size(SIZE_CAPTION)
                        .color(TEXT_MUTED),
                );
                changed |= pose_controls(ui, &mut s.pose);
                match &mut s.shape {
                    Shape::Sphere { radius_mm } | Shape::EvenAsphere { radius_mm, .. } => {
                        changed |= number_row(ui, "曲率半径", radius_mm, " mm", 0.1, -1e12..=1e12);
                    }
                    Shape::TriangleMesh { triangles, .. } => {
                        ui.label(
                            egui::RichText::new(format!("CAD · {} triangles", triangles.len()))
                                .size(SIZE_CAPTION)
                                .color(TEXT_MUTED),
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
                        ui.label(
                            egui::RichText::new(label)
                                .size(SIZE_CAPTION)
                                .color(TEXT_MUTED),
                        );
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

    pub(super) fn wavelength(&self) -> f64 {
        self.project
            .monochromatic_override_um
            .or_else(|| self.project.scene.sources.first().map(|s| s.wavelength_um))
            .unwrap_or(0.55)
    }
}

fn shape_name(s: &Shape) -> &'static str {
    match s {
        Shape::ConicalSide { .. } => "レンズ側面",
        Shape::Plane => "平面",
        Shape::Sphere { .. } => "球面",
        Shape::EvenAsphere { .. } | Shape::FlatEvenAsphere { .. } => "非球面",
        Shape::TriangleMesh { .. } => "CAD",
    }
}

pub(super) fn kind_label(kind: ComponentKind) -> &'static str {
    if kind == ComponentKind::Detector {
        "スクリーン"
    } else {
        kind.label()
    }
}

pub(super) fn short_name(name: &str, count: usize) -> String {
    if name.chars().count() > count {
        format!("{}…", name.chars().take(count).collect::<String>())
    } else {
        name.into()
    }
}

fn pose_controls(ui: &mut egui::Ui, pose: &mut Pose) -> bool {
    let mut changed = false;
    ui.label(
        egui::RichText::new("位置")
            .family(egui::FontFamily::Name("bold".into()))
            .size(SIZE_CAPTION),
    );
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
                .size(SIZE_CAPTION)
                .color(TEXT_MUTED),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::test_support::*;
    #[test]
    fn workspace_switches_and_part_selection_preserve_the_optical_system() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
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
}
