//! Small native editor; expensive imports and tracing run off the UI thread.
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use raytrace_core::{
    math::{Frame, V3},
    model::*,
    screen::ScreenAnalysis,
    trace::{Report, TraceControl},
};
use raytrace_io::{
    assembly::{self, Component, ComponentKind},
    project::Project,
    record::RunContext,
    zemax,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};
mod theme;
use theme::{viz, *};

mod camera;
mod components;
mod controls;
mod detector;
mod diagnostics;
mod files;
mod history;
mod jobs;
mod layout;
mod study;
#[cfg(test)]
mod test_support;
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
    roi: Option<raytrace_core::screen::ScreenRoi>,
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
        raytrace_core::screen::ScreenRoi,
        Result<raytrace_core::screen::RoiIntegral, String>,
    )>,
}
enum Job {
    Exported(Result<PathBuf, String>),
    Compared(Result<Vec<study::Comparison>, String>),
    Cancelled,
    Scanned(Result<Box<raytrace_app::scan::ScanManifest>, String>),
    LoadedRun(Result<Box<raytrace_io::record::RunRecord>, String>),
    Added(ComponentKind, Result<Box<Project>, String>),
    Loaded(Result<Box<Project>, String>),
    Traced(Result<Box<ResultView>, String>),
}
struct App {
    viewing_explicit_record: bool,
    data_directory: PathBuf,
    export_requested: Option<PathBuf>,
    preferences: raytrace_io::storage::Preferences,
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
    last_window_title: Option<String>,
    stale: bool,
    catalogs: Vec<PathBuf>,
    smoke: Option<PathBuf>,
    smoke_frames: usize,
    smoke_ready_at: Option<Instant>,
    screenshot_requested: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: &[String]) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self::demo();
        app.data_directory = raytrace_io::storage::data_dir();
        app.preferences =
            raytrace_io::storage::Preferences::load(&raytrace_io::storage::data_dir());
        app.study.root = raytrace_io::storage::data_dir().join("scans");
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
                        Some(raytrace_core::screen::ScreenRoi {
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
            preferences: raytrace_io::storage::Preferences::default(),
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
            last_window_title: None,
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
            self.error = Some(e.to_string());
        }
        self.changed();
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
            if let Some((b, _)) = raytrace_core::geometry::bounds(&s.shape, &s.aperture) {
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
            Err(e) => self.error = Some(e.to_string()),
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
            .exact_height(38.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::muted(&self.status));
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
                                    "{} 本 · {} 面 · {:.2} ms",
                                    r.calculation.input_count(),
                                    r.scene.surfaces.len(),
                                    r.seconds * 1000.0
                                ))
                                .size(SIZE_CAPTION)
                                .color(TEXT_MUTED),
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
                    ui.label(error.to_string());
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
        self.update_window_title(ctx);
        self.smoke(ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped {
            if self.job.is_none() {
                self.load(path, ctx);
            }
        }
    }
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
