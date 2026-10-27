use super::*;

impl App {
    pub(in crate::gui) fn study_window(&mut self, ctx: &egui::Context) {
        if !self.study.open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut resume = false;
        let mut load = false;
        let mut selected_case = None;
        let mut compare = false;
        egui::Window::new("パラメーター走査")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 70.0).max(300.0))
            .open(&mut open)
            .default_width(860.0)
            .default_height(620.0)
            .min_width(540.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.job.is_none(), egui::Button::new("走査記録を開く…"))
                        .clicked()
                    {
                        load = true;
                    }
                    if self.job.is_some() {
                        ui.spinner();
                        if let Some(c) = &self.control
                            && ui.button("中止").clicked()
                        {
                            c.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                });
                let has_result = self.study.manifest.is_some();
                egui::CollapsingHeader::new("走査の設定")
                    .default_open(!has_result)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            let old = self.study.parameter.clone();
                            let options = parameters(&self.project);
                            egui::ComboBox::from_id_salt("scan_parameter")
                                .width(410.0)
                                .selected_text(
                                    old.as_ref().map_or("対象を選択".into(), |p| {
                                        p.label(&self.project)
                                    }),
                                )
                                .show_ui(ui, |ui| {
                                    for p in options {
                                        let label = p.label(&self.project);
                                        ui.selectable_value(
                                            &mut self.study.parameter,
                                            Some(p),
                                            label,
                                        );
                                    }
                                });

                            if old != self.study.parameter
                                && let Some(p) = self.study.parameter.clone()
                            {
                                self.set_study_parameter(p);
                            }
                            let suffix = self.study.parameter.as_ref().map_or("", unit);
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.study.list_mode, false, "範囲と点数");
                                ui.radio_value(&mut self.study.list_mode, true, "値の一覧");
                                ui.label(suffix);
                            });
                            if self.study.list_mode {
                                ui.text_edit_singleline(&mut self.study.values);
                            } else {
                                ui.horizontal(|ui| {
                                    ui.label("開始");
                                    ui.add(egui::DragValue::new(&mut self.study.start).speed(0.1));
                                    ui.label("終了");
                                    ui.add(egui::DragValue::new(&mut self.study.end).speed(0.1));
                                    ui.label("点数");
                                    ui.add(
                                        egui::DragValue::new(&mut self.study.count).range(2..=256),
                                    );
                                });
                            }
                            egui::CollapsingHeader::new("2軸の走査・値の連動").show(ui, |ui| {
                                let options = parameters(&self.project);
                                let mut enabled = self.study.secondary.is_some();
                                if checkbox(ui, &mut enabled, "第2軸を追加").changed() {
                                    self.study.secondary = if enabled {
                                        options
                                            .iter()
                                            .find(|p| Some(*p) != self.study.parameter.as_ref())
                                            .cloned()
                                    } else {
                                        None
                                    };
                                    if let Some(p) = &self.study.secondary {
                                        self.study.secondary_values = format!(
                                            "{:.6}",
                                            p.value(&self.project).unwrap_or(0.) * scale(p)
                                        );
                                    }
                                }
                                if let Some(p) = &mut self.study.secondary {
                                    let previous = p.clone();
                                    parameter_choice(
                                        ui,
                                        "secondary_axis",
                                        p,
                                        &options,
                                        &self.project,
                                    );
                                    if *p != previous {
                                        self.study.secondary_values = format!(
                                            "{:.6}",
                                            p.value(&self.project).unwrap_or(0.) * scale(p)
                                        );
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(format!("値一覧 ({})", unit(p)));
                                        ui.text_edit_singleline(&mut self.study.secondary_values);
                                    });
                                    ui.label("2軸の全組合せを計算します（合計256ケースまで）。");
                                }
                                let mut remove = None;
                                for (i, l) in self.study.links.iter_mut().enumerate() {
                                    ui.push_id(i, |ui| {
                                        ui.separator();
                                        ui.label("連動元");
                                        parameter_choice(
                                            ui,
                                            "link_source",
                                            &mut l.source,
                                            &options,
                                            &self.project,
                                        );
                                        ui.label("連動先");
                                        parameter_choice(
                                            ui,
                                            "link_target",
                                            &mut l.target,
                                            &options,
                                            &self.project,
                                        );
                                        ui.horizontal(|ui| {
                                            ui.label("先 = 元 ×");
                                            ui.add(egui::DragValue::new(&mut l.scale).speed(0.01));
                                            ui.label("+");
                                            ui.add(egui::DragValue::new(&mut l.offset).speed(0.1));
                                            ui.label(l.target.unit());
                                            if ui.small_button("削除").clicked() {
                                                remove = Some(i);
                                            }
                                        });
                                    });
                                }
                                if let Some(i) = remove {
                                    self.study.links.remove(i);
                                }
                                if ui.small_button("距離・差分を保つ連動を追加").clicked()
                                    && let Some(source) = self.study.parameter.clone()
                                    && let Some(target) = options
                                        .iter()
                                        .find(|p| {
                                            **p != source
                                                && p.unit() == source.unit()
                                                && Some(*p) != self.study.secondary.as_ref()
                                                && !self.study.links.iter().any(|l| l.target == **p)
                                        })
                                        .cloned()
                                {
                                    let offset = target.value(&self.project).unwrap_or(0.)
                                        - source.value(&self.project).unwrap_or(0.);
                                    self.study.links.push(ParameterLink {
                                        source,
                                        target,
                                        scale: 1.,
                                        offset,
                                    });
                                }
                            });
                            checkbox(ui, &mut self.study.use_current, "現在の光源本数を使用");
                            if !self.study.use_current {
                                ui.horizontal(|ui| {
                                    ui.label("各光源の本数");
                                    ui.add(
                                        egui::DragValue::new(&mut self.study.rays)
                                            .range(1..=2_000_000)
                                            .speed(64),
                                    );
                                });
                            }
                            ui.label(
                                egui::RichText::new(
                                    "各ケースで光源の方式・seedを揃えて比較します。",
                                )
                                .size(SIZE_CAPTION)
                                .color(TEXT_MUTED),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("保存先…").clicked()
                                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                                {
                                    self.study.root = dir;
                                }
                                ui.label(self.study.root.display().to_string());
                            });
                            if ui.button("走査を開始").clicked() {
                                start = true;
                            }
                        });
                    });
                if let Some(m) = &self.study.manifest {
                    ui.separator();
                    ui.label(format!(
                        "{} · {} ケース",
                        m.plan.parameter.label(&m.base),
                        m.cases.len()
                    ));
                    if let Some(s) = &m.plan.secondary {
                        ui.label(format!("第2軸：{}", s.parameter.label(&m.base)));
                    }
                    let done = m
                        .cases
                        .iter()
                        .filter(|c| c.state == "completed" || c.state == "failed")
                        .count();
                    ui.add(
                        egui::ProgressBar::new(done as f32 / m.cases.len().max(1) as f32)
                            .text(format!("{done} / {} ケース完了", m.cases.len())),
                    );
                    if self.job.is_none()
                        && m.cases
                            .iter()
                            .any(|c| c.state == "pending" || c.state == "running")
                        && ui.button("未完了のケースを再開").clicked()
                    {
                        resume = true;
                    }
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.study.metric, 0, "受光量 [%]");
                        ui.selectable_value(&mut self.study.metric, 1, "FWHM X");
                        ui.selectable_value(&mut self.study.metric, 2, "FWHM Y");
                    });
                    if let Some(case) = m.cases.iter().find(|c| !c.detectors.is_empty()) {
                        egui::ComboBox::from_id_salt("scan_detector")
                            .selected_text(
                                case.detectors
                                    .get(self.study.detector)
                                    .map_or("スクリーン", |d| d.name.as_str()),
                            )
                            .show_ui(ui, |ui| {
                                for (i, d) in case.detectors.iter().enumerate() {
                                    ui.selectable_value(&mut self.study.detector, i, &d.name);
                                }
                            });
                    }
                    if let Some(index) = study_plot(ui, m, self.study.detector, self.study.metric) {
                        selected_case = Some(index);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("scan_cases")
                        .max_height(225.0)
                        .show(ui, |ui| {
                            egui::Grid::new("scan_table").striped(true).show(ui, |ui| {
                                for text in ["値", "受光量 [%]", "FWHM X [mm]", "FWHM Y [mm]", ""]
                                {
                                    ui.label(text);
                                }
                                ui.end_row();
                                for (i, c) in m.cases.iter().enumerate() {
                                    ui.label(case_label(c, &m.plan));
                                    let d = c.detectors.get(self.study.detector);
                                    ui.label(d.map_or("—".into(), |d| {
                                        format!("{:.5}", d.received_percent)
                                    }));
                                    for a in 0..2 {
                                        ui.label(
                                            d.and_then(|d| d.fwhm_mm[a])
                                                .map_or("—".into(), |v| format!("{v:.5}")),
                                        );
                                    }
                                    if c.state == "completed" {
                                        if ui
                                            .add_enabled(
                                                self.job.is_none(),
                                                egui::Button::new("表示"),
                                            )
                                            .clicked()
                                        {
                                            selected_case = Some(i);
                                        }
                                    } else {
                                        ui.label(match c.state.as_str() {
                                            "running" => "計算中",
                                            "failed" => "失敗",
                                            _ => "未計算",
                                        })
                                        .on_hover_text(c.error.as_deref().unwrap_or(""));
                                    }
                                    ui.end_row();
                                }
                            });
                        });
                    if let Some(dir) = &self.study.directory {
                        ui.label(
                            egui::RichText::new(dir.display().to_string())
                                .size(SIZE_CAPTION)
                                .color(TEXT_MUTED),
                        );
                    }
                }
                if let Some(m) = &self.study.manifest {
                    ui.horizontal(|ui| {
                        for axis in 0..2 {
                            egui::ComboBox::from_id_salt(("comparison_case", axis))
                                .width(135.0)
                                .selected_text(format!(
                                    "{}: ケース {}",
                                    if axis == 0 { "A" } else { "B" },
                                    self.study.pair[axis] + 1
                                ))
                                .show_ui(ui, |ui| {
                                    for (i, c) in m
                                        .cases
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, c)| c.state == "completed")
                                    {
                                        ui.selectable_value(
                                            &mut self.study.pair[axis],
                                            i,
                                            format!("{}: {}", i + 1, case_label(c, &m.plan)),
                                        );
                                    }
                                });
                            if let Some(c) = m.cases.get(self.study.pair[axis]) {
                                egui::ComboBox::from_id_salt(("compare_screen", axis))
                                    .width(140.)
                                    .selected_text(
                                        c.detectors
                                            .get(self.study.comparison_detectors[axis])
                                            .map_or("スクリーン", |d| d.name.as_str()),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, d) in c.detectors.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut self.study.comparison_detectors[axis],
                                                i,
                                                &d.name,
                                            );
                                        }
                                    });
                            }
                        }
                        if ui
                            .add_enabled(self.job.is_none(), egui::Button::new("2ケースを比較"))
                            .clicked()
                        {
                            compare = true;
                        }
                    });
                }
            });
        self.study.open = open;
        if start {
            self.launch_study(ctx, false);
        } else if resume {
            self.launch_study(ctx, true);
        } else if load {
            self.open_study(ctx);
        } else if compare {
            self.load_comparison(ctx);
        } else if let Some(i) = selected_case.filter(|_| self.job.is_none())
            && let Some(path) = self
                .study
                .manifest
                .as_ref()
                .and_then(|m| m.cases.get(i))
                .and_then(|c| c.run_file.as_ref())
                .zip(self.study.directory.as_ref())
                .map(|(file, dir)| dir.join(file))
        {
            self.load(path, ctx);
            self.study.open = false;
        }
    }
}

fn study_plot(
    ui: &mut egui::Ui,
    m: &ScanManifest,
    detector: usize,
    metric: usize,
) -> Option<usize> {
    let points: Vec<_> = m
        .cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let d = c.detectors.get(detector)?;
            let y = if metric == 0 {
                Some(d.received_percent)
            } else {
                d.fwhm_mm[metric - 1]
            }?;
            Some((i, c.value * scale(&m.plan.parameter), y))
        })
        .collect();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 160.0), egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, BG_CANVAS);
    if points.is_empty() {
        return None;
    }
    let minx = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let maxx = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let maxy = points.iter().map(|p| p.2).fold(0.0, f64::max).max(1e-12);
    let area = rect.shrink2(Vec2::new(64.0, 28.0));
    let pos = |x: f64, y: f64| {
        Pos2::new(
            area.left() + ((x - minx) / (maxx - minx).max(1e-12)) as f32 * area.width(),
            area.bottom() - (y / maxy) as f32 * area.height(),
        )
    };
    let series = |i: usize| {
        m.cases[i]
            .secondary_value
            .and_then(|v| {
                m.plan
                    .secondary
                    .as_ref()
                    .and_then(|s| s.values.iter().position(|x| *x == v))
            })
            .unwrap_or(0)
    };
    let palette = viz::SERIES;
    for k in 0..m.plan.secondary.as_ref().map_or(1, |s| s.values.len()) {
        let mut line = points
            .iter()
            .filter(|p| series(p.0) == k)
            .map(|p| pos(p.1, p.2))
            .collect::<Vec<_>>();
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
        if line.len() > 1 {
            painter.add(egui::Shape::line(
                line,
                Stroke::new(1.0_f32, palette[k % palette.len()]),
            ));
        }
    }
    for &(i, x, y) in &points {
        let p = pos(x, y);
        painter.circle_filled(p, 3.5, palette[series(i) % palette.len()]);
        if response.clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|pointer| pointer.distance(p) < 14.0)
        {
            return Some(i);
        }
    }
    painter.text(
        rect.left_top() + Vec2::new(4.0, 4.0),
        egui::Align2::LEFT_TOP,
        format!("{maxy:.4} {}", if metric == 0 { "%" } else { "mm" }),
        egui::FontId::proportional(SIZE_CAPTION),
        TEXT_MUTED,
    );
    if let Some(s) = &m.plan.secondary {
        ui.horizontal_wrapped(|ui| {
            for (i, v) in s.values.iter().enumerate() {
                ui.colored_label(
                    palette[i % palette.len()],
                    format!("● {:.4} {}", v * scale(&s.parameter), unit(&s.parameter)),
                );
            }
        });
    }
    painter.text(
        Pos2::new(area.left(), rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{minx:.4}"),
        egui::FontId::proportional(SIZE_CAPTION),
        TEXT_MUTED,
    );
    painter.text(
        Pos2::new(area.right(), rect.bottom() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{maxx:.4} {}", unit(&m.plan.parameter)),
        egui::FontId::proportional(SIZE_CAPTION),
        TEXT_MUTED,
    );
    None
}
