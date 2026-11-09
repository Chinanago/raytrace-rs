//! Small native editor; expensive imports and tracing run off the UI thread.
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use raytrace_core::{
    math::{Frame, V3},
    model::*,
    trace::TraceControl,
};
use raytrace_engine::{
    Session,
    analysis::ScreenOutput,
    commands,
    engine::{ComponentAddRequest, DocSetRequest},
};
use raytrace_io::{
    design::{self, Compiled, Design, PartKind},
    record::RunRecord,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver},
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
mod edit;
mod files;
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
    /// The document the result was calculated from. The result is current while the
    /// session still holds this very value.
    design: Arc<Design>,
    /// The ray limit the calculation used.
    preview_limit: Option<usize>,
    /// The calculation record: frozen calculation, trace time and report. It is shared with
    /// the analyses, which take it as their `run`.
    record: Arc<RunRecord>,
    /// The effective scene of `record`.
    scene: Scene,
    texture: Option<egui::TextureHandle>,
    /// The `analysis.screen` result for the displayed screen and cursor.
    screen: Option<ScreenOutput>,
    roi_cache: Option<(
        raytrace_core::screen::ScreenRoi,
        Result<raytrace_core::screen::RoiIntegral, String>,
    )>,
}
enum Job {
    Exported(Result<PathBuf, String>),
    Compared(Result<Vec<study::Comparison>, String>),
    Cancelled,
    Scanned(Result<Box<raytrace_engine::scan::ScanManifest>, String>),
    LoadedRun(Result<Box<RunRecord>, String>),
    /// A copy of the session with the part added, and the group of the edit that adds it.
    Added(PartKind, String, Result<Box<Session>, String>),
    /// A session that opened the file.
    Loaded(Result<Box<Session>, String>),
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
    /// The document, its undo history and its change log. Every edit is a command run on
    /// it (see `edit.rs`); the GUI reads the document with `App::design`.
    session: Session,
    /// The compiled design: flat scene, material names and issues. Valid for the session
    /// revision `compiled_revision`.
    compiled: Compiled,
    compiled_revision: Option<u64>,
    /// Numbers the editing gestures; changes of one gesture share a `group`.
    gesture: u64,
    gesture_used: bool,
    selections: Vec<(std::sync::Weak<Design>, edit::Selection)>,
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
    last_window_title: Option<String>,
    catalogs: Vec<PathBuf>,
    smoke: Option<PathBuf>,
    smoke_frames: usize,
    smoke_ready_at: Option<Instant>,
    screenshot_requested: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: &[String]) -> Self {
        install(&cc.egui_ctx);
        let mut app = Self::demo();
        app.data_directory = raytrace_io::storage::data_dir();
        app.preferences =
            raytrace_io::storage::Preferences::load(&raytrace_io::storage::data_dir());
        app.study.root = raytrace_io::storage::data_dir().join("scans");
        let mut input = None;
        let mut study_input = None;
        let mut allow_uncoated = false;
        let mut wavelength_um = None;
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
                "--allow-uncoated" => allow_uncoated = true,
                "--wavelength-um" => {
                    i += 1;
                    wavelength_um = args.get(i).and_then(|s| s.parse().ok());
                }
                p if !p.starts_with('-') => input = Some(PathBuf::from(p)),
                _ => (),
            }
            i += 1;
        }
        if allow_uncoated || wavelength_um.is_some() {
            // The conditions belong to the starting document, which is not an edit of it.
            let mut design = app.design().clone();
            design.conditions.allow_uncoated |= allow_uncoated;
            design.conditions.monochromatic_override_um = wavelength_um;
            app.start_document(Some(design));
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
        match design::demo_design() {
            Ok(design) => Self::with_design(design),
            Err(e) => {
                let mut app = Self::with_design(Design::empty());
                app.error = Some(format!("サンプルを開けません: {e}"));
                app
            }
        }
    }
    fn with_design(design: Design) -> Self {
        let mut app = Self {
            viewing_explicit_record: false,
            data_directory: PathBuf::from("results/gui_runtime"),
            export_requested: None,
            preferences: raytrace_io::storage::Preferences::default(),
            control: None,
            study: study::Study::default(),
            camera: LayoutCamera::default(),
            session: Session::new(),
            compiled: Compiled {
                scene: design::empty_scene(""),
                material_names: Vec::new(),
                issues: Vec::new(),
            },
            compiled_revision: None,
            gesture: 0,
            gesture_used: false,
            selections: Vec::new(),
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
            last_window_title: None,
            catalogs: Vec::new(),
            smoke: None,
            smoke_frames: 0,
            smoke_ready_at: None,
            screenshot_requested: false,
        };
        app.start_document(Some(design));
        app
    }
    fn next_z(&self) -> f64 {
        let mut end = self
            .compiled
            .scene
            .sources
            .iter()
            .map(|s| s.origin_mm.0[2])
            .fold(0.0, f64::max);
        for s in &self.compiled.scene.surfaces {
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
    /// Add a rod or a screen to the document.
    fn add_generated(&mut self, kind: PartKind) {
        let z = self.next_z();
        self.end_gesture();
        let group = self.group();
        let request = ComponentAddRequest {
            kind: Some(kind),
            group: Some(group.clone()),
            ..ComponentAddRequest::default()
        };
        if let Some(added) = self.execute(commands::COMPONENT_ADD, request) {
            self.place_added(&added.occurrence, kind, z, &group);
        }
        self.end_gesture();
    }
    /// Select the part that was just added, and put it behind the system unless it is a
    /// source. The position joins the undo step of the addition.
    fn place_added(&mut self, occurrence: &str, kind: PartKind, z: f64, group: &str) {
        if kind != PartKind::Source {
            let request = DocSetRequest {
                path: format!("/occurrences/@{occurrence}/pose/translation_mm/2"),
                value: serde_json::json!(z),
                group: Some(group.to_string()),
                expected_revision: None,
            };
            self.execute(commands::DOC_SET, request);
        }
        self.component = self
            .design()
            .occurrences
            .iter()
            .position(|o| o.id.as_str() == occurrence)
            .unwrap_or_else(|| self.design().occurrences.len().saturating_sub(1));
        self.detector = 0;
        self.status = "部品を追加しました・位置を調整して計算してください".into();
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
            let screen = self.result.as_ref().and_then(|r| r.screen.as_ref()).map(|a| serde_json::json!({"peak_weight":a.peak_weight,"peak_position_mm":a.peak_position_mm,"pixel_pitch_mm":a.pixel_pitch_mm,"fwhm_x_mm":a.x_slice.as_ref().and_then(|p|p.fwhm_mm),"fwhm_y_mm":a.y_slice.as_ref().and_then(|p|p.fwhm_mm)}));
            let status = serde_json::json!({
                "scene": self.design().display_name(),
                "view": format!("{:?}", self.view),
                "details": self.details.map(|d| format!("{d:?}")),
                "components": self.design().is_assembly().then_some(self.design().occurrences.len()),
                "sources": self.compiled.scene.sources.len(),
                "surfaces": self.compiled.scene.surfaces.len(),
                "issues": self.compiled.issues,
                "compatibility": self.design().compatibility_report(),
                "error": self.error,
                "trace_seconds": self.result.as_ref().map(|r| r.record.trace_seconds),
                "budget": self.result.as_ref().map(|r| &r.record.report.budget),
                "diagnostics": self.result.as_ref().map(|r| &r.record.report.diagnostics),
                "calculation_version": self.result.as_ref().map(|r| &r.record.calculation.software_version),
                "launch_wavelengths_um": self.result.as_ref().map(|r| &r.record.calculation.launch_wavelengths_um),
                "screen": screen,
                "camera": &self.camera,
                "scan_cases":self.study.manifest.as_ref().map(|m|&m.cases),
                "comparison_screens":self.study.comparisons.len(),
            });
            if let Ok(bytes) = serde_json::to_vec_pretty(&status) {
                let _ = std::fs::write(dir.join("gui_smoke.json"), bytes);
            }
            let path = dir.join("gui.png");
            let context = ctx.clone();
            ctx.request_screenshot(move |image| {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(e) = image::save_buffer(
                    path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("スモークテストのスクリーンショットを保存できません: {e}");
                }
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            });
            self.screenshot_requested = true;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.poll(ctx);
        self.sync_compiled();
        if self.job.is_none()
            && self.result.is_some()
            && self.error.is_none()
            && let Some(dir) = self.export_requested.take()
        {
            self.export_current_screen(&dir, ctx);
        }
        self.history_shortcuts(ctx);
        self.toolbar(ui);
        egui::Panel::bottom("status")
            .exact_size(STATUS_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 0)),
            )
            .show(ui, |ui| {
                ui.add_space(STATUS_PADDING_Y);
                ui.horizontal(|ui| {
                    ui.label(muted(&self.status));
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
                                    r.record.calculation.input_count(),
                                    r.scene.surfaces.len(),
                                    r.record.trace_seconds * 1000.0
                                ))
                                .size(SIZE_CAPTION)
                                .color(TEXT_MUTED),
                            );
                        }
                    });
                });
            });
        self.sidebar(ui);
        let workspace = ui.available_rect_before_wrap();
        self.content(ui);
        self.details_window(ctx, workspace);
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
        self.finish_frame(ctx);
        self.update_window_title(ctx);
        self.smoke(ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped
            && self.job.is_none()
        {
            self.load(path, ctx);
        }
    }
}
pub fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("raytrace-gui {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut args = args;
    let mut renderer = eframe::Renderer::Wgpu;
    let mut automatic = true;
    if let Some(index) = args.iter().position(|a| a == "--renderer") {
        renderer = match args.get(index + 1).map(String::as_str) {
            Some("wgpu") => eframe::Renderer::Wgpu,
            Some("glow") => eframe::Renderer::Glow,
            _ => {
                eprintln!("--renderer requires wgpu or glow");
                std::process::exit(2);
            }
        };
        automatic = false;
        args.drain(index..=index + 1);
    }
    if let Err(error) = run(&args, renderer) {
        if automatic && matches!(error, eframe::Error::Wgpu(_)) {
            let notice = error.to_string().replace(['\r', '\n'], " ");
            eprintln!("wgpu initialization failed; retrying with glow: {notice}");
            if let Err(error) = run(&args, eframe::Renderer::Glow) {
                eprintln!("{error}");
                std::process::exit(1);
            }
        } else {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn run(args: &[String], renderer: eframe::Renderer) -> eframe::Result {
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
            use eframe::wgpu::{Backend, DeviceType};

            let name = std::env::var("WGPU_ADAPTER_NAME").ok();
            adapters
                .iter()
                .filter(|adapter| surface.is_none_or(|s| adapter.is_surface_supported(s)))
                .filter(|adapter| {
                    name.as_ref().is_none_or(|name| {
                        adapter
                            .get_info()
                            .name
                            .to_lowercase()
                            .contains(&name.to_lowercase())
                    })
                })
                .min_by_key(|adapter| {
                    let info = adapter.get_info();
                    // Prefer hardware, then DX12 on Windows. Keep CPU adapters
                    // eligible so WARP works on machines without a usable GPU.
                    let backend = match info.backend {
                        Backend::Dx12 => 0,
                        Backend::Vulkan => 1,
                        _ => 2,
                    };
                    (info.device_type == DeviceType::Cpu, backend)
                })
                .cloned()
                .ok_or_else(|| "no compatible wgpu adapter".into())
        }));
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 860.0])
            .with_min_inner_size([1050.0, 740.0]),
        renderer,
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(
        "Raytrace · 光線追跡",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, args)))),
    )
}
