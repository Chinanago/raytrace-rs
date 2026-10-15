//! Small native editor; expensive imports and tracing run off the UI thread.
use crate::{
    assembly::{self, Component, ComponentKind},
    geometry::sag,
    math::{Frame, V3},
    model::*,
    project::Project,
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
        let mut app = Self {
            project: assembly::demo_project().expect("assembly demo"),
            path: None,
            result: None,
            job: None,
            selected: 0,
            component: 2,
            source: 0,
            detector: 0,
            projection: 0,
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
        let mut input = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--assembly-demo" => (),
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
                    self.detector = 0;
                    self.dirty = false;
                    self.stale = false;
                    self.status = "読込完了・互換性の確認が必要です".into();
                    if self.project.trace_scene().is_ok() {
                        self.run(ctx);
                    }
                }
                Job::Traced(Ok(r)) => {
                    self.status = format!("追跡完了  {:.2} ms", r.seconds * 1000.0);
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
    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(68.0)
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(16.0))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("●").color(BLUE).size(24.0));
                    ui.label(
                        egui::RichText::new("Raytrace")
                            .color(INK)
                            .size(23.0)
                            .strong(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(if self.dirty {
                            format!("{}  •", self.project.scene.name)
                        } else {
                            self.project.scene.name.clone()
                        })
                        .size(12.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.job.is_none(),
                                egui::Button::new(
                                    egui::RichText::new("▶  計算する").color(Color32::WHITE),
                                )
                                .fill(BLUE)
                                .min_size(Vec2::new(118.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.run(ctx);
                        }
                        if ui
                            .add_enabled(
                                self.job.is_none() && self.project.assembly.is_none(),
                                egui::Button::new("ZMX書出し…"),
                            )
                            .on_hover_text(
                                "組立光学系は「保存…」でJSONプロジェクトとして保存します",
                            )
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("optics_edited.zmx")
                                .add_filter("Zemax", &["zmx"])
                                .save_file()
                            {
                                match zemax::export(&self.project, &path) {
                                    Ok(()) => self.status = format!("ZMX保存: {}", path.display()),
                                    Err(e) => self.error = Some(e),
                                }
                            }
                        }
                        if ui.button("保存…").clicked() {
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
                            .add_enabled(self.job.is_none(), egui::Button::new("開く…"))
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Optics", &["zmx", "ZMX", "json"])
                                .pick_file()
                            {
                                self.load(path, ctx);
                            }
                        }
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            ui.menu_button("部品を追加", |ui| {
                                for (label, kind) in [
                                    ("光源ZMX…", ComponentKind::Source),
                                    ("レンズZMX…", ComponentKind::Optic),
                                ] {
                                    if ui.button(label).clicked() {
                                        ui.close_menu();
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("Optics", &["zmx", "ZMX", "json"])
                                            .pick_file()
                                        {
                                            self.add_file(path, kind, ctx);
                                        }
                                    }
                                }
                                if ui.button("ロッド").clicked() {
                                    let demo: Scene = serde_json::from_str(include_str!(
                                        "../examples/integrator_rod.json"
                                    ))
                                    .unwrap();
                                    self.append_component(Component::rod(
                                        demo.materials[1].clone(),
                                        "Silica".into(),
                                    ));
                                    ui.close_menu();
                                }
                                if ui.button("検出器").clicked() {
                                    self.append_component(Component::detector());
                                    ui.close_menu();
                                }
                            });
                            if ui.button("新規").clicked() {
                                self.project = Project::new_assembly();
                                self.result = None;
                                self.path = None;
                                self.component = 0;
                                self.source = 0;
                                self.detector = 0;
                                self.preview_limit = Some(256);
                                self.changed();
                                self.status = "部品を追加して光学系を組み立てます".into();
                            }
                        });
                    });
                });
            });
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("controls")
            .exact_width(246.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(246, 248, 251))
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.label(egui::RichText::new("計算設定").strong().size(17.0));
                        ui.add_space(10.0);
                        ui.label("追跡方式");
                        let mut changed = false;
                        let assembled = self.project.assembly.is_some();
                        changed |= ui
                            .add_enabled(
                                !assembled,
                                egui::Button::new("シーケンシャル")
                                    .selected(self.project.scene.mode == Mode::Sequential),
                            )
                            .clicked();
                        if changed {
                            self.project.scene.mode = Mode::Sequential;
                        }
                        changed |= ui
                            .selectable_value(
                                &mut self.project.scene.mode,
                                Mode::NonSequential,
                                "非シーケンシャル",
                            )
                            .changed();
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(egui::RichText::new("光源").strong());
                        let mut preview = self.preview_limit.is_some();
                        if ui
                            .checkbox(&mut preview, "プレビュー（各光源 最大256本）")
                            .changed()
                        {
                            self.preview_limit = preview.then_some(256);
                            self.stale = true;
                        }
                        if !self.project.scene.sources.is_empty() {
                            self.source = self.source.min(self.project.scene.sources.len() - 1);
                            egui::ComboBox::from_id_salt("source")
                                .selected_text(format!(
                                    "光源 {} / {}",
                                    self.source + 1,
                                    self.project.scene.sources.len()
                                ))
                                .show_ui(ui, |ui| {
                                    for i in 0..self.project.scene.sources.len() {
                                        ui.selectable_value(
                                            &mut self.source,
                                            i,
                                            format!("光源 {}", i + 1),
                                        );
                                    }
                                });
                            let s = &mut self.project.scene.sources[self.source];
                            ui.label("サンプリング");
                            egui::ComboBox::from_id_salt("sampling")
                                .selected_text(match s.sampling {
                                    Sampling::Sobol => "Sobol（低偏差列）",
                                    Sampling::Pseudorandom => "疑似乱数",
                                })
                                .show_ui(ui, |ui| {
                                    changed |= ui
                                        .selectable_value(
                                            &mut s.sampling,
                                            Sampling::Sobol,
                                            "Sobol（低偏差列）",
                                        )
                                        .changed();
                                    changed |= ui
                                        .selectable_value(
                                            &mut s.sampling,
                                            Sampling::Pseudorandom,
                                            "疑似乱数",
                                        )
                                        .changed();
                                });
                            ui.horizontal(|ui| {
                                ui.label("光線数");
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut s.count)
                                            .range(1..=2_000_000)
                                            .speed(100),
                                    )
                                    .changed();
                            });
                            if s.sampling == Sampling::Sobol && !s.count.is_power_of_two() {
                                ui.label(
                                    egui::RichText::new("Sobol推奨本数: 256 / 1024 / 4096…")
                                        .size(11.0)
                                        .color(MUTED),
                                );
                            }
                            let mut nm = s.wavelength_um * 1000.0;
                            ui.horizontal(|ui| {
                                ui.label(if self.project.monochromatic_override_um.is_some() {
                                    "定義の波長"
                                } else {
                                    "波長"
                                });
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut nm)
                                            .range(100.0..=20000.0)
                                            .speed(1.0)
                                            .suffix(" nm"),
                                    )
                                    .changed()
                                {
                                    s.wavelength_um = nm / 1000.0;
                                    changed = true;
                                }
                            });
                            ui.label(
                                egui::RichText::new(if s.diode.is_some() {
                                    "ZMX Source Diode"
                                } else {
                                    "円板 / 円錐光源"
                                })
                                .color(MUTED)
                                .size(12.0),
                            );
                            egui::CollapsingHeader::new("位置・分布").show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("seed");
                                    changed |= ui.add(egui::DragValue::new(&mut s.seed)).changed();
                                });
                                for a in 0..3 {
                                    ui.horizontal(|ui| {
                                        ui.label(["X", "Y", "Z"][a]);
                                        changed |= ui
                                            .add(
                                                egui::DragValue::new(&mut s.origin_mm.0[a])
                                                    .speed(0.1)
                                                    .suffix(" mm"),
                                            )
                                            .changed();
                                    });
                                }
                                if let Some(d) = s.diode.as_mut() {
                                    for a in 0..2 {
                                        ui.horizontal(|ui| {
                                            ui.label(["X発散", "Y発散"][a]);
                                            changed |= ui
                                                .add(
                                                    egui::DragValue::new(&mut d.divergence_deg[a])
                                                        .speed(0.1)
                                                        .suffix(" °"),
                                                )
                                                .changed();
                                        });
                                    }
                                } else {
                                    ui.horizontal(|ui| {
                                        ui.label("円板半径");
                                        changed |= ui
                                            .add(
                                                egui::DragValue::new(&mut s.disk_radius_mm)
                                                    .speed(0.1)
                                                    .range(0.0..=10000.0)
                                                    .suffix(" mm"),
                                            )
                                            .changed();
                                    });
                                }
                            });
                            if ui.button("光線数・方式を全光源へ").clicked() {
                                let count = s.count;
                                let sampling = s.sampling;
                                for s in &mut self.project.scene.sources {
                                    s.count = count;
                                    s.sampling = sampling;
                                }
                                changed = true;
                            }
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(egui::RichText::new("Zemax互換性").strong());
                        if self.project.zemax.is_some() {
                            ui.label(
                                egui::RichText::new("ZMXを直接読込済み")
                                    .color(BLUE)
                                    .size(12.0),
                            );
                        }
                        if self.project.issues.iter().any(|i| i.code == "coating") {
                            changed |= ui
                                .checkbox(&mut self.project.allow_uncoated, "未コートFresnelで計算")
                                .changed();
                            ui.label(
                                egui::RichText::new("元のコーティング名は保持")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        }
                        let mut mono = self.project.monochromatic_override_um.is_some();
                        if ui.checkbox(&mut mono, "単色の波長を明示する").changed() {
                            self.project.monochromatic_override_um = mono.then_some(0.465);
                            changed = true;
                        }
                        if let Some(value) = self.project.monochromatic_override_um.as_mut() {
                            let mut nm = *value * 1000.0;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut nm)
                                        .range(100.0..=20000.0)
                                        .speed(1.0)
                                        .suffix(" nm"),
                                )
                                .changed()
                            {
                                *value = nm / 1000.0;
                                changed = true;
                            }
                        }
                        if !self.project.issues.is_empty() {
                            let blocked = self.project.trace_scene().is_err();
                            ui.label(
                                egui::RichText::new(if blocked {
                                    "確認が必要な項目があります"
                                } else {
                                    "対応条件で計算できます"
                                })
                                .color(if blocked {
                                    Color32::from_rgb(173, 93, 20)
                                } else {
                                    BLUE
                                })
                                .size(12.0),
                            );
                            egui::CollapsingHeader::new(format!(
                                "読込レポート ({})",
                                self.project.issues.len()
                            ))
                            .show(ui, |ui| {
                                for issue in &self.project.issues {
                                    ui.label(
                                        egui::RichText::new(&issue.message).size(11.0).color(MUTED),
                                    );
                                    ui.add_space(5.0);
                                }
                            });
                        }
                        if ui.button("AGFフォルダーを追加…").clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.catalogs.push(dir);
                                if let Some(path) = self.path.clone() {
                                    self.load(path, ctx);
                                }
                            }
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        egui::CollapsingHeader::new("サンプルを開く").show(ui, |ui| {
                            if ui.button("光源＋レンズ＋ロッド").clicked() {
                                self.project = assembly::demo_project().expect("assembly demo");
                                self.path = None;
                                self.result = None;
                                self.component = 2;
                                self.source = 0;
                                self.detector = 0;
                                self.preview_limit = None;
                                self.dirty = false;
                                self.run(ctx);
                            }
                            for (name, text) in [
                                ("レンズ", include_str!("../examples/sequential_lens.json")),
                                ("ロッド", include_str!("../examples/integrator_rod.json")),
                                ("ミラー", include_str!("../examples/folded_mirror.json")),
                            ] {
                                if ui.button(name).clicked() {
                                    self.project =
                                        Project::new(serde_json::from_str(text).unwrap());
                                    self.path = None;
                                    self.result = None;
                                    self.source = 0;
                                    self.selected = 0;
                                    self.detector = 0;
                                    self.dirty = false;
                                    self.run(ctx);
                                }
                            }
                        });
                        egui::CollapsingHeader::new("計算上限").show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("スレッド");
                                ui.add(egui::DragValue::new(&mut self.threads).range(1..=64));
                            });
                            changed |= ui
                                .add(
                                    egui::DragValue::new(
                                        &mut self.project.scene.settings.max_depth,
                                    )
                                    .range(1..=1000)
                                    .prefix("最大深さ "),
                                )
                                .changed();
                        });
                        if changed {
                            if let Some(a) = &mut self.project.assembly {
                                a.sync_sources(&self.project.scene);
                            }
                            if let Err(e) = self.project.refresh_assembly() {
                                self.error = Some(e);
                            }
                            self.changed();
                        }
                    });
                });
            });
    }
    fn components(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("部品と配置").strong().size(16.0));
        let materials: Vec<_> = self
            .project
            .material_names
            .iter()
            .cloned()
            .zip(self.project.scene.materials.iter().cloned())
            .collect();
        let Some(assembly) = &mut self.project.assembly else {
            return;
        };
        if assembly.components.is_empty() {
            ui.label("「部品を追加」から光源・レンズ・ロッド・検出器を追加します。");
            return;
        }
        self.component = self.component.min(assembly.components.len() - 1);
        let mut changed = false;
        let mut remove = false;
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            ui.columns(2, |cols| {
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(&mut cols[0], |ui| {
                        egui::Grid::new("components").striped(true).show(ui, |ui| {
                            ui.label("使用");
                            ui.label("部品");
                            ui.label("種類");
                            ui.label("Z [mm]");
                            ui.end_row();
                            for (i, c) in assembly.components.iter_mut().enumerate() {
                                changed |= ui.checkbox(&mut c.enabled, "").changed();
                                let title = format!(
                                    "{}  {}",
                                    i + 1,
                                    c.name.chars().take(21).collect::<String>()
                                );
                                if ui
                                    .selectable_label(self.component == i, title)
                                    .on_hover_text(&c.name)
                                    .clicked()
                                {
                                    self.component = i;
                                }
                                ui.label(c.kind.label());
                                ui.label(format!("{:.2}", c.pose.translation_mm.0[2]));
                                ui.end_row();
                            }
                        });
                    });
                let ui = &mut cols[1];
                let c = &mut assembly.components[self.component];
                ui.horizontal(|ui| {
                    changed |= ui.text_edit_singleline(&mut c.name).changed();
                    remove = ui.button("削除").clicked();
                });
                ui.horizontal(|ui| {
                    for a in 0..3 {
                        ui.label(["X", "Y", "Z"][a]);
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut c.pose.translation_mm.0[a])
                                    .speed(0.1)
                                    .max_decimals(5),
                            )
                            .changed();
                    }
                    ui.label("mm");
                });
                ui.horizontal(|ui| {
                    for a in 0..3 {
                        ui.label(["θx", "θy", "θz"][a]);
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut c.pose.rotation_deg.0[a])
                                    .speed(0.1)
                                    .max_decimals(3),
                            )
                            .changed();
                    }
                    ui.label("°");
                });
                if let Some(r) = &mut c.rod {
                    ui.horizontal(|ui| {
                        for (name, v) in [
                            ("幅X", &mut r.width_mm),
                            ("幅Y", &mut r.height_mm),
                            ("長さ", &mut r.length_mm),
                        ] {
                            ui.label(name);
                            changed |= ui
                                .add(egui::DragValue::new(v).speed(0.1).range(0.0001..=1e9))
                                .changed();
                        }
                        ui.label("mm");
                    });
                    egui::ComboBox::from_id_salt("rod_material")
                        .width(280.0)
                        .selected_text(
                            c.project
                                .material_names
                                .get(1)
                                .map(String::as_str)
                                .unwrap_or("材料"),
                        )
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
                    let wavelength = self
                        .project
                        .monochromatic_override_um
                        .or_else(|| self.project.scene.sources.first().map(|s| s.wavelength_um))
                        .unwrap_or(0.55);
                    if let Some(n) = c.project.scene.materials[1].index(wavelength) {
                        ui.label(format!("n({:.1} nm) = {:.7}", wavelength * 1000.0, n));
                    }
                }
                if c.kind == ComponentKind::Detector {
                    let s = &mut c.project.scene.surfaces[0];
                    if let Aperture::Rectangle {
                        half_width_mm,
                        half_height_mm,
                    } = &mut s.aperture
                    {
                        ui.horizontal(|ui| {
                            for (name, value) in [("幅", half_width_mm), ("高さ", half_height_mm)]
                            {
                                let mut full = *value * 2.0;
                                ui.label(name);
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut full)
                                            .speed(0.5)
                                            .range(0.0001..=1e9)
                                            .suffix(" mm"),
                                    )
                                    .changed()
                                {
                                    *value = full / 2.0;
                                    changed = true;
                                }
                            }
                        });
                    }
                    if let Interaction::Detector { pixels } = &mut s.interaction {
                        ui.horizontal(|ui| {
                            ui.label("画素 X / Y");
                            for p in pixels {
                                changed |=
                                    ui.add(egui::DragValue::new(p).range(1..=1024)).changed();
                            }
                        });
                    }
                }
                ui.label(
                    egui::RichText::new(match c.kind {
                        ComponentKind::Source => "配置基準：発光配列の中心（Zは最初の発射面）",
                        ComponentKind::Optic => "配置基準：最初の光学面の頂点",
                        ComponentKind::Rod => "配置基準：ロッド入口の中心",
                        _ => "部品全体の位置・傾きを設定します",
                    })
                    .size(11.0)
                    .color(MUTED),
                );
            });
        });
        if remove {
            assembly.components.remove(self.component);
            changed = true;
        }
        if changed {
            self.refresh_components();
        }
    }
    fn surfaces(&mut self, ui: &mut egui::Ui) {
        if self.project.assembly.is_some() {
            self.components(ui);
            return;
        }
        ui.label(egui::RichText::new("光学面").strong().size(16.0));
        if self.project.scene.surfaces.is_empty() {
            ui.label("計算可能な光学面はありません。読込レポートを確認してください。");
            return;
        }
        self.selected = self.selected.min(self.project.scene.surfaces.len() - 1);
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(&mut cols[0], |ui| {
                    egui::Grid::new("surfaces")
                        .striped(true)
                        .spacing([12.0, 7.0])
                        .show(ui, |ui| {
                            ui.label("面");
                            ui.label("形状");
                            ui.label("Z [mm]");
                            ui.end_row();
                            for (i, s) in self.project.scene.surfaces.iter().enumerate() {
                                let name = if s.name.chars().count() > 22 {
                                    format!("{}…", s.name.chars().take(21).collect::<String>())
                                } else {
                                    s.name.clone()
                                };
                                if ui.selectable_label(self.selected == i, name).clicked() {
                                    self.selected = i;
                                }
                                ui.label(shape_name(&s.shape));
                                ui.label(format!("{:.3}", s.pose.translation_mm.0[2]));
                                ui.end_row();
                            }
                        });
                });
            let mut changed = false;
            let wavelength = self
                .project
                .monochromatic_override_um
                .or_else(|| {
                    self.project
                        .scene
                        .sources
                        .get(self.source)
                        .map(|s| s.wavelength_um)
                })
                .unwrap_or(0.55);
            cols[1].add_enabled_ui(self.job.is_none(), |ui| {
                let s = &mut self.project.scene.surfaces[self.selected];
                let title = if s.name.chars().count() > 50 {
                    format!(
                        "{}…{}",
                        s.name.chars().take(12).collect::<String>(),
                        s.name
                            .chars()
                            .skip(s.name.chars().count() - 35)
                            .collect::<String>()
                    )
                } else {
                    s.name.clone()
                };
                ui.label(egui::RichText::new(title).strong())
                    .on_hover_text(&s.name);
                ui.horizontal(|ui| {
                    for a in 0..3 {
                        ui.label(["X", "Y", "Z"][a]);
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut s.pose.translation_mm.0[a])
                                    .speed(0.1)
                                    .max_decimals(5),
                            )
                            .changed();
                    }
                });
                ui.horizontal(|ui| {
                    for a in 0..3 {
                        ui.label(["θx", "θy", "θz"][a]);
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut s.pose.rotation_deg.0[a])
                                    .speed(0.1)
                                    .max_decimals(3),
                            )
                            .changed();
                    }
                });
                match &mut s.shape {
                    Shape::Sphere { radius_mm } | Shape::EvenAsphere { radius_mm, .. } => {
                        ui.horizontal(|ui| {
                            ui.label("曲率半径 [mm]");
                            changed |= ui.add(egui::DragValue::new(radius_mm).speed(0.1)).changed();
                        });
                    }
                    Shape::TriangleMesh { triangles, .. } => {
                        ui.label(
                            egui::RichText::new(format!("CAD  {} triangles", triangles.len()))
                                .size(12.0)
                                .color(MUTED),
                        );
                    }
                    _ => (),
                }
                if let Shape::EvenAsphere { conic, .. } = &mut s.shape {
                    ui.horizontal(|ui| {
                        ui.label("円錐定数");
                        changed |= ui.add(egui::DragValue::new(conic).speed(0.001)).changed();
                    });
                }
                if let Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    ..
                } = &mut s.interaction
                {
                    for (label, value) in [("法線 − 側", minus_medium), ("法線 ＋ 側", plus_medium)]
                    {
                        egui::ComboBox::from_id_salt(label)
                            .selected_text(format!(
                                "{label}: {}",
                                self.project
                                    .material_names
                                    .get(*value)
                                    .map(String::as_str)
                                    .unwrap_or("?")
                            ))
                            .show_ui(ui, |ui| {
                                for (i, name) in self.project.material_names.iter().enumerate() {
                                    changed |= ui.selectable_value(value, i, name).changed();
                                }
                            });
                        if let Some(n) = self
                            .project
                            .scene
                            .materials
                            .get(*value)
                            .and_then(|m| m.index(wavelength))
                        {
                            ui.label(
                                egui::RichText::new(format!(
                                    "n({:.1} nm) = {:.7}",
                                    wavelength * 1000.0,
                                    n
                                ))
                                .size(11.0)
                                .color(MUTED),
                            );
                        }
                    }
                }
            });
            if changed {
                self.changed();
            }
        });
    }
    fn content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("光路と検出器").strong().size(19.0));
                    if self.job.is_some() {
                        ui.spinner();
                    }
                    if self.stale {
                        ui.colored_label(
                            Color32::from_rgb(173, 93, 20),
                            "設定が変わりました・再計算で更新",
                        );
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("projection")
                        .selected_text(["Z–X 投影", "Z–Y 投影", "X–Y 投影"][self.projection])
                        .show_ui(ui, |ui| {
                            for (i, t) in ["Z–X 投影", "Z–Y 投影", "X–Y 投影"].iter().enumerate()
                            {
                                ui.selectable_value(&mut self.projection, i, *t);
                            }
                        });
                    if let Some(r) = &self.result {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} 本  ·  {:.2} ms  ·  {} 面",
                                r.scene.sources.iter().map(|s| s.count).sum::<usize>(),
                                r.seconds * 1000.0,
                                r.scene.surfaces.len()
                            ))
                            .color(MUTED)
                            .size(12.0),
                        );
                    }
                });
                let height = (ui.available_height() * 0.52).clamp(210.0, 370.0);
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
                                label: format!("{} {}", i + 1, c.kind.label()),
                                position: c.pose.translation_mm,
                                enabled: c.enabled,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut placement = None;
                ui.columns(2, |cols| {
                    let scene = if self.project.assembly.is_some() {
                        &self.project.scene
                    } else {
                        self.result
                            .as_ref()
                            .map(|r| &r.scene)
                            .unwrap_or(&self.project.scene)
                    };
                    let paths = if self.stale && self.project.assembly.is_some() {
                        &[][..]
                    } else {
                        self.result
                            .as_ref()
                            .map(|r| r.report.paths.as_slice())
                            .unwrap_or(&[])
                    };
                    placement = draw_paths(
                        &mut cols[0],
                        scene,
                        paths,
                        self.projection,
                        height,
                        &markers,
                        self.component,
                        self.job.is_none(),
                    );
                    let ui = &mut cols[1];
                    if let Some(r) = self.result.as_mut() {
                        if !r.report.detectors.is_empty() {
                            let old = self.detector;
                            self.detector = self.detector.min(r.report.detectors.len() - 1);
                            egui::ComboBox::from_id_salt("detector")
                                .selected_text(&r.report.detectors[self.detector].name)
                                .show_ui(ui, |ui| {
                                    for (i, d) in r.report.detectors.iter().enumerate() {
                                        ui.selectable_value(&mut self.detector, i, &d.name);
                                    }
                                });
                            if old != self.detector {
                                r.texture = None;
                                r.screen = None;
                            }
                            draw_detector(
                                ui,
                                ctx,
                                r,
                                self.detector,
                                height - 32.0,
                                &mut self.screen_display,
                            );
                        } else {
                            ui.label("検出器のないモデルです。");
                        }
                    } else {
                        ui.label(egui::RichText::new("計算後に照度分布を表示します").color(MUTED));
                    }
                });
                if let Some((index, delta)) = placement {
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
                ui.add_space(8.0);
                ui.separator();
                if let Some(r) = &self.result {
                    let b = &r.report.budget;
                    let reference = b.launched;
                    ui.label(
                        egui::RichText::new(format!("基準面：{} = 100%", r.scene.reference_plane))
                            .color(MUTED)
                            .size(11.0),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for (label, value) in [
                            ("検出器合計", b.detected),
                            ("追跡領域外", b.escaped),
                            ("幾何学的外れ", b.geometric_miss),
                            ("主光路からの反射", b.sequential_reflection_not_followed),
                            ("吸収", b.surface_absorbed + b.bulk_absorbed),
                            ("分岐・深さ打切り", b.branch_cutoff + b.interaction_limit),
                            ("未解決", b.numerical_failure + b.medium_mismatch),
                        ] {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{label}  {:.3}%",
                                    100.0 * value / reference
                                ))
                                .size(12.0)
                                .color(
                                    if label == "未解決" && value > 0.0 {
                                        Color32::from_rgb(190, 65, 40)
                                    } else {
                                        INK
                                    },
                                ),
                            );
                            ui.add_space(6.0);
                        }
                    });
                }
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                self.surfaces(ui);
            });
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
            let status = serde_json::json!({"scene":self.project.scene.name,"components":self.project.assembly.as_ref().map(|a|a.components.len()),"sources":self.project.scene.sources.len(),"surfaces":self.project.scene.surfaces.len(),"issues":self.project.issues,"error":self.error,"trace_seconds":self.result.as_ref().map(|r|r.seconds),"budget":self.result.as_ref().map(|r|&r.report.budget),"screen":screen});
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
            .exact_height(27.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&self.status).size(11.0).color(MUTED));
            });
        self.sidebar(ctx);
        self.content(ctx);
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
    let span = [(high[0] - low[0]).max(1e-3), (high[1] - low[1]).max(1e-3)];
    let grid_id = ui.id().with(("part_grid", projection));
    let active_id = grid_id.with("active");
    let mut grid = [low[0], low[1], span[0], span[1]];
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
    let inner = rect.shrink2(Vec2::new(32.0, 28.0));
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
    }
    for line in lines {
        painter.add(egui::Shape::line(
            line.into_iter().map(map).collect(),
            Stroke::new(1.0_f32, Color32::from_rgb(104, 139, 172)),
        ));
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
            "投影の縦横縮尺は独立・CAD外形線は間引き表示"
        } else {
            "●をドラッグして配置（縦横縮尺は独立）・傾きは下の数値で設定"
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
        if ui.button("断面…").clicked() {
            display.profiles = true;
        }
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
    let [hx, hy] = map.half_extent_mm;
    let aspect = (hx / hy) as f32;
    let width = (ui.available_width() - 90.0)
        .min((height - 116.0).max(80.0) * aspect)
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
        result.texture.as_ref().unwrap().id(),
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
    let reference = result.report.budget.launched.max(1e-300);
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
    if display.profiles {
        let mut open = true;
        egui::Window::new("ピークを通る断面")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                if let Some([x, y]) = analysis.peak_position_mm {
                    ui.label(format!("ピーク位置 X={x:.4}, Y={y:.4} mm"));
                    ui.columns(2, |cols| {
                        draw_profile(
                            &mut cols[0],
                            analysis.x_profile.as_ref().unwrap(),
                            "X",
                            hx,
                            peak,
                            display,
                            reference,
                        );
                        draw_profile(
                            &mut cols[1],
                            analysis.y_profile.as_ref().unwrap(),
                            "Y",
                            hy,
                            peak,
                            display,
                            reference,
                        );
                    });
                    ui.label(
                        egui::RichText::new(
                            "赤破線：ピークの50%とその交点。画素中心の値を線形補間。",
                        )
                        .size(11.0)
                        .color(MUTED),
                    );
                } else {
                    ui.label("受光がありません。");
                }
            });
        display.profiles = open;
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
                    action = draw_paths(ui, &scene, &[], 0, 300.0, &markers, 0, true);
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
