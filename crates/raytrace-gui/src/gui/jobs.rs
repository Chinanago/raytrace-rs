use super::*;
use raytrace_core::trace::{trace, trace_controlled, TraceOptions};

impl App {
    pub(super) fn run(&mut self, ctx: &egui::Context) {
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
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
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
}

pub(super) fn record_viewer_paths(
    record: &raytrace_io::record::RunRecord,
) -> Result<Vec<raytrace_core::trace::PathEvent>, String> {
    let scene = &record.calculation.scene;
    let rays = match &record.calculation.input {
        raytrace_io::record::LaunchInput::Explicit { rays } => rays
            .iter()
            .take(64)
            .enumerate()
            .map(|(id, r)| Ray {
                id: id as u64,
                ..*r
            })
            .collect::<Vec<_>>(),
        raytrace_io::record::LaunchInput::Generated { .. } => {
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
