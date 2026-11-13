use super::*;
use raytrace_engine::analysis::ScreenOutput;
use raytrace_engine::scan::{
    Parameter, ParameterLink, ScanAxis, ScanCase, ScanManifest, ScanPlan, linear_values,
};
use raytrace_engine::{
    commands,
    engine::{ModelInput, ResumeScanRequest, ScanRequest, StudyOpenRequest},
};
use raytrace_io::record::RunRecord;

mod comparison;
mod view;

/// A scan to start or to continue, both run by the same worker.
enum StudyJob {
    Scan(Box<ScanRequest>),
    Resume(ResumeScanRequest),
}

pub(super) struct Study {
    pub auto_compare: bool,
    pub comparisons: Vec<Comparison>,
    pub comparison_open: bool,
    pub comparison_max: f64,
    pair: [usize; 2],
    comparison_detectors: [usize; 2],
    pub open: bool,
    pub parameter: Option<Parameter>,
    secondary: Option<Parameter>,
    secondary_values: String,
    links: Vec<ParameterLink>,
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
    progress: Option<Receiver<Box<ScanManifest>>>,
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
            comparison_detectors: [0, 0],
            open: false,
            parameter: None,
            secondary: None,
            secondary_values: String::new(),
            links: Vec::new(),
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
    map: raytrace_core::trace::DetectorMap,
    pub reference: f64,
    pub analysis: ScreenOutput,
    texture: Option<egui::TextureHandle>,
}

fn parameters(design: &Design) -> Vec<Parameter> {
    let mut p = Vec::new();
    if let Some(part) = design.lone_system() {
        for s in &part.scene.surfaces {
            for axis in [2, 0, 1] {
                p.push(Parameter::SurfacePosition {
                    occurrence: None,
                    surface: s.name.clone(),
                    axis,
                });
            }
            for axis in 0..3 {
                p.push(Parameter::SurfaceRotation {
                    occurrence: None,
                    surface: s.name.clone(),
                    axis,
                });
            }
        }
    } else {
        for (i, o) in design.occurrences.iter().enumerate() {
            let occurrence = o.id.clone();
            for axis in [2, 0, 1] {
                p.push(Parameter::ComponentPosition {
                    occurrence: occurrence.clone(),
                    axis,
                });
            }
            for axis in 0..3 {
                p.push(Parameter::ComponentRotation {
                    occurrence: occurrence.clone(),
                    axis,
                });
            }
            if design.occurrence_part(i).is_some_and(|c| c.rod.is_some()) {
                for axis in [2, 0, 1] {
                    p.push(Parameter::RodSize {
                        occurrence: occurrence.clone(),
                        axis,
                    });
                }
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
fn parameter_choice(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut Parameter,
    options: &[Parameter],
    design: &Design,
) {
    egui::ComboBox::from_id_salt(id)
        .width(390.)
        .selected_text(value.label(design))
        .show_ui(ui, |ui| {
            for p in options {
                ui.selectable_value(value, p.clone(), p.label(design));
            }
        });
}
fn case_label(c: &ScanCase, plan: &ScanPlan) -> String {
    let mut label = format!(
        "{:.5} {}",
        c.value * scale(&plan.parameter),
        unit(&plan.parameter)
    );
    if let Some((s, v)) = plan.secondary.as_ref().zip(c.secondary_value) {
        label += &format!(" / {:.5} {}", v * scale(&s.parameter), unit(&s.parameter));
    }
    label
}

impl App {
    pub(super) fn remember_path(&mut self) {
        if let Some(path) = &self.path {
            self.preferences.remember(path);
            let _ = self.preferences.save(&self.data_directory);
        }
    }
    pub(super) fn configure_study(&mut self) {
        let parameter =
            if self.design().is_assembly() {
                self.design().occurrences.get(self.component).map(|o| {
                    Parameter::ComponentPosition {
                        occurrence: o.id.clone(),
                        axis: 2,
                    }
                })
            } else {
                self.compiled.scene.surfaces.get(self.selected).map(|s| {
                    Parameter::SurfacePosition {
                        occurrence: None,
                        surface: s.name.clone(),
                        axis: 2,
                    }
                })
            }
            .unwrap_or(Parameter::Wavelength);
        self.set_study_parameter(parameter);
        self.study.open = true;
        self.study.use_current = self.preview_limit.is_none();
        self.study.rays = self.preview_limit.unwrap_or(256);
    }
    fn set_study_parameter(&mut self, parameter: Parameter) {
        let value = parameter.value(self.design()).unwrap_or(0.55) * scale(&parameter);
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
            while let Ok(manifest) = rx.try_recv() {
                self.study.manifest = Some(*manifest);
            }
        }
    }
    fn launch_study(&mut self, ctx: &egui::Context, resume: bool) {
        self.end_gesture();
        let prepared = (|| -> Result<(StudyJob, PathBuf), String> {
            if resume {
                self.study
                    .manifest
                    .as_ref()
                    .ok_or("走査記録を開いてください")?;
                let directory = self.study.directory.clone().ok_or("保存先がありません")?;
                let request = StudyJob::Resume(ResumeScanRequest {
                    directory: directory.clone(),
                    include_manifest: true,
                });
                return Ok((request, directory));
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
                linear_values(self.study.start, self.study.end, self.study.count)
                    .map_err(|e| e.to_string())?
            };
            let values = values.into_iter().map(|v| v / scale(&parameter)).collect();
            let plan = ScanPlan {
                parameter,
                values,
                secondary: self
                    .study
                    .secondary
                    .clone()
                    .map(|parameter| -> Result<ScanAxis, String> {
                        let values = self
                            .study
                            .secondary_values
                            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                            .filter(|v| !v.is_empty())
                            .map(|v| {
                                v.parse::<f64>()
                                    .map(|v| v / scale(&parameter))
                                    .map_err(|_| "第2軸の値を確認してください".into())
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        Ok(ScanAxis { parameter, values })
                    })
                    .transpose()?,
                links: self.study.links.clone(),
                threads: self.threads,
                rays_per_source: (!self.study.use_current).then_some(self.study.rays),
            };
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let dir = self.study.root.join(format!("scan_{stamp}"));
            if dir.exists() {
                return Err("同名の走査記録があります。新しい保存先を指定してください".into());
            }
            let request = StudyJob::Scan(Box::new(ScanRequest {
                model: Some(ModelInput::Design(Box::new(self.design().clone()))),
                catalogs: Vec::new(),
                plan,
                output: dir.clone(),
                include_manifest: true,
            }));
            Ok((request, dir))
        })();
        match prepared {
            Ok((request, dir)) => self.spawn_study(request, dir, ctx),
            Err(error) => self.error = Some(error),
        }
    }
    fn spawn_study(&mut self, request: StudyJob, dir: PathBuf, ctx: &egui::Context) {
        self.study.directory = Some(dir);
        let (progress, rx) = mpsc::channel();
        self.study.progress = Some(rx);
        self.status = "パラメーターを走査中…".into();
        self.submit(ctx, true, Some(progress), move |session, context| {
            let result = match request {
                StudyJob::Scan(r) => session.call(commands::STUDY_SCAN, *r, context),
                StudyJob::Resume(r) => session.call(commands::STUDY_RESUME, r, context),
            };
            Job::Scanned(match result {
                Ok(s) => s
                    .manifest
                    .ok_or_else(|| "走査記録が返されませんでした".into()),
                Err(e) => Err(e.message),
            })
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
        self.study.directory = Some(dir.clone());
        self.study.manifest = None;
        let request = StudyOpenRequest { path: dir };
        self.submit(ctx, false, None, move |session, context| {
            Job::Scanned(
                session
                    .call(commands::STUDY_OPEN, request, context)
                    .map(|opened| opened.manifest)
                    .map_err(|e| e.message),
            )
        });
    }
    pub(super) fn adopt_record(
        &mut self,
        record: RunRecord,
        viewer_paths: Arc<Vec<raytrace_core::trace::PathEvent>>,
    ) {
        self.end_gesture();
        let mut design = record.calculation.design.clone();
        let mut sources = record.calculation.scene.sources.iter();
        let copy = |s: &mut Source, actual: &Source| {
            s.count = actual.count;
            s.seed = actual.seed;
            s.sampling = actual.sampling;
            s.wavelength_um = actual.wavelength_um;
            s.total_weight = actual.total_weight;
        };
        if design.is_assembly() {
            for i in 0..design.occurrences.len() {
                if !design.occurrences[i].enabled {
                    continue;
                }
                if let Some(part) = design.occurrence_part_mut(i) {
                    for s in &mut part.scene.sources {
                        if let Some(actual) = sources.next() {
                            copy(s, actual);
                        }
                    }
                }
            }
        } else if let Some(part) = design.lone_system_mut() {
            for s in &mut part.scene.sources {
                if let Some(actual) = sources.next() {
                    copy(s, actual);
                }
            }
        }
        // The record's document starts a new history; a document that does not compile is
        // refused by the session and reported.
        if !self.start_document(Some(design)) {
            return;
        }
        self.viewing_explicit_record = matches!(
            record.calculation.input,
            raytrace_io::record::LaunchInput::Explicit { .. }
        );
        self.preview_limit = None;
        self.component = 0;
        self.selected = 0;
        self.source = 0;
        self.detector = 0;
        self.camera = LayoutCamera::default();
        let Some(design) = self.session.design().cloned() else {
            return;
        };
        self.result = Some(ResultView::of_record(design, None, record));
        if let Some(result) = &mut self.result {
            result.viewer_paths = viewer_paths;
        }
        if let Some(result) = &self.result {
            self.session.set_latest_run_view(
                result.record.clone(),
                None,
                result.viewer_paths.clone(),
            );
        }
        self.view = View::Screen;
        self.status = "保存された計算結果を表示しています".into();
        self.remember_path();
    }
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
        install(&ctx);
        test_support::set_document(&mut app, |design| {
            for s in &mut design.occurrence_part_mut(0).unwrap().scene.sources {
                s.count = 2_000_000;
            }
        });
        app.run(&ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.job.is_some() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
            app.poll(&ctx);
        }
        assert!(
            app.error
                .as_ref()
                .is_some_and(|e| e.contains("合計1〜2,000,000"))
        );
        app.error = None;
        test_support::set_document(&mut app, |design| {
            for s in &mut design.occurrence_part_mut(0).unwrap().scene.sources {
                s.count = 32;
            }
        });
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
        install(&ctx);
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
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
        assert!(
            app.study
                .manifest
                .as_ref()
                .unwrap()
                .cases
                .iter()
                .all(|c| c.state == "completed")
        );
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
            serde_json::to_value(&r.record.report.budget).unwrap(),
            stored_budget
        );
        assert!(!r.viewer_paths.is_empty());
        assert_eq!(r.record.calculation.input_count(), 64);
        assert_eq!(app.design().occurrences[2].pose.translation_mm.0[2], 68.0);
        assert!(!app.is_stale());
        assert_eq!(app.view, View::Screen);
        app.study.pair = [0, 2];
        app.load_comparison(&ctx);
        wait(&mut app, &ctx);
        assert_eq!(app.study.comparisons.len(), 2);
        assert!(app.study.comparison_max > 0.0);
        let output = test_support::run_ui(
            &ctx,
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
