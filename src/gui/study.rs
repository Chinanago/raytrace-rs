use super::*;
use crate::{
    record::RunRecord,
    scan::{linear_values, Parameter, ScanCase, ScanManifest, ScanPlan},
};

pub(super) struct Study {
    pub auto_compare: bool,
    pub comparisons: Vec<Comparison>,
    pub comparison_open: bool,
    pub comparison_max: f64,
    pair: [usize; 2],
    pub open: bool,
    pub parameter: Option<Parameter>,
    start: f64,
    end: f64,
    count: usize,
    list_mode: bool,
    values: String,
    use_current: bool,
    rays: usize,
    pub root: PathBuf,
    pub directory: Option<PathBuf>,
    pub manifest: Option<ScanManifest>,
    progress: Option<Receiver<Vec<ScanCase>>>,
    metric: usize,
    detector: usize,
}
impl Default for Study {
    fn default() -> Self {
        Self {
            auto_compare: false,
            comparisons: Vec::new(),
            comparison_open: false,
            comparison_max: 1.0,
            pair: [0, 1],
            open: false,
            parameter: None,
            start: 0.0,
            end: 1.0,
            count: 7,
            list_mode: false,
            values: String::new(),
            use_current: true,
            rays: 256,
            root: PathBuf::from("results/scans"),
            directory: None,
            manifest: None,
            progress: None,
            metric: 0,
            detector: 0,
        }
    }
}

pub(super) struct Comparison {
    label: String,
    reference_plane: String,
    map: crate::trace::DetectorMap,
    pub reference: f64,
    pub analysis: ScreenAnalysis,
    texture: Option<egui::TextureHandle>,
}

fn parameters(project: &Project) -> Vec<Parameter> {
    let mut p = Vec::new();
    if let Some(a) = &project.assembly {
        for (component, c) in a.components.iter().enumerate() {
            for axis in [2, 0, 1] {
                p.push(Parameter::ComponentPosition { component, axis });
            }
            for axis in 0..3 {
                p.push(Parameter::ComponentRotation { component, axis });
            }
            if c.rod.is_some() {
                for axis in [2, 0, 1] {
                    p.push(Parameter::RodSize { component, axis });
                }
            }
        }
    } else {
        for surface in 0..project.scene.surfaces.len() {
            for axis in [2, 0, 1] {
                p.push(Parameter::SurfacePosition { surface, axis });
            }
            for axis in 0..3 {
                p.push(Parameter::SurfaceRotation { surface, axis });
            }
        }
    }
    p.push(Parameter::Wavelength);
    p
}
fn scale(parameter: &Parameter) -> f64 {
    if *parameter == Parameter::Wavelength {
        1000.0
    } else {
        1.0
    }
}
fn unit(parameter: &Parameter) -> &str {
    if *parameter == Parameter::Wavelength {
        "nm"
    } else {
        parameter.unit()
    }
}

impl App {
    pub(super) fn remember_path(&mut self) {
        if let Some(path) = &self.path {
            self.preferences.remember(path);
            let _ = self.preferences.save(&self.data_directory);
        }
    }
    pub(super) fn configure_study(&mut self) {
        let parameter = if self.project.assembly.is_some() {
            Parameter::ComponentPosition {
                component: self.component,
                axis: 2,
            }
        } else {
            Parameter::SurfacePosition {
                surface: self.selected,
                axis: 2,
            }
        };
        self.set_study_parameter(parameter);
        self.study.open = true;
        self.study.use_current = self.preview_limit.is_none();
        self.study.rays = self.preview_limit.unwrap_or(256);
    }
    fn set_study_parameter(&mut self, parameter: Parameter) {
        let value = parameter.value(&self.project).unwrap_or(0.55) * scale(&parameter);
        let spread = if parameter == Parameter::Wavelength {
            20.0
        } else if matches!(parameter, Parameter::RodSize { .. }) {
            value.abs() * 0.1
        } else {
            1.0
        };
        self.study.start = value - spread;
        self.study.end = value + spread;
        self.study.values = format!("{:.6}, {:.6}, {:.6}", value - spread, value, value + spread);
        self.study.parameter = Some(parameter);
    }
    pub(super) fn poll_study(&mut self) {
        if let Some(rx) = &self.study.progress {
            while let Ok(cases) = rx.try_recv() {
                if let Some(m) = &mut self.study.manifest {
                    m.cases = cases;
                }
            }
        }
    }
    fn launch_study(&mut self, ctx: &egui::Context, resume: bool) {
        self.commit_edit();
        let prepared = (|| -> Result<(ScanManifest, PathBuf), String> {
            if resume {
                return Ok((
                    self.study
                        .manifest
                        .clone()
                        .ok_or("走査記録を開いてください")?,
                    self.study.directory.clone().ok_or("保存先がありません")?,
                ));
            }
            let parameter = self
                .study
                .parameter
                .clone()
                .ok_or("走査対象を選んでください")?;
            let values = if self.study.list_mode {
                self.study
                    .values
                    .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                    .filter(|v| !v.is_empty())
                    .map(|v| {
                        v.parse::<f64>()
                            .map_err(|_| "値の一覧を確認してください".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                linear_values(self.study.start, self.study.end, self.study.count)?
            };
            let values = values.into_iter().map(|v| v / scale(&parameter)).collect();
            let plan = ScanPlan {
                parameter,
                values,
                threads: self.threads,
                rays_per_source: (!self.study.use_current).then_some(self.study.rays),
            };
            let manifest = ScanManifest::new(&self.project, plan)?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let dir = self.study.root.join(format!("scan_{stamp}"));
            if dir.exists() {
                return Err("同名の走査記録があります。新しい保存先を指定してください".into());
            }
            Ok((manifest, dir))
        })();
        match prepared {
            Ok((manifest, dir)) => self.spawn_study(manifest, dir, ctx),
            Err(error) => self.error = Some(error),
        }
    }
    fn spawn_study(&mut self, mut manifest: ScanManifest, dir: PathBuf, ctx: &egui::Context) {
        self.study.manifest = Some(manifest.clone());
        self.study.directory = Some(dir.clone());
        let (tx, rx) = mpsc::channel();
        let (ptx, prx) = mpsc::channel();
        self.study.progress = Some(prx);
        self.job = Some(rx);
        let control = Arc::new(TraceControl::default());
        self.control = Some(control.clone());
        let context = ctx.clone();
        self.status = "パラメーターを走査中…".into();
        std::thread::spawn(move || {
            let result = manifest
                .execute(&dir, &control, |m| {
                    let _ = ptx.send(m.cases.clone());
                    context.request_repaint();
                })
                .map(|_| Box::new(manifest));
            let _ = tx.send(Job::Scanned(result));
            context.request_repaint();
        });
    }
    fn open_study(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("走査記録のフォルダー")
            .pick_folder()
        {
            self.open_study_path(dir, ctx);
        }
    }
    pub(super) fn open_study_path(&mut self, dir: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.study.directory = Some(dir.clone());
        self.study.manifest = None;
        let context = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Job::Scanned(ScanManifest::open(&dir).map(Box::new)));
            context.request_repaint();
        });
    }
    pub(super) fn adopt_record(&mut self, record: RunRecord) {
        self.commit_edit();
        self.viewing_explicit_record = matches!(
            record.calculation.input,
            crate::record::LaunchInput::Explicit { .. }
        );
        self.project = record.calculation.project.clone();
        let mut sources = record.calculation.scene.sources.iter();
        let copy = |s: &mut Source, actual: &Source| {
            s.count = actual.count;
            s.seed = actual.seed;
            s.sampling = actual.sampling;
            s.wavelength_um = actual.wavelength_um;
            s.total_weight = actual.total_weight;
        };
        if let Some(a) = &mut self.project.assembly {
            for c in a.components.iter_mut().filter(|c| c.enabled) {
                for s in &mut c.project.scene.sources {
                    if let Some(actual) = sources.next() {
                        copy(s, actual);
                    }
                }
            }
        } else {
            for s in &mut self.project.scene.sources {
                if let Some(actual) = sources.next() {
                    copy(s, actual);
                }
            }
        }
        if let Err(error) = self.project.refresh_assembly() {
            self.error = Some(error);
            return;
        }
        self.preview_limit = None;
        self.component = 0;
        self.selected = 0;
        self.source = 0;
        self.detector = 0;
        self.reset_history(true);
        self.dirty = false;
        self.camera = LayoutCamera::default();
        self.result = Some(ResultView {
            revision: self.history.revision,
            scene: record.calculation.scene.clone(),
            calculation: record.calculation,
            report: record.report,
            seconds: record.trace_seconds,
            texture: None,
            screen: None,
        });
        self.stale = false;
        self.view = View::Screen;
        self.status = "保存された計算結果を表示しています".into();
        self.remember_path();
    }
    pub(super) fn study_window(&mut self, ctx: &egui::Context) {
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
            .max_height((ctx.screen_rect().height() - 70.0).max(300.0))
            .open(&mut open)
            .default_width(740.0)
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
                        if let Some(c) = &self.control {
                            if ui.button("中止").clicked() {
                                c.cancel.store(true, Ordering::Relaxed);
                            }
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
                            if old != self.study.parameter {
                                if let Some(p) = self.study.parameter.clone() {
                                    self.set_study_parameter(p);
                                }
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
                            ui.checkbox(&mut self.study.use_current, "現在の光源本数を使用");
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
                                .size(11.0)
                                .color(MUTED),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("保存先…").clicked() {
                                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                        self.study.root = dir;
                                    }
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
                                    ui.label(format!(
                                        "{:.5} {}",
                                        c.value * scale(&m.plan.parameter),
                                        unit(&m.plan.parameter)
                                    ));
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
                                .size(10.0)
                                .color(MUTED),
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
                                            format!(
                                                "{}: {:.4} {}",
                                                i + 1,
                                                c.value * scale(&m.plan.parameter),
                                                unit(&m.plan.parameter)
                                            ),
                                        );
                                    }
                                });
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
        } else if let Some(i) = selected_case.filter(|_| self.job.is_none()) {
            if let Some(path) = self
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

    pub(super) fn load_comparison(&mut self, ctx: &egui::Context) {
        let request = (|| -> Result<Vec<(PathBuf, String)>, String> {
            let m = self.study.manifest.as_ref().ok_or("走査結果がありません")?;
            let dir = self.study.directory.as_ref().ok_or("保存先がありません")?;
            self.study
                .pair
                .iter()
                .map(|&i| {
                    let c = m.cases.get(i).ok_or("比較するケースを選んでください")?;
                    let file = c
                        .run_file
                        .as_ref()
                        .ok_or("完了したケースを選んでください")?;
                    Ok((
                        dir.join(file),
                        format!(
                            "{:.5} {}",
                            c.value * scale(&m.plan.parameter),
                            unit(&m.plan.parameter)
                        ),
                    ))
                })
                .collect()
        })();
        let request = match request {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let detector = self.study.detector;
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<Comparison>, String> {
                let mut images = Vec::new();
                for (path, label) in request {
                    let r = RunRecord::open(&path)?;
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
                        analysis: ScreenAnalysis::from_detector(&map)?,
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
    pub(super) fn comparison_window(&mut self, ctx: &egui::Context) {
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
    painter.rect_filled(rect, 4.0, Color32::from_rgb(248, 250, 253));
    if points.is_empty() {
        return None;
    }
    let minx = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let maxx = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let maxy = points.iter().map(|p| p.2).fold(0.0, f64::max).max(1e-12);
    let area = rect.shrink2(Vec2::new(45.0, 22.0));
    let pos = |x: f64, y: f64| {
        Pos2::new(
            area.left() + ((x - minx) / (maxx - minx).max(1e-12)) as f32 * area.width(),
            area.bottom() - (y / maxy) as f32 * area.height(),
        )
    };
    for &(i, x, y) in &points {
        let p = pos(x, y);
        painter.circle_filled(p, 3.5, BLUE);
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
        egui::FontId::proportional(10.0),
        MUTED,
    );
    painter.text(
        Pos2::new(area.left(), rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{minx:.4}"),
        egui::FontId::proportional(10.0),
        MUTED,
    );
    painter.text(
        Pos2::new(area.right(), rect.bottom() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{maxx:.4} {}", unit(&m.plan.parameter)),
        egui::FontId::proportional(10.0),
        MUTED,
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(app: &mut App, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.job.is_some() {
            assert!(Instant::now() < deadline, "background study timed out");
            std::thread::sleep(Duration::from_millis(5));
            app.poll(ctx);
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }
    #[test]
    fn gui_checks_the_total_before_generating_rays_and_can_cancel_a_single_trace() {
        let mut app = App::demo();
        let ctx = egui::Context::default();
        for s in &mut app.project.assembly.as_mut().unwrap().components[0]
            .project
            .scene
            .sources
        {
            s.count = 2_000_000;
        }
        app.project.refresh_assembly().unwrap();
        app.run(&ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.job.is_some() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
            app.poll(&ctx);
        }
        assert!(app
            .error
            .as_ref()
            .is_some_and(|e| e.contains("合計1〜2,000,000")));
        app.error = None;
        for s in &mut app.project.assembly.as_mut().unwrap().components[0]
            .project
            .scene
            .sources
        {
            s.count = 32;
        }
        app.project.refresh_assembly().unwrap();
        app.run(&ctx);
        app.control
            .as_ref()
            .unwrap()
            .cancel
            .store(true, Ordering::Relaxed);
        wait(&mut app, &ctx);
        assert!(app.result.is_none());
        assert!(app.status.contains("中止"));
    }
    #[test]
    fn study_can_run_select_a_saved_case_and_compare_screens_without_retracing_the_full_case() {
        let mut app = App::demo();
        let ctx = egui::Context::default();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/study_ui_tests")
            .join(format!("{}", std::process::id()));
        app.data_directory = dir.join("data");
        app.study.root = dir;
        app.configure_study();
        app.study.start = 68.0;
        app.study.end = 69.0;
        app.study.count = 3;
        app.study.use_current = false;
        app.study.rays = 32;
        app.launch_study(&ctx, false);
        assert!(app.job.is_some());
        wait(&mut app, &ctx);
        assert!(app
            .study
            .manifest
            .as_ref()
            .unwrap()
            .cases
            .iter()
            .all(|c| c.state == "completed"));
        let path = app
            .study
            .directory
            .as_ref()
            .unwrap()
            .join("case_0000.raytrace-run.json");
        let stored = RunRecord::open(&path).unwrap();
        let stored_budget = serde_json::to_value(&stored.report.budget).unwrap();
        app.load(path, &ctx);
        wait(&mut app, &ctx);
        let r = app.result.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(&r.report.budget).unwrap(),
            stored_budget
        );
        assert!(!r.report.paths.is_empty());
        assert_eq!(r.calculation.input_count(), 64);
        assert_eq!(
            app.project.assembly.as_ref().unwrap().components[2]
                .pose
                .translation_mm
                .0[2],
            68.0
        );
        assert!(!app.stale);
        assert_eq!(app.view, View::Screen);
        app.study.pair = [0, 2];
        app.load_comparison(&ctx);
        wait(&mut app, &ctx);
        assert_eq!(app.study.comparisons.len(), 2);
        assert!(app.study.comparison_max > 0.0);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(1320.0, 860.0),
                )),
                ..Default::default()
            },
            |ctx| {
                app.study_window(ctx);
                app.comparison_window(ctx);
            },
        );
        assert!(output.shapes.len() > 20);
    }
}
