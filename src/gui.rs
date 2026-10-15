//! Small native editor; expensive imports and tracing run off the UI thread.
use crate::{
    assembly::{self, Component, ComponentKind},
    geometry::sag,
    math::{Frame, V3},
    model::*,
    project::Project,
    record::RunContext,
    screen::{Profile, ScreenAnalysis},
    trace::{trace, trace_controlled, Report, TraceControl, TraceOptions},
    zemax,
};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::{
    path::PathBuf,
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};
const BLUE: Color32 = Color32::from_rgb(40, 90, 180);
const INK: Color32 = Color32::from_rgb(34, 45, 62);
const MUTED: Color32 = Color32::from_rgb(105, 117, 132);
const LINE: Color32 = Color32::from_rgb(221, 226, 234);
const HALF_COLOR: Color32 = Color32::from_rgb(188, 48, 92);

mod camera;
mod history;
mod study;
mod workspace;
use camera::{CameraRequest, LayoutCamera};

#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum View {
    #[default]
    Layout,
    Screen,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Details {
    Calculation,
    Import,
    Budget,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum ScreenNormalization {
    #[default]
    Peak,
    Reference,
}
#[derive(Clone, Copy, Default, PartialEq)]
enum ScreenScale {
    #[default]
    Linear,
    SquareRoot,
}
struct ScreenDisplay {
    cursor_mm: Option<[f64; 2]>,
    roi: Option<crate::screen::ScreenRoi>,
    maximum_percent: Option<f64>,
    normalization: ScreenNormalization,
    scale: ScreenScale,
    half_maximum: bool,
    profiles: bool,
}
impl Default for ScreenDisplay {
    fn default() -> Self {
        Self {
            cursor_mm: None,
            roi: None,
            normalization: ScreenNormalization::Peak,
            maximum_percent: None,
            scale: ScreenScale::Linear,
            half_maximum: true,
            profiles: false,
        }
    }
}
impl ScreenDisplay {
    fn maximum_weight(&self, peak: f64, reference: f64) -> f64 {
        if self.normalization == ScreenNormalization::Reference {
            self.maximum_percent
                .filter(|v| v.is_finite() && *v > 0.0)
                .map_or(peak, |v| v * reference / 100.0)
        } else {
            peak
        }
    }
}

struct ResultView {
    revision: u64,
    calculation: RunContext,
    scene: Scene,
    report: Report,
    seconds: f64,
    texture: Option<egui::TextureHandle>,
    screen: Option<ScreenAnalysis>,
    roi_cache: Option<(
        crate::screen::ScreenRoi,
        Result<crate::screen::RoiIntegral, String>,
    )>,
}
enum Job {
    Exported(Result<PathBuf, String>),
    Compared(Result<Vec<study::Comparison>, String>),
    Cancelled,
    Scanned(Result<Box<crate::scan::ScanManifest>, String>),
    LoadedRun(Result<Box<crate::record::RunRecord>, String>),
    Added(ComponentKind, Result<Box<Project>, String>),
    Loaded(Result<Box<Project>, String>),
    Traced(Result<Box<ResultView>, String>),
}
struct App {
    viewing_explicit_record: bool,
    data_directory: PathBuf,
    export_requested: Option<PathBuf>,
    preferences: crate::storage::Preferences,
    control: Option<Arc<TraceControl>>,
    study: study::Study,
    camera: LayoutCamera,
    history: history::History,
    project: Project,
    path: Option<PathBuf>,
    loading_path: Option<PathBuf>,
    result: Option<ResultView>,
    job: Option<Receiver<Job>>,
    selected: usize,
    component: usize,
    source: usize,
    detector: usize,
    projection: usize,
    view: View,
    details: Option<Details>,
    source_selected: bool,
    show_rays: bool,
    equal_scale: bool,
    screen_display: ScreenDisplay,
    threads: usize,
    preview_limit: Option<usize>,
    status: String,
    error: Option<String>,
    dirty: bool,
    stale: bool,
    catalogs: Vec<PathBuf>,
    smoke: Option<PathBuf>,
    smoke_frames: usize,
    smoke_ready_at: Option<Instant>,
    screenshot_requested: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: &[String]) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        for path in [
            "C:/Windows/Fonts/meiryo.ttc",
            "C:/Windows/Fonts/msgothic.ttc",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("Japanese".into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .get_mut(&egui::FontFamily::Proportional)
                    .unwrap()
                    .insert(0, "Japanese".into());
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(9.0, 8.0);
        style.visuals.panel_fill = Color32::from_rgb(246, 248, 251);
        style.visuals.selection.bg_fill = BLUE;
        cc.egui_ctx.set_style(style);
        let mut app = Self::demo();
        app.data_directory = crate::storage::data_dir();
        app.preferences = crate::storage::Preferences::load(&crate::storage::data_dir());
        app.study.root = crate::storage::data_dir().join("scans");
        let mut input = None;
        let mut study_input = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--assembly-demo" => (),
                "--study" | "--compare-study" => {
                    app.study.auto_compare = args[i] == "--compare-study";
                    i += 1;
                    study_input = args.get(i).map(PathBuf::from);
                }
                "--screen" => app.view = View::Screen,
                "--settings" => app.details = Some(Details::Calculation),
                "--diagnostics" => app.details = Some(Details::Budget),
                "--import-report" => app.details = Some(Details::Import),
                "--zoom-selected" => app.camera.request = Some(CameraRequest::Selected),
                "--screen-profiles" => app.screen_display.profiles = true,
                "--screen-cursor-mm" => {
                    app.screen_display.cursor_mm = args
                        .get(i + 1)
                        .zip(args.get(i + 2))
                        .and_then(|(x, y)| Some([x.parse().ok()?, y.parse().ok()?]));
                    app.screen_display.profiles = true;
                    i += 2;
                }
                "--screen-roi-mm" => {
                    app.screen_display.roi = (|| {
                        Some(crate::screen::ScreenRoi {
                            minimum_mm: [
                                args.get(i + 1)?.parse().ok()?,
                                args.get(i + 2)?.parse().ok()?,
                            ],
                            maximum_mm: [
                                args.get(i + 3)?.parse().ok()?,
                                args.get(i + 4)?.parse().ok()?,
                            ],
                        })
                    })();
                    i += 4;
                }
                "--screen-reference" => {
                    app.screen_display.normalization = ScreenNormalization::Reference
                }
                "--screen-sqrt" => app.screen_display.scale = ScreenScale::SquareRoot,
                "--screen-max-percent" => {
                    i += 1;
                    app.screen_display.maximum_percent = args
                        .get(i)
                        .and_then(|s| s.parse::<f64>().ok())
                        .filter(|v| v.is_finite() && *v > 0.0);
                    app.screen_display.normalization = ScreenNormalization::Reference;
                }
                "--export-screen" => {
                    i += 1;
                    app.export_requested = args.get(i).map(PathBuf::from);
                }
                "--smoke" => {
                    i += 1;
                    app.smoke = args.get(i).map(PathBuf::from);
                }
                "--allow-uncoated" => app.project.allow_uncoated = true,
                "--wavelength-um" => {
                    i += 1;
                    app.project.monochromatic_override_um =
                        args.get(i).and_then(|s| s.parse().ok());
                }
                p if !p.starts_with('-') => input = Some(PathBuf::from(p)),
                _ => (),
            }
            i += 1;
        }
        if let Some(dir) = study_input {
            app.study.open = true;
            app.open_study_path(dir, &cc.egui_ctx);
        } else if let Some(path) = input {
            app.load(path, &cc.egui_ctx);
        } else {
            app.run(&cc.egui_ctx);
        }
        app
    }
    fn demo() -> Self {
        let mut app = Self {
            viewing_explicit_record: false,
            data_directory: PathBuf::from("results/gui_runtime"),
            export_requested: None,
            preferences: crate::storage::Preferences::default(),
            control: None,
            study: study::Study::default(),
            camera: LayoutCamera::default(),
            history: history::History::default(),
            project: assembly::demo_project().expect("assembly demo"),
            path: None,
            loading_path: None,
            result: None,
            job: None,
            selected: 0,
            component: 2,
            source: 0,
            detector: 0,
            projection: 0,
            view: View::Layout,
            details: None,
            source_selected: false,
            show_rays: true,
            equal_scale: true,
            screen_display: ScreenDisplay::default(),
            threads: 4,
            preview_limit: None,
            status: "準備完了".into(),
            error: None,
            dirty: false,
            stale: false,
            catalogs: Vec::new(),
            smoke: None,
            smoke_frames: 0,
            smoke_ready_at: None,
            screenshot_requested: false,
        };
        app.reset_history(true);
        app
    }
    fn changed(&mut self) {
        self.history.begin();
        self.dirty = true;
        self.stale = self
            .result
            .as_ref()
            .is_some_and(|r| r.revision != self.history.revision);
    }
    fn refresh_components(&mut self) {
        if let Err(e) = self.project.refresh_assembly() {
            self.error = Some(e);
        }
        self.changed();
    }
    fn add_file(&mut self, path: PathBuf, kind: ComponentKind, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let catalogs = self.catalogs.clone();
        let context = ctx.clone();
        self.status = "部品を読込中…".into();
        self.job = Some(rx);
        std::thread::spawn(move || {
            let result = Project::open(&path, &catalogs).map(Box::new);
            let _ = tx.send(Job::Added(kind, result));
            context.request_repaint();
        });
    }
    fn next_z(&self) -> f64 {
        let mut end = self
            .project
            .scene
            .sources
            .iter()
            .map(|s| s.origin_mm.0[2])
            .fold(0.0, f64::max);
        for s in &self.project.scene.surfaces {
            if matches!(s.interaction, Interaction::Detector { .. }) {
                continue;
            }
            if let Some((b, _)) = crate::geometry::bounds(&s.shape, &s.aperture) {
                let f = Frame::new(&s.pose);
                for x in [b[0].0[0], b[1].0[0]] {
                    for y in [b[0].0[1], b[1].0[1]] {
                        for z in [b[0].0[2], b[1].0[2]] {
                            end = end.max(f.world_point(V3([x, y, z])).0[2]);
                        }
                    }
                }
            }
        }
        end + 10.0
    }
    fn append_component(&mut self, mut component: Component) {
        if component.kind != ComponentKind::Source {
            component.pose.translation_mm.0[2] = self.next_z();
        }
        match self.project.add_component(component) {
            Ok(()) => {
                self.component = self.project.assembly.as_ref().unwrap().components.len() - 1;
                self.detector = 0;
                self.changed();
                self.status = "部品を追加しました・位置を調整して計算してください".into();
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn load(&mut self, path: PathBuf, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let catalogs = self.catalogs.clone();
        let context = ctx.clone();
        self.status = "ファイルを読込中…".into();
        self.loading_path = Some(path.clone());
        self.job = Some(rx);
        std::thread::spawn(move || {
            let result = Project::open(&path, &catalogs).map(Box::new);
            let message = if result
                .as_ref()
                .err()
                .is_some_and(|e| e.starts_with("計算記録は"))
            {
                Job::LoadedRun(
                    crate::record::RunRecord::open(&path).and_then(|mut record| {
                        record.report.paths = record_viewer_paths(&record)?;
                        Ok(Box::new(record))
                    }),
                )
            } else {
                Job::Loaded(result)
            };
            let _ = tx.send(message);
            context.request_repaint();
        });
    }
    fn run(&mut self, ctx: &egui::Context) {
        self.commit_edit();
        let revision = self.history.revision;
        let mut scene = match self.project.trace_scene() {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        if let Some(limit) = self.preview_limit {
            for source in &mut scene.sources {
                source.count = source.count.min(limit);
            }
        }
        let threads = self.threads;
        let control = Arc::new(TraceControl::default());
        self.control = Some(control.clone());
        let project_snapshot = self.project.clone();
        let (tx, rx) = mpsc::channel();
        let context = ctx.clone();
        self.job = Some(rx);
        self.status = "光線を追跡中…".into();
        std::thread::spawn(move || {
            let result = (|| -> Result<Option<Box<ResultView>>, String> {
                let total: u128 = scene.sources.iter().map(|s| s.count as u128).sum();
                if total == 0 || total > 2_000_000 {
                    return Err("光線数は合計1〜2,000,000本で指定してください".into());
                }
                if control.cancel.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .map_err(|e| e.to_string())?;
                let prepared = scene.clone().prepare()?;
                if control.cancel.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let rays = prepared.generate_rays()?;
                if rays.is_empty() || rays.len() > 2_000_000 {
                    return Err("光線数は1〜2,000,000本で指定してください".into());
                }
                let power = rays.iter().map(|r| r.weight).sum::<f64>();
                if !power.is_finite() || power <= 0.0 {
                    return Err("発射基準光量は正の有限値で指定してください".into());
                }
                let started = Instant::now();
                let Some(mut report) = pool.install(|| {
                    trace_controlled(&prepared, &rays, TraceOptions::default(), &control)
                }) else {
                    return Ok(None);
                };
                let seconds = started.elapsed().as_secs_f64();
                let n = rays.len().min(64);
                // Each source contributes a prefix; striding a Sobol sequence destroys its balance.
                let mut preview = Vec::with_capacity(n);
                let mut layer = 0;
                while preview.len() < n {
                    let mut offset = 0;
                    for source in &scene.sources {
                        if layer < source.count {
                            preview.push(Ray {
                                id: preview.len() as u64,
                                ..rays[offset + layer]
                            });
                        }
                        offset += source.count;
                        if preview.len() == n {
                            break;
                        }
                    }
                    layer += 1;
                }
                report.paths = pool
                    .install(|| {
                        trace(
                            &prepared,
                            &preview,
                            TraceOptions {
                                keep_terminals: false,
                                path_launch_count: n as u64,
                            },
                        )
                    })
                    .paths;
                Ok(Some(Box::new(ResultView {
                    revision,
                    calculation: RunContext::capture(
                        &project_snapshot,
                        &scene,
                        None,
                        threads,
                        TraceOptions::default(),
                    ),
                    scene,
                    report,
                    seconds,
                    texture: None,
                    screen: None,
                    roi_cache: None,
                })))
            })();
            let message = match result {
                Ok(Some(result)) => Job::Traced(Ok(result)),
                Ok(None) => Job::Cancelled,
                Err(error) => Job::Traced(Err(error)),
            };
            let _ = tx.send(message);
            context.request_repaint();
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_study();
        let message = self.job.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(message) = message {
            self.job = None;
            self.control = None;
            if let Some(path) = self.loading_path.take() {
                if matches!(&message, Job::Loaded(Ok(_)) | Job::LoadedRun(Ok(_))) {
                    self.path = Some(path);
                }
            }
            match message {
                Job::Exported(Ok(dir)) => {
                    self.status = format!("スクリーンを書き出しました：{}", dir.display())
                }
                Job::Compared(Ok(images)) => {
                    self.study.comparison_max = images
                        .iter()
                        .map(|c| c.analysis.peak_weight * 100.0 / c.reference)
                        .fold(0.0, f64::max)
                        .max(1e-12);
                    self.study.comparisons = images;
                    self.study.comparison_open = true;
                    self.study.open = false;
                }
                Job::Cancelled => {
                    self.status = "計算を中止しました。完了済みの結果を保持しています".into()
                }
                Job::Scanned(Ok(manifest)) => {
                    self.study.manifest = Some(*manifest);
                    self.study.open = true;
                    self.status = "走査の計算記録を保存しました".into();
                    if self.study.auto_compare {
                        self.study.auto_compare = false;
                        self.load_comparison(ctx);
                    }
                }
                Job::LoadedRun(Ok(record)) => self.adopt_record(*record),
                Job::Added(kind, Ok(p)) => match Component::imported(*p, kind) {
                    Ok(c) => {
                        self.append_component(c);
                        self.preview_limit = Some(256);
                    }
                    Err(e) => self.error = Some(e),
                },
                Job::Added(_, Err(e)) => self.error = Some(e),
                Job::Loaded(Ok(mut p)) => {
                    p.allow_uncoated |= self.project.allow_uncoated;
                    if self.project.monochromatic_override_um.is_some() {
                        p.monochromatic_override_um = self.project.monochromatic_override_um;
                    }
                    self.preview_limit = (p.zemax.is_some() || p.assembly.is_some()).then_some(256);
                    self.project = *p;
                    self.viewing_explicit_record = false;
                    self.result = None;
                    self.selected = 0;
                    self.component = 0;
                    self.source = 0;
                    self.source_selected = false;
                    self.detector = 0;
                    self.dirty = false;
                    self.stale = false;
                    self.camera = LayoutCamera::default();
                    self.reset_history(true);
                    self.remember_path();
                    self.status = "読込完了".into();
                    if self.project.trace_scene().is_ok() {
                        self.run(ctx);
                    } else {
                        self.details = Some(Details::Import);
                        self.status = "読込レポートで計算条件を確認してください".into();
                    }
                }
                Job::Traced(Ok(r)) => {
                    self.status = "計算完了".into();
                    self.result = Some(*r);
                    self.stale = false;
                }
                Job::Loaded(Err(e))
                | Job::Traced(Err(e))
                | Job::Scanned(Err(e))
                | Job::LoadedRun(Err(e))
                | Job::Compared(Err(e)) => {
                    self.error = Some(e);
                    self.status = "設定を確認してください".into();
                }
                Job::Exported(Err(e)) => self.error = Some(e),
            }
        }
    }
    fn smoke(&mut self, ctx: &egui::Context) {
        let Some(dir) = &self.smoke else {
            return;
        };
        self.smoke_frames += 1;
        if self.job.is_none()
            && (self.result.is_some() || self.error.is_some() || self.smoke_frames > 60)
        {
            self.smoke_ready_at.get_or_insert_with(Instant::now);
        } else {
            self.smoke_ready_at = None;
        }
        if self.job.is_none()
            && (self.result.is_some() || self.error.is_some() || self.smoke_frames > 60)
            && !self.screenshot_requested
            && self.smoke_frames > 5
            && self
                .smoke_ready_at
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(350))
        {
            let _ = std::fs::create_dir_all(dir);
            let screen = self.result.as_ref().and_then(|r| r.screen.as_ref()).map(|a| serde_json::json!({"peak_weight":a.peak_weight,"peak_position_mm":a.peak_position_mm,"pixel_pitch_mm":a.pixel_pitch_mm,"fwhm_x_mm":a.x_profile.as_ref().and_then(|p|p.half_maximum.width_mm()),"fwhm_y_mm":a.y_profile.as_ref().and_then(|p|p.half_maximum.width_mm())}));
            let status = serde_json::json!({
                "scene": self.project.scene.name,
                "view": format!("{:?}", self.view),
                "details": self.details.map(|d| format!("{d:?}")),
                "components": self.project.assembly.as_ref().map(|a| a.components.len()),
                "sources": self.project.scene.sources.len(),
                "surfaces": self.project.scene.surfaces.len(),
                "issues": self.project.issues,
                "compatibility": self.project.compatibility_report(),
                "error": self.error,
                "trace_seconds": self.result.as_ref().map(|r| r.seconds),
                "budget": self.result.as_ref().map(|r| &r.report.budget),
                "diagnostics": self.result.as_ref().map(|r| &r.report.diagnostics),
                "calculation_version": self.result.as_ref().map(|r| &r.calculation.software_version),
                "launch_wavelengths_um": self.result.as_ref().map(|r| &r.calculation.launch_wavelengths_um),
                "screen": screen,
                "camera": &self.camera,
                "scan_cases":self.study.manifest.as_ref().map(|m|&m.cases),
                "comparison_screens":self.study.comparisons.len(),
            });
            let _ = std::fs::write(
                dir.join("gui_smoke.json"),
                serde_json::to_vec_pretty(&status).unwrap(),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
        }
        let screenshots: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
                .collect()
        });
        for image in screenshots {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let _ = image::save_buffer(
                dir.join("gui.png"),
                &bytes,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if self.job.is_none() && self.result.is_some() && self.error.is_none() {
            if let Some(dir) = self.export_requested.take() {
                self.export_current_screen(&dir, ctx);
            }
        }
        self.history_shortcuts(ctx);
        self.toolbar(ctx);
        egui::TopBottomPanel::bottom("status")
            .exact_height(30.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(248, 250, 252))
                    .inner_margin(egui::Margin::symmetric(18, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&self.status).size(11.0).color(MUTED));
                    if let Some(control) = &self.control {
                        let total = control.total.load(Ordering::Relaxed);
                        let completed = control.completed.load(Ordering::Relaxed).min(total);
                        if total > 0 {
                            ui.add(
                                egui::ProgressBar::new(completed as f32 / total as f32)
                                    .desired_width(170.0)
                                    .text(format!("{completed} / {total} 本")),
                            );
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(r) = &self.result {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} 本 · {:.2} ms",
                                    r.calculation.input_count(),
                                    r.seconds * 1000.0
                                ))
                                .size(11.0)
                                .color(MUTED),
                            );
                        }
                    });
                });
            });
        self.sidebar(ctx);
        self.content(ctx);
        self.details_window(ctx);
        self.study_window(ctx);
        self.comparison_window(ctx);
        if let Some(error) = self.error.clone() {
            let mut close = false;
            egui::Window::new("設定・読込の確認")
                .collapsible(false)
                .resizable(true)
                .default_width(540.0)
                .show(ctx, |ui| {
                    ui.label(error);
                    if ui.button("閉じる").clicked() {
                        close = true;
                    }
                });
            if close {
                self.error = None;
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if !ctx.input(|i| i.pointer.any_down()) && !ctx.wants_keyboard_input() {
            self.commit_edit();
        }
        self.smoke(ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped {
            if self.job.is_none() {
                self.load(path, ctx);
            }
        }
    }
}
fn shape_name(s: &Shape) -> &'static str {
    match s {
        Shape::ConicalSide { .. } => "レンズ側面",
        Shape::Plane => "平面",
        Shape::Sphere { .. } => "球面",
        Shape::EvenAsphere { .. } => "非球面",
        Shape::TriangleMesh { .. } => "CAD",
    }
}

fn record_viewer_paths(
    record: &crate::record::RunRecord,
) -> Result<Vec<crate::trace::PathEvent>, String> {
    let scene = &record.calculation.scene;
    let rays = match &record.calculation.input {
        crate::record::LaunchInput::Explicit { rays } => rays
            .iter()
            .take(64)
            .enumerate()
            .map(|(id, r)| Ray {
                id: id as u64,
                ..*r
            })
            .collect::<Vec<_>>(),
        crate::record::LaunchInput::Generated { .. } => {
            let mut preview = scene.clone();
            for s in &mut preview.sources {
                s.count = s.count.min(64);
            }
            let generated = preview.clone().prepare()?.generate_rays()?;
            let total = generated.len().min(64);
            let mut rays = Vec::with_capacity(total);
            let mut layer = 0;
            while rays.len() < total {
                let mut offset = 0;
                for (i, s) in preview.sources.iter().enumerate() {
                    if layer < s.count && rays.len() < total {
                        rays.push(Ray {
                            id: rays.len() as u64,
                            weight: scene.sources[i].total_weight / scene.sources[i].count as f64,
                            ..generated[offset + layer]
                        });
                    }
                    offset += s.count;
                }
                layer += 1;
            }
            rays
        }
    };
    let mut geometry = scene.clone();
    for s in &mut geometry.surfaces {
        if let Interaction::Detector { pixels } = &mut s.interaction {
            *pixels = [1, 1];
        }
    }
    Ok(trace(
        &geometry.prepare()?,
        &rays,
        TraceOptions {
            keep_terminals: false,
            path_launch_count: rays.len() as u64,
        },
    )
    .paths)
}
struct PartMarker {
    index: usize,
    label: String,
    position: V3,
    enabled: bool,
}
#[allow(clippy::too_many_arguments)]
fn draw_paths(
    ui: &mut egui::Ui,
    scene: &Scene,
    paths: &[crate::trace::PathEvent],
    projection: usize,
    height: f32,
    markers: &[PartMarker],
    selected: usize,
    editable: bool,
    equal_scale: bool,
    camera: &mut LayoutCamera,
) -> Option<(usize, V3)> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_rgb(248, 250, 253));
    let axes = [(2, 0), (2, 1), (0, 1)][projection];
    let mut lines: Vec<Vec<V3>> = Vec::new();
    let mut points = Vec::new();
    let mut focus_points = Vec::new();
    let selected_prefix = format!("{}:", selected + 1);
    for (surface_index, s) in scene.surfaces.iter().enumerate() {
        let point_start = points.len();
        let f = Frame::new(&s.pose);
        let (hx, hy) = s.aperture.bounds();
        if let Shape::ConicalSide {
            radius_start_mm,
            radius_end_mm,
            length_mm,
        } = &s.shape
        {
            for theta in [
                0.0,
                std::f64::consts::FRAC_PI_2,
                std::f64::consts::PI,
                3.0 * std::f64::consts::FRAC_PI_2,
            ] {
                let line = vec![
                    f.world_point(V3([
                        radius_start_mm * theta.cos(),
                        radius_start_mm * theta.sin(),
                        0.0,
                    ])),
                    f.world_point(V3([
                        radius_end_mm * theta.cos(),
                        radius_end_mm * theta.sin(),
                        *length_mm,
                    ])),
                ];
                points.extend_from_slice(&line);
                lines.push(line);
            }
        } else if let Shape::TriangleMesh {
            vertices,
            triangles,
            ..
        } = &s.shape
        {
            let step = (triangles.len() / 100).max(1);
            for t in triangles.iter().step_by(step) {
                let line = vec![
                    f.world_point(vertices[t[0] as usize]),
                    f.world_point(vertices[t[1] as usize]),
                    f.world_point(vertices[t[2] as usize]),
                    f.world_point(vertices[t[0] as usize]),
                ];
                points.extend_from_slice(&line);
                lines.push(line);
            }
        } else {
            let extent = if axes.1 == 1 { hy } else { hx };
            let line: Vec<_> = (0..65)
                .filter_map(|i| {
                    let u = -extent + 2.0 * extent * i as f64 / 64.0;
                    let (x, y) = if axes.0 == 0 {
                        match s.aperture {
                            Aperture::Circle { radius_mm } => {
                                let t = std::f64::consts::TAU * i as f64 / 64.0;
                                (radius_mm * t.cos(), radius_mm * t.sin())
                            }
                            Aperture::Rectangle {
                                half_width_mm: x,
                                half_height_mm: y,
                            } => {
                                let t = i as f64 / 16.0;
                                if t <= 1.0 {
                                    (-x + 2.0 * x * t, -y)
                                } else if t <= 2.0 {
                                    (x, -y + 2.0 * y * (t - 1.0))
                                } else if t <= 3.0 {
                                    (x - 2.0 * x * (t - 2.0), y)
                                } else {
                                    (-x, y - 2.0 * y * (t - 3.0))
                                }
                            }
                        }
                    } else if axes.1 == 1 {
                        (0.0, u)
                    } else {
                        (u, 0.0)
                    };
                    Some(f.world_point(V3([x, y, sag(&s.shape, x, y)?.0])))
                })
                .collect();
            points.extend_from_slice(&line);
            lines.push(line);
        }
        if (markers.is_empty() && surface_index == selected)
            || (!markers.is_empty() && s.name.starts_with(&selected_prefix))
        {
            focus_points.extend_from_slice(&points[point_start..]);
        }
    }
    for p in paths {
        points.push(p.start_mm);
        points.push(p.end_mm);
    }
    points.extend(
        markers
            .iter()
            .filter(|m| m.position.finite())
            .map(|m| m.position),
    );
    if points.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "光学面を読み込んでください",
            egui::FontId::proportional(13.0),
            MUTED,
        );
        return None;
    }
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for p in &points {
        for (a, k) in [axes.0, axes.1].into_iter().enumerate() {
            low[a] = low[a].min(p.0[k]);
            high[a] = high[a].max(p.0[k]);
        }
    }
    let inner = rect.shrink2(Vec2::new(44.0, 36.0));
    let grid_id = ui.id().with(("part_grid", projection));
    let active_id = grid_id.with("active");
    if focus_points.is_empty() {
        let center = markers
            .iter()
            .find(|m| m.index == selected)
            .map_or(points[0], |m| m.position);
        focus_points.extend([center - V3([1.0; 3]), center + V3([1.0; 3])]);
    }
    let mut focus_low = [f64::INFINITY; 2];
    let mut focus_high = [f64::NEG_INFINITY; 2];
    for p in &focus_points {
        for (a, k) in [axes.0, axes.1].into_iter().enumerate() {
            focus_low[a] = focus_low[a].min(p.0[k]);
            focus_high[a] = focus_high[a].max(p.0[k]);
        }
    }
    let auto = plot_bounds(low, high, inner.size(), equal_scale);
    let focus = plot_bounds(focus_low, focus_high, inner.size(), equal_scale);
    let requested_view = camera.request.is_some();
    let mut grid = camera.grid(projection, auto, focus, inner.size(), equal_scale);
    let to_screen = |g: [f64; 4], p: V3| {
        Pos2::new(
            inner.left() + ((p.0[axes.0] - g[0]) / g[2]) as f32 * inner.width(),
            inner.bottom() - ((p.0[axes.1] - g[1]) / g[3]) as f32 * inner.height(),
        )
    };
    if response.drag_started_by(egui::PointerButton::Primary) && editable {
        if let Some(pointer) = ui.ctx().input(|i| i.pointer.press_origin()) {
            if let Some(m) = markers
                .iter()
                .filter(|m| {
                    m.position.finite() && to_screen(grid, m.position).distance(pointer) < 22.0
                })
                .min_by(|a, b| {
                    to_screen(grid, a.position)
                        .distance(pointer)
                        .total_cmp(&to_screen(grid, b.position).distance(pointer))
                })
            {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(active_id, m.index);
                    d.insert_temp(grid_id, grid);
                });
            }
        }
    }
    let active = ui.ctx().data_mut(|d| d.get_temp::<usize>(active_id));
    if response.dragged_by(egui::PointerButton::Primary) && active.is_some() {
        if let Some(old) = ui.ctx().data_mut(|d| d.get_temp::<[f64; 4]>(grid_id)) {
            grid = old;
        }
    } else if response.dragged() {
        let delta = ui.ctx().input(|i| i.pointer.delta());
        grid[0] -= delta.x as f64 / inner.width() as f64 * grid[2];
        grid[1] += delta.y as f64 / inner.height() as f64 * grid[3];
        camera.set(projection, grid, inner.size());
    }
    if !response.dragged_by(egui::PointerButton::Primary) {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<[f64; 4]>(grid_id);
            d.remove_temp::<usize>(active_id);
        });
    }
    if !requested_view && response.hovered() && !ui.ctx().input(|i| i.pointer.any_down()) {
        if let Some(pointer) = response.hover_pos().filter(|p| inner.contains(*p)) {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                grid = camera::zoom_grid(
                    grid,
                    [
                        (pointer.x - inner.left()) as f64 / inner.width() as f64,
                        (inner.bottom() - pointer.y) as f64 / inner.height() as f64,
                    ],
                    (-scroll as f64 * 0.003).exp(),
                );
                camera.set(projection, grid, inner.size());
            }
        }
    }
    let map = |p: V3| {
        Pos2::new(
            inner.left() + ((p.0[axes.0] - grid[0]) / grid[2]) as f32 * inner.width(),
            inner.bottom() - ((p.0[axes.1] - grid[1]) / grid[3]) as f32 * inner.height(),
        )
    };
    for i in 0..=4 {
        let y = inner.top() + inner.height() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)],
            Stroke::new(0.5_f32, LINE),
        );
        painter.text(
            Pos2::new(inner.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.1}", grid[1] + grid[3] * (1.0 - i as f64 / 4.0)),
            egui::FontId::proportional(10.0),
            MUTED,
        );
        let x = inner.left() + inner.width() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())],
            Stroke::new(0.5_f32, LINE),
        );
    }
    let max = paths
        .iter()
        .map(|p| p.incident_weight)
        .fold(0.0, f64::max)
        .max(1e-300);
    let geometry_painter = painter.with_clip_rect(inner);
    for p in paths {
        let t = p.incident_weight / max;
        if t < 0.002 {
            continue;
        }
        geometry_painter.line_segment(
            [map(p.start_mm), map(p.end_mm)],
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(224, 135, 48, (25.0 + 155.0 * t.sqrt()) as u8),
            ),
        );
    }
    for line in lines {
        geometry_painter.add(egui::Shape::line(
            line.into_iter().map(map).collect(),
            Stroke::new(1.0_f32, Color32::from_rgb(104, 139, 172)),
        ));
    }
    for m in markers.iter().filter(|m| m.position.finite()) {
        let p = map(m.position);
        if !inner.contains(p) {
            continue;
        }
        let color = if m.enabled { BLUE } else { MUTED };
        geometry_painter.circle_filled(p, if m.index == selected { 6.0 } else { 4.0 }, color);
        if m.index == selected {
            geometry_painter.circle_stroke(p, 9.0, Stroke::new(1.0_f32, BLUE));
        }
        let label =
            painter.layout_no_wrap(m.label.clone(), egui::FontId::proportional(10.0), color);
        let size = label.size();
        let left = if p.x + 8.0 + size.x < rect.right() - 6.0 {
            p.x + 8.0
        } else {
            p.x - size.x - 8.0
        };
        let top = if m.index % 2 == 0 {
            p.y + 12.0
        } else {
            p.y - 12.0 - size.y
        }
        .clamp(rect.top() + 22.0, rect.bottom() - 22.0 - size.y);
        let pos = Pos2::new(left, top);
        geometry_painter.rect_filled(
            egui::Rect::from_min_size(pos, size).expand(2.0),
            2.0,
            Color32::from_rgb(248, 250, 253),
        );
        geometry_painter.galley(pos, label, color);
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 4.0),
        egui::Align2::CENTER_BOTTOM,
        format!(
            "{}  [{:.2} … {:.2}] mm",
            ["X", "Y", "Z"][axes.0],
            grid[0],
            grid[0] + grid[2]
        ),
        egui::FontId::proportional(10.0),
        MUTED,
    );
    painter.text(
        rect.left_top() + Vec2::new(8.0, 7.0),
        egui::Align2::LEFT_TOP,
        format!("{} [mm]  ·  光路は表示用サンプル", ["X", "Y", "Z"][axes.1]),
        egui::FontId::proportional(10.0),
        MUTED,
    );
    ui.label(
        egui::RichText::new(if markers.is_empty() {
            "ホイールで拡大 · 背景をドラッグして表示を移動"
        } else {
            "丸印をドラッグして配置 · ホイールで拡大 · 背景をドラッグして表示を移動"
        })
        .size(10.0)
        .color(MUTED),
    );
    let mut action = None;
    if editable && response.clicked_by(egui::PointerButton::Primary) {
        let pointer = ui
            .ctx()
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if let Some(p) = pointer {
            if let Some(m) = markers
                .iter()
                .filter(|m| m.position.finite() && map(m.position).distance(p) < 22.0)
                .min_by(|a, b| {
                    map(a.position)
                        .distance(p)
                        .total_cmp(&map(b.position).distance(p))
                })
            {
                action = Some((m.index, V3::default()));
            }
        }
    }
    if editable && response.dragged_by(egui::PointerButton::Primary) {
        if let Some(index) = ui.ctx().data_mut(|d| d.get_temp::<usize>(active_id)) {
            let pixel = ui.ctx().input(|i| i.pointer.delta());
            let mut delta = V3::default();
            delta.0[axes.0] = pixel.x as f64 / inner.width() as f64 * grid[2];
            delta.0[axes.1] = -pixel.y as f64 / inner.height() as f64 * grid[3];
            action = Some((index, delta));
        }
    }
    action
}

fn plot_bounds(low: [f64; 2], high: [f64; 2], size: Vec2, equal_scale: bool) -> [f64; 4] {
    let mut span = [(high[0] - low[0]).max(1e-3), (high[1] - low[1]).max(1e-3)];
    if equal_scale {
        let units_per_pixel =
            (span[0] / size.x.max(1.0) as f64).max(span[1] / size.y.max(1.0) as f64);
        span = [
            units_per_pixel * size.x as f64,
            units_per_pixel * size.y as f64,
        ];
    }
    span = [span[0] * 1.12, span[1] * 1.12];
    [
        (low[0] + high[0] - span[0]) * 0.5,
        (low[1] + high[1] - span[1]) * 0.5,
        span[0],
        span[1],
    ]
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
            if ui.checkbox(&mut roi,"ROIの光量を表示").changed() {display.roi=roi.then_some(crate::screen::ScreenRoi{minimum_mm:[-hx/2.,-hy/2.],maximum_mm:[hx/2.,hy/2.]});}
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
                ui.label(error);
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
            result.roi_cache = Some((roi, roi.integrate(map, reference)));
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

fn draw_detector_map(
    ui: &mut egui::Ui,
    map: &crate::trace::DetectorMap,
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

fn screen_color(fraction: f64, scale: ScreenScale) -> Color32 {
    let [r, g, b] = crate::export::rgb(fraction, scale == ScreenScale::SquareRoot);
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

fn draw_profile(
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
pub fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("raytrace-gui {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 860.0])
            .with_min_inner_size([1050.0, 740.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        "Raytrace · 光線追跡",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, &args)))),
    ) {
        eprintln!("{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_scale_preserves_lengths_in_both_axes_and_contains_the_scene() {
        let size = Vec2::new(800.0, 500.0);
        for (low, high) in [([0.0, -2.0], [100.0, 3.0]), ([-1.0, -50.0], [2.0, 90.0])] {
            let grid = plot_bounds(low, high, size, true);
            assert!((grid[2] / size.x as f64 - grid[3] / size.y as f64).abs() < 1e-12);
            for a in 0..2 {
                assert!(grid[a] <= low[a]);
                assert!(grid[a] + grid[a + 2] >= high[a]);
            }
        }
    }

    #[test]
    fn drag_handle_changes_the_component_in_the_selected_projection() {
        let ctx = egui::Context::default();
        let scene = assembly::empty_scene("drag check");
        let markers = [
            PartMarker {
                index: 0,
                label: "A".into(),
                position: V3([0.0, 0.0, 0.0]),
                enabled: true,
            },
            PartMarker {
                index: 1,
                label: "B".into(),
                position: V3([0.0, 0.0, 10.0]),
                enabled: true,
            },
        ];
        let mut camera = LayoutCamera::default();
        let mut frame = |events| {
            let mut action = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(640.0, 480.0),
                )),
                events,
                ..Default::default()
            };
            let output = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = draw_paths(
                        ui,
                        &scene,
                        &[],
                        0,
                        300.0,
                        &markers,
                        0,
                        true,
                        true,
                        &mut camera,
                    );
                });
            });
            (output, action)
        };
        let (output, _) = frame(vec![]);
        let p = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Circle(c) if c.radius == 6.0 => Some(c.center),
                _ => None,
            })
            .unwrap();
        frame(vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, action) = frame(vec![egui::Event::PointerMoved(p + Vec2::new(40.0, 0.0))]);
        let (part, delta) = action.expect("dragging a marker must produce a placement edit");
        assert_eq!(part, 0);
        assert!(delta.0[2] > 0.0);
        assert_eq!(delta.0[0], 0.0);
        assert_eq!(delta.0[1], 0.0);
    }
}
