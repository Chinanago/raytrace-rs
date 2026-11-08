use super::*;
use raytrace_engine::{
    engine::{self, EngineError, ModelInput, Outcome, Overrides, Progress, Request, TraceRequest},
    scan::ScanManifest,
};
use raytrace_io::record::{RUN_FORMAT, RunRecord};

impl ResultView {
    fn new(revision: u64, record: engine::RunOutput) -> Self {
        Self {
            revision,
            calculation: record.calculation,
            scene: record.scene,
            report: record.report,
            seconds: record.trace_seconds,
            texture: None,
            screen: None,
            roi_cache: None,
        }
    }
    /// The calculation record for saving or export; `paths` are the viewer samples.
    pub(super) fn record(&self, keep_paths: bool) -> RunRecord {
        let mut report = self.report.clone();
        if !keep_paths {
            // Viewer paths use reindexed sample IDs; archive the actual calculation.
            report.paths.clear();
        }
        RunRecord {
            format: RUN_FORMAT.into(),
            calculation: self.calculation.clone(),
            trace_seconds: self.seconds,
            report,
        }
    }
}

impl App {
    /// Run one engine request off the UI thread; `finish` turns its outcome into a job
    /// message. Cancellable requests expose their control to the cancel button.
    pub(super) fn submit(
        &mut self,
        ctx: &egui::Context,
        request: Request,
        cancellable: bool,
        progress: Option<mpsc::Sender<Box<ScanManifest>>>,
        finish: impl FnOnce(Result<Outcome, EngineError>) -> Job + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        let control = Arc::new(TraceControl::default());
        if cancellable {
            self.control = Some(control.clone());
        }
        let context = ctx.clone();
        std::thread::spawn(move || {
            let mut report = |p: Progress<'_>| {
                if let (Progress::Scan(m), Some(progress)) = (p, &progress) {
                    let _ = progress.send(Box::new(m.clone()));
                    context.request_repaint();
                }
            };
            let result = engine::execute_with(request, &control, &mut report);
            let _ = tx.send(finish(result));
            context.request_repaint();
        });
    }

    pub(super) fn run(&mut self, ctx: &egui::Context) {
        self.commit_edit();
        let revision = self.history.revision;
        self.status = "光線を追跡中…".into();
        let request = Request::Trace(TraceRequest {
            model: ModelInput::Design(Box::new(self.design.clone())),
            catalogs: Vec::new(),
            rays: None,
            overrides: Overrides {
                max_rays_per_source: self.preview_limit,
                ..Overrides::default()
            },
            threads: Some(self.threads),
            repeat: 1,
            warmup: false,
            terminals: false,
            paths: 0,
            preview_paths: 64,
            max_total_rays: Some(2_000_000),
            export_zmx: None,
            output: None,
            include_record: true,
        });
        self.submit(ctx, request, true, None, move |result| match result {
            Ok(Outcome::Traced(t)) => match t.record {
                Some(record) => Job::Traced(Ok(Box::new(ResultView::new(revision, *record)))),
                None => Job::Traced(Err("計算記録が返されませんでした".into())),
            },
            Ok(_) => Job::Traced(Err("追跡以外の結果が返されました".into())),
            Err(e) if e.code == "cancelled" => Job::Cancelled,
            Err(e) => Job::Traced(Err(e.message)),
        });
    }
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.poll_study();
        let message = self.job.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(message) = message {
            self.job = None;
            self.control = None;
            if let Some(path) = self.loading_path.take()
                && matches!(&message, Job::Loaded(Ok(_)) | Job::LoadedRun(Ok(_)))
            {
                self.path = Some(path);
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
                Job::Added(kind, Ok(p)) => match design::imported_component(*p, kind) {
                    Ok(c) => {
                        self.append_component(c);
                        self.preview_limit = Some(256);
                    }
                    Err(e) => self.error = Some(e.to_string()),
                },
                Job::Added(_, Err(e)) => self.error = Some(e.to_string()),
                Job::Loaded(Ok(mut p)) => {
                    p.conditions.allow_uncoated |= self.design.conditions.allow_uncoated;
                    if self.design.conditions.monochromatic_override_um.is_some() {
                        p.conditions.monochromatic_override_um =
                            self.design.conditions.monochromatic_override_um;
                    }
                    self.preview_limit = (p.is_assembly()
                        || p.lone_system().is_some_and(|s| s.provenance.is_some()))
                    .then_some(256);
                    self.design = *p;
                    self.recompile();
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
                    if self.design.trace_scene().is_ok() {
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
                    self.error = Some(e.to_string());
                    self.status = "設定を確認してください".into();
                }
                Job::Exported(Err(e)) => self.error = Some(e.to_string()),
            }
        }
    }
}
