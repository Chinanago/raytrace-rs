//! Small native editor; expensive imports and tracing run off the UI thread.
use crate::{
    assembly::{self, Component, ComponentKind},
    geometry::sag,
    math::{Frame, V3},
    model::*,
    project::Project,
    record::RunContext,
    screen::{Profile, ScreenAnalysis},
    trace::{trace, Report, TraceOptions},
    zemax,
};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
const BLUE: Color32 = Color32::from_rgb(40, 90, 180);
const INK: Color32 = Color32::from_rgb(34, 45, 62);
const MUTED: Color32 = Color32::from_rgb(105, 117, 132);
const LINE: Color32 = Color32::from_rgb(221, 226, 234);
const HALF_COLOR: Color32 = Color32::from_rgb(188, 48, 92);

mod workspace;

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
    normalization: ScreenNormalization,
    scale: ScreenScale,
    half_maximum: bool,
    profiles: bool,
}
impl Default for ScreenDisplay {
    fn default() -> Self {
        Self {
            normalization: ScreenNormalization::Peak,
            scale: ScreenScale::Linear,
            half_maximum: true,
            profiles: false,
        }
    }
}

struct ResultView {
    calculation: RunContext,
    scene: Scene,
    report: Report,
    seconds: f64,
    texture: Option<egui::TextureHandle>,
    screen: Option<ScreenAnalysis>,
}
enum Job {
    Added(ComponentKind, Result<Box<Project>, String>),
    Loaded(Result<Box<Project>, String>),
    Traced(Result<Box<ResultView>, String>),
}
struct App {
    project: Project,
    path: Option<PathBuf>,
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
        let mut input = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--assembly-demo" => (),
                "--screen" => app.view = View::Screen,
                "--settings" => app.details = Some(Details::Calculation),
                "--diagnostics" => app.details = Some(Details::Budget),
                "--import-report" => app.details = Some(Details::Import),
                "--screen-profiles" => app.screen_display.profiles = true,
                "--screen-reference" => {
                    app.screen_display.normalization = ScreenNormalization::Reference
                }
                "--screen-sqrt" => app.screen_display.scale = ScreenScale::SquareRoot,
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
        if let Some(path) = input {
            app.load(path, &cc.egui_ctx);
        } else {
            app.run(&cc.egui_ctx);
        }
        app
    }
    fn demo() -> Self {
        Self {
            project: assembly::demo_project().expect("assembly demo"),
            path: None,
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
        }
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.stale = true;
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
                self.result = None;
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
        self.path = Some(path.clone());
        self.job = Some(rx);
        std::thread::spawn(move || {
            let result = Project::open(&path, &catalogs).map(Box::new);
            let _ = tx.send(Job::Loaded(result));
            context.request_repaint();
        });
    }
    fn run(&mut self, ctx: &egui::Context) {
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
        let project_snapshot = self.project.clone();
        let (tx, rx) = mpsc::channel();
        let context = ctx.clone();
        self.job = Some(rx);
        self.status = "光線を追跡中…".into();
        std::thread::spawn(move || {
            let result = (|| -> Result<Box<ResultView>, String> {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .map_err(|e| e.to_string())?;
                let prepared = scene.clone().prepare()?;
                let rays = prepared.generate_rays()?;
                if rays.is_empty() || rays.len() > 2_000_000 {
                    return Err("光線数は1〜2,000,000本で指定してください".into());
                }
                let started = Instant::now();
                let mut report = pool.install(|| trace(&prepared, &rays, TraceOptions::default()));
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
                Ok(Box::new(ResultView {
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
                }))
            })();
            let _ = tx.send(Job::Traced(result));
            context.request_repaint();
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let message = self.job.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(message) = message {
            self.job = None;
            match message {
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
                    self.result = None;
                    self.selected = 0;
                    self.component = 0;
                    self.source = 0;
                    self.source_selected = false;
                    self.detector = 0;
                    self.dirty = false;
                    self.stale = false;
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
                Job::Loaded(Err(e)) | Job::Traced(Err(e)) => {
                    self.error = Some(e);
                    self.status = "設定を確認してください".into();
                }
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(r) = &self.result {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} 本 · {:.2} ms",
                                    r.scene.sources.iter().map(|s| s.count).sum::<usize>(),
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
    for s in &scene.surfaces {
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
    let mut grid = plot_bounds(low, high, inner.size(), equal_scale);
    if response.dragged() {
        if let Some(old) = ui.ctx().data_mut(|d| d.get_temp::<[f64; 4]>(grid_id)) {
            grid = old;
        }
    } else {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<[f64; 4]>(grid_id);
            d.remove_temp::<usize>(active_id);
        });
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
    for p in paths {
        let t = p.incident_weight / max;
        if t < 0.002 {
            continue;
        }
        painter.line_segment(
            [map(p.start_mm), map(p.end_mm)],
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(224, 135, 48, (25.0 + 155.0 * t.sqrt()) as u8),
            ),
        );
    }
    for line in lines {
        painter.add(egui::Shape::line(
            line.into_iter().map(map).collect(),
            Stroke::new(1.0_f32, Color32::from_rgb(104, 139, 172)),
        ));
    }
    for m in markers.iter().filter(|m| m.position.finite()) {
        let p = map(m.position);
        let color = if m.enabled { BLUE } else { MUTED };
        painter.circle_filled(p, if m.index == selected { 6.0 } else { 4.0 }, color);
        if m.index == selected {
            painter.circle_stroke(p, 9.0, Stroke::new(1.0_f32, BLUE));
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
        painter.rect_filled(
            egui::Rect::from_min_size(pos, size).expand(2.0),
            2.0,
            Color32::from_rgb(248, 250, 253),
        );
        painter.galley(pos, label, color);
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
            "光源と面の設定は左側で編集 · CAD外形線は間引き表示"
        } else {
            "丸印をドラッグして配置 · 位置と傾きは左側で設定"
        })
        .size(10.0)
        .color(MUTED),
    );
    let mut action = None;
    if editable && (response.clicked() || response.drag_started()) {
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
                if response.drag_started() {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(grid_id, grid);
                        d.insert_temp(active_id, m.index);
                    });
                }
            }
        }
    }
    if editable && response.dragged() {
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
    let old_scale = display.scale;
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
    });
    if old_scale != display.scale {
        result.texture = None;
    }
    let map = &result.report.detectors[index];
    if result.screen.is_none() {
        match ScreenAnalysis::from_detector(map) {
            Ok(analysis) => result.screen = Some(analysis),
            Err(error) => {
                ui.label(error);
                return;
            }
        }
    }
    let analysis = result.screen.as_ref().unwrap();
    let peak = analysis.peak_weight;
    if result.texture.is_none() {
        let pixels = map
            .weights
            .chunks(map.pixels[0])
            .rev()
            .flatten()
            .map(|w| screen_color(w / peak.max(1e-300), display.scale))
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
    let reference = result.report.budget.launched.max(1e-300);
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
            if let Some([x, y]) = analysis.peak_position_mm {
                ui.label(format!("ピーク位置 X={x:.4}, Y={y:.4} mm"));
                for (axis, profile, half) in [
                    (
                        "X",
                        analysis.x_profile.as_ref().unwrap(),
                        map.half_extent_mm[0],
                    ),
                    (
                        "Y",
                        analysis.y_profile.as_ref().unwrap(),
                        map.half_extent_mm[1],
                    ),
                ] {
                    draw_profile(ui, profile, axis, half, peak, display, reference);
                    ui.add_space(10.0);
                }
                ui.label(
                    egui::RichText::new("赤破線：ピークの50%とその交点。画素中心の値を線形補間。")
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
            screen_color(if peak > 0.0 { fraction } else { 0.0 }, display.scale),
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
            ScreenNormalization::Reference => fraction * 100.0 * peak / reference,
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
    let t = match scale {
        ScreenScale::Linear => fraction,
        ScreenScale::SquareRoot => fraction.sqrt(),
    }
    .clamp(0.0, 1.0) as f32;
    Color32::from_rgb(
        (247.0 - 212.0 * t) as u8,
        (250.0 - 158.0 * t) as u8,
        (254.0 - 78.0 * t) as u8,
    )
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
    if let Some([x, y]) = a.peak_position_mm {
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
    let top = match display.normalization {
        ScreenNormalization::Peak => "1.0".into(),
        ScreenNormalization::Reference => format!("{:.5}%", peak * 100.0 / reference),
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
        let fraction = v / peak;
        let t = match display.scale {
            ScreenScale::Linear => fraction,
            ScreenScale::SquareRoot => fraction.sqrt(),
        };
        inner.bottom() - t as f32 * inner.height()
    };
    painter.add(egui::Shape::line(
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
            Pos2::new(inner.left(), y(peak * 0.5)),
            Pos2::new(inner.right(), y(peak * 0.5)),
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
        let frame = |events| {
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
                    action = draw_paths(ui, &scene, &[], 0, 300.0, &markers, 0, true, true);
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
