//! Small native editor; expensive imports and tracing run off the UI thread.
use crate::{
    geometry::sag,
    math::{Frame, V3},
    model::*,
    project::Project,
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

struct ResultView {
    scene: Scene,
    report: Report,
    seconds: f64,
    texture: Option<egui::TextureHandle>,
}
enum Job {
    Loaded(Result<Box<Project>, String>),
    Traced(Result<Box<ResultView>, String>),
}
struct App {
    project: Project,
    path: Option<PathBuf>,
    result: Option<ResultView>,
    job: Option<Receiver<Job>>,
    selected: usize,
    source: usize,
    detector: usize,
    projection: usize,
    threads: usize,
    preview_limit: Option<usize>,
    status: String,
    error: Option<String>,
    dirty: bool,
    stale: bool,
    catalogs: Vec<PathBuf>,
    smoke: Option<PathBuf>,
    smoke_frames: usize,
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
        let scene = serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
        let mut app = Self {
            project: Project::new(scene),
            path: None,
            result: None,
            job: None,
            selected: 0,
            source: 0,
            detector: 0,
            projection: 0,
            threads: 4,
            preview_limit: None,
            status: "準備完了".into(),
            error: None,
            dirty: false,
            stale: false,
            catalogs: Vec::new(),
            smoke: None,
            smoke_frames: 0,
            screenshot_requested: false,
        };
        let mut input = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
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
                Job::Loaded(Ok(mut p)) => {
                    p.allow_uncoated |= self.project.allow_uncoated;
                    if self.project.monochromatic_override_um.is_some() {
                        p.monochromatic_override_um = self.project.monochromatic_override_um;
                    }
                    self.preview_limit = p.zemax.as_ref().map(|_| 256);
                    self.project = *p;
                    self.result = None;
                    self.selected = 0;
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
                            .add_enabled(self.job.is_none(), egui::Button::new("ZMX書出し…"))
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
                            self.changed();
                        }
                    });
                });
            });
    }
    fn surfaces(&mut self, ui: &mut egui::Ui) {
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
                ui.columns(2, |cols| {
                    let scene = self
                        .result
                        .as_ref()
                        .map(|r| &r.scene)
                        .unwrap_or(&self.project.scene);
                    let paths = self
                        .result
                        .as_ref()
                        .map(|r| r.report.paths.as_slice())
                        .unwrap_or(&[]);
                    draw_paths(&mut cols[0], scene, paths, self.projection, height);
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
                            }
                            draw_detector(ui, ctx, r, self.detector, height - 32.0);
                        } else {
                            ui.label("検出器のないモデルです。");
                        }
                    } else {
                        ui.label(egui::RichText::new("計算後に照度分布を表示します").color(MUTED));
                    }
                });
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
            && !self.screenshot_requested
            && self.smoke_frames > 5
        {
            let _ = std::fs::create_dir_all(dir);
            let status = serde_json::json!({"scene":self.project.scene.name,"sources":self.project.scene.sources.len(),"surfaces":self.project.scene.surfaces.len(),"issues":self.project.issues,"error":self.error,"trace_seconds":self.result.as_ref().map(|r|r.seconds),"budget":self.result.as_ref().map(|r|&r.report.budget)});
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
        Shape::Plane => "平面",
        Shape::Sphere { .. } => "球面",
        Shape::EvenAsphere { .. } => "非球面",
        Shape::TriangleMesh { .. } => "CAD",
    }
}
fn draw_paths(
    ui: &mut egui::Ui,
    scene: &Scene,
    paths: &[crate::trace::PathEvent],
    projection: usize,
    height: f32,
) {
    let (rect, _response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_rgb(248, 250, 253));
    let axes = [(2, 0), (2, 1), (0, 1)][projection];
    let mut lines: Vec<Vec<V3>> = Vec::new();
    let mut points = Vec::new();
    for s in &scene.surfaces {
        let f = Frame::new(&s.pose);
        let (hx, hy) = s.aperture.bounds();
        if let Shape::TriangleMesh {
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
                    let (x, y) = if axes.1 == 1 { (0.0, u) } else { (u, 0.0) };
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
    if points.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "光学面を読み込んでください",
            egui::FontId::proportional(13.0),
            MUTED,
        );
        return;
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
    let inner = rect.shrink2(Vec2::new(32.0, 28.0));
    let map = |p: V3| {
        Pos2::new(
            inner.left() + ((p.0[axes.0] - low[0]) / span[0]) as f32 * inner.width(),
            inner.bottom() - ((p.0[axes.1] - low[1]) / span[1]) as f32 * inner.height(),
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
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 4.0),
        egui::Align2::CENTER_BOTTOM,
        format!(
            "{}  [{:.2} … {:.2}] mm",
            ["X", "Y", "Z"][axes.0],
            low[0],
            high[0]
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
        egui::RichText::new("投影の縦横縮尺は独立・CAD外形線は間引き表示")
            .size(10.0)
            .color(MUTED),
    );
}
fn draw_detector(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    result: &mut ResultView,
    index: usize,
    height: f32,
) {
    let map = &result.report.detectors[index];
    let max = map.weights.iter().copied().fold(0.0, f64::max).max(1e-300);
    if result.texture.is_none() {
        let pixels = map
            .weights
            .chunks(map.pixels[0])
            .rev()
            .flatten()
            .map(|w| {
                let t = (w / max).sqrt() as f32;
                Color32::from_rgb(
                    (247.0 - 212.0 * t) as u8,
                    (250.0 - 158.0 * t) as u8,
                    (254.0 - 78.0 * t) as u8,
                )
            })
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
    let width = ui.available_width().min((height - 34.0) * aspect).max(30.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Image::new((
            result.texture.as_ref().unwrap().id(),
            Vec2::new(width, width / aspect),
        )));
    });
    ui.label(
        egui::RichText::new(format!(
            "この検出器 {:.3}%  ·  幅 {:.3} × {:.3} mm",
            100.0 * map.weights.iter().sum::<f64>() / result.report.budget.launched,
            hx * 2.0,
            hy * 2.0
        ))
        .size(11.0)
        .color(MUTED),
    );
    ui.label(
        egui::RichText::new(format!(
            "1画素 0 〜 {:.5}%（色は平方根目盛）",
            100.0 * max / result.report.budget.launched
        ))
        .size(10.0)
        .color(MUTED),
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
