use super::*;

impl App {
    pub(super) fn calculation_controls(&mut self, ui: &mut egui::Ui) {
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
                .size(SIZE_BODY_LARGE)
                .color(TEXT_MUTED),
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
                        .size(SIZE_BODY_LARGE)
                        .color(TEXT_MUTED),
                    );
                }
            } else {
                ui.label(
                    egui::RichText::new("光源を追加すると本数を設定できます。").color(TEXT_MUTED),
                );
            }
            section_label(ui, "追跡方式");
            if self.project.assembly.is_some() {
                ui.label("非シーケンシャル");
                ui.label(
                    egui::RichText::new("組立光学系は、光線が次に当たる部品を探索して追跡します。")
                        .size(SIZE_BODY_LARGE)
                        .color(TEXT_MUTED),
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
}

pub(super) fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.add_space(10.0);
    ui.label(heading(label));
}

pub(super) fn number_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    unit: &str,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [88.0, 32.0],
            egui::Label::new(egui::RichText::new(label).size(SIZE_CAPTION)),
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

pub(super) fn source_controls(
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
        ui.add_sized([88.0, 32.0], egui::Label::new("光線数"));
        changed |= ui
            .add(
                egui::DragValue::new(&mut s.count)
                    .range(1..=2_000_000)
                    .clamp_existing_to_range(false)
                    .speed(256),
            )
            .changed();
    });
    if let Some(sp) = &s.spectrum {
        if let Some((first, last)) = sp.samples.first().zip(sp.samples.last()) {
            ui.label(format!(
                "スペクトル：{}波長 · {:.1}〜{:.1} nm",
                sp.samples.len(),
                first[0] * 1000.,
                last[0] * 1000.
            ));
        }
        egui::CollapsingHeader::new("波長と光量重み").show(ui, |ui| {
            for &[w, weight] in &sp.samples {
                ui.label(format!("{:.3} nm · 相対重み {:.6}", w * 1000., weight));
            }
        });
        if ui.small_button("単色に切り替える").clicked() {
            s.spectrum = None;
            changed = true;
        }
    } else {
        let mut nm = s.wavelength_um * 1000.;
        if number_row(ui, "波長", &mut nm, " nm", 1., 100.0..=20000.0) {
            s.wavelength_um = nm / 1000.;
            changed = true;
        }
    }
    if let Some(value) = mono {
        ui.label(
            egui::RichText::new(format!(
                "単色指定中：計算には {:.1} nm を使用",
                value * 1000.0
            ))
            .size(SIZE_CAPTION)
            .color(TEXT_MUTED),
        );
    }
    egui::CollapsingHeader::new("分布・サンプリング").show(ui, |ui| {
        changed |= sampling_control(ui, &mut s.sampling);
        if s.sampling == Sampling::Sobol && !s.count.is_power_of_two() {
            ui.label(
                egui::RichText::new("Sobol推奨：256 / 1024 / 4096…本")
                    .size(SIZE_CAPTION)
                    .color(TEXT_MUTED),
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
                    ["X 発散 α", "Y 発散 α"][a],
                    &mut d.divergence_deg[a],
                    " °",
                    0.1,
                    0.0..=1e9,
                );
            }
            egui::CollapsingHeader::new("分布と非点収差").show(ui, |ui| {
                ui.label("発散 α は強度が1/e²になる半角です");
                ui.label("形の係数 G/H は1で通常のGaussianになります");
                changed |= number_row(
                    ui,
                    "非点収差",
                    &mut d.astigmatism_mm,
                    " mm",
                    0.01,
                    0.0..=1e9,
                );
                for a in 0..2 {
                    changed |= number_row(
                        ui,
                        ["X 角度 G", "Y 角度 G"][a],
                        &mut d.angular_supergaussian[a],
                        "",
                        0.01,
                        0.01..=50.,
                    );
                    if d.astigmatism_mm == 0. {
                        changed |= number_row(
                            ui,
                            ["X 発光半幅", "Y 発光半幅"][a],
                            &mut d.half_width_mm[a],
                            " mm",
                            0.001,
                            0.0..=1e9,
                        );
                        changed |= number_row(
                            ui,
                            ["X 幅 s", "Y 幅 s"][a],
                            &mut d.gaussian_width_mm[a],
                            " mm",
                            0.001,
                            0.0..=1e9,
                        );
                        changed |= number_row(
                            ui,
                            ["X 形 H", "Y 形 H"][a],
                            &mut d.spatial_supergaussian[a],
                            "",
                            0.01,
                            0.01..=f64::MAX,
                        );
                    }
                }
                if d.astigmatism_mm > 0. {
                    ui.label("Xの仮想発光点を後方へ移し、Z=0の線上から発射します");
                }
            });
        } else if let Some(p) = &mut s.pupil {
            ui.label(if p.finite_object {
                "有限距離の物点 → 瞳の面積一様照射"
            } else {
                "無限遠の視野 → 瞳の面積一様照射"
            });
            if let Some(i) = p.surface {
                ui.label(if p.optical.is_some() {
                    format!("STOP：面 {}", i + 1)
                } else {
                    format!("瞳の配置は面 {} に追従", i + 1)
                });
            }
            egui::CollapsingHeader::new("視野と瞳").show(ui, |ui| {
                if let Some(o) = &mut p.optical {
                    use raytrace_core::illumination::RayAiming;
                    ui.label("入射瞳は光学面と主波長から計算します");
                    ui.label(format!("主波長：{:.1} nm", o.primary_wavelength_um * 1000.));
                    let label = |m| match m {
                        RayAiming::Off => "近軸瞳へ照射",
                        RayAiming::Paraxial => "絞りへ照準（近軸半径）",
                        RayAiming::Real => "絞りへ照準（実光線半径）",
                    };
                    egui::ComboBox::from_id_salt("ray_aiming")
                        .selected_text(label(o.aiming))
                        .show_ui(ui, |ui| {
                            for mode in [RayAiming::Off, RayAiming::Paraxial, RayAiming::Real] {
                                changed |= ui
                                    .selectable_value(&mut o.aiming, mode, label(mode))
                                    .changed();
                            }
                        });
                    if let Some(angles) = &mut o.finite_field_angle_deg {
                        for a in 0..2 {
                            changed |= number_row(
                                ui,
                                ["X 視野角", "Y 視野角"][a],
                                &mut angles[a],
                                " °",
                                0.1,
                                -89.9..=89.9,
                            );
                        }
                        ui.label("物点のXY位置は視野角と入射瞳から決まります");
                    }
                    if let Aperture::Circle { radius_mm } = &mut p.aperture {
                        changed |= number_row(ui, "入射瞳半径", radius_mm, " mm", 0.1, 1e-9..=1e9);
                    }
                }
                for a in 0..2 {
                    changed |= number_row(
                        ui,
                        ["瞳のずれ X", "瞳のずれ Y"][a],
                        &mut p.vignetting.decenter[a],
                        "",
                        0.01,
                        -f64::MAX..=f64::MAX,
                    );
                    changed |= number_row(
                        ui,
                        ["瞳の圧縮 X", "瞳の圧縮 Y"][a],
                        &mut p.vignetting.compression[a],
                        "",
                        0.01,
                        -f64::MAX..=1.,
                    );
                }
                changed |= number_row(
                    ui,
                    "瞳の回転",
                    &mut p.vignetting.tangential_angle_deg,
                    " °",
                    0.1,
                    -360.0..=360.0,
                );
                ui.label("瞳補正後に発射する光線の重み合計を100%とします");
            });
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
                if a < 2
                    && s.pupil
                        .as_ref()
                        .and_then(|p| p.optical.as_ref())
                        .is_some_and(|o| o.finite_field_angle_deg.is_some())
                {
                    continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_source_controls_keep_component_local_coordinates_and_disabled_sources() {
        let mut project = assembly::demo_project().unwrap();
        let a = project.assembly.as_mut().unwrap();
        let mut disabled = a.components[0].clone();
        disabled.id = Default::default();
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
        assert!(
            project
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
                .all(|s| s.count == 1024)
        );
    }
}
