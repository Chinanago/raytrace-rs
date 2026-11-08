use super::*;
use raytrace_engine::{
    Context as EngineContext, Progress, Session, commands,
    engine::{self, ModelInput, Overrides, TraceRequest},
    scan::ScanManifest,
};
use raytrace_io::record::{RUN_FORMAT, RunRecord};

impl ResultView {
    fn new(design: Arc<Design>, preview_limit: Option<usize>, record: engine::RunOutput) -> Self {
        Self {
            design,
            preview_limit,
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
    /// Run `work` off the UI thread in a new session; it calls engine commands and turns their
    /// results into a job message. Cancellable work exposes its control to the cancel button.
    pub(super) fn submit(
        &mut self,
        ctx: &egui::Context,
        cancellable: bool,
        progress: Option<mpsc::Sender<Box<ScanManifest>>>,
        work: impl FnOnce(&mut Session, &mut EngineContext<'_>) -> Job + Send + 'static,
    ) {
        self.submit_on(Session::new(), ctx, cancellable, progress, work);
    }
    /// [`App::submit`] on a given session, such as a copy of the one the GUI edits.
    pub(super) fn submit_on(
        &mut self,
        mut session: Session,
        ctx: &egui::Context,
        cancellable: bool,
        progress: Option<mpsc::Sender<Box<ScanManifest>>>,
        work: impl FnOnce(&mut Session, &mut EngineContext<'_>) -> Job + Send + 'static,
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
            let job = work(
                &mut session,
                &mut EngineContext::with_progress(&control, &mut report),
            );
            let _ = tx.send(job);
            context.request_repaint();
        });
    }

    pub(super) fn run(&mut self, ctx: &egui::Context) {
        self.end_gesture();
        let Some(design) = self.session.design().cloned() else {
            return;
        };
        let preview_limit = self.preview_limit;
        self.status = "光線を追跡中…".into();
        let request = TraceRequest {
            model: Some(ModelInput::Design(Box::new(Design::clone(&design)))),
            catalogs: Vec::new(),
            rays: None,
            overrides: Overrides {
                max_rays_per_source: preview_limit,
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
        };
        self.submit(ctx, true, None, move |session, context| {
            match session.call(commands::ANALYSIS_TRACE, request, context) {
                Ok(t) => match t.record {
                    Some(record) => Job::Traced(Ok(Box::new(ResultView::new(
                        design,
                        preview_limit,
                        *record,
                    )))),
                    None => Job::Traced(Err("計算記録が返されませんでした".into())),
                },
                Err(e) if e.code == "cancelled" => Job::Cancelled,
                Err(e) => Job::Traced(Err(e.message)),
            }
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
                Job::Added(kind, group, Ok(added)) => {
                    // The position comes from the scene before the part was added.
                    let z = self.next_z();
                    self.session.adopt(*added);
                    self.sync_compiled();
                    let occurrence = self
                        .design()
                        .occurrences
                        .last()
                        .map(|o| o.id.as_str().to_string());
                    if let Some(occurrence) = occurrence {
                        self.place_added(&occurrence, kind, z, &group);
                        self.preview_limit = Some(256);
                    }
                    self.end_gesture();
                }
                Job::Added(_, _, Err(e)) => self.error = Some(e.to_string()),
                Job::Loaded(Ok(opened)) => {
                    let conditions = self.design().conditions.clone();
                    self.session.adopt(*opened);
                    self.selections.clear();
                    self.end_gesture();
                    self.sync_compiled();
                    // Conditions the user chose carry over to the file that is opened next.
                    let mut edits = edit::Edits::default();
                    let opened = self.design().conditions.clone();
                    if conditions.allow_uncoated && !opened.allow_uncoated {
                        edits.set("/conditions/allow_uncoated", &true);
                    }
                    if conditions.monochromatic_override_um.is_some()
                        && conditions.monochromatic_override_um != opened.monochromatic_override_um
                    {
                        edits.set(
                            "/conditions/monochromatic_override_um",
                            &conditions.monochromatic_override_um,
                        );
                    }
                    self.apply(edits);
                    self.end_gesture();
                    self.preview_limit = (self.design().is_assembly()
                        || self
                            .design()
                            .lone_system()
                            .is_some_and(|s| s.provenance.is_some()))
                    .then_some(256);
                    self.viewing_explicit_record = false;
                    self.result = None;
                    self.selected = 0;
                    self.component = 0;
                    self.source = 0;
                    self.source_selected = false;
                    self.detector = 0;
                    self.camera = LayoutCamera::default();
                    self.remember_path();
                    self.status = "読込完了".into();
                    if self.design().trace_scene().is_ok() {
                        self.run(ctx);
                    } else {
                        self.details = Some(Details::Import);
                        self.status = "読込レポートで計算条件を確認してください".into();
                    }
                }
                Job::Traced(Ok(r)) => {
                    self.status = "計算完了".into();
                    self.result = Some(*r);
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
