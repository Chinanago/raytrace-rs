use super::*;
use raytrace_core::trace::{TraceOptions, trace};

impl App {
    pub(super) fn run(&mut self, _ctx: &egui::Context) {
        self.commit_edit();
        let request = raytrace_engine::Request {
            expected_revision: None,
            command: raytrace_engine::Command::Trace {
                options: raytrace_engine::TraceSettings {
                    threads: self.threads,
                    preview_limit: self.preview_limit,
                    ..Default::default()
                },
            },
        };
        if let Err(error) = self.apply_automation(request) {
            self.error = Some(error.to_string());
        }
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
                    Err(e) => self.error = Some(e.to_string()),
                },
                Job::Added(_, Err(e)) => self.error = Some(e.to_string()),
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
                Job::Loaded(Err(e))
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
            let prepared = scene.clone().prepare().map_err(|e| e.to_string())?;
            let generator = prepared.ray_generator().map_err(|e| e.to_string())?;
            let total = generator.len().min(64);
            let mut rays = Vec::with_capacity(total);
            let mut layer = 0;
            while rays.len() < total {
                let mut offset = 0;
                for source in &scene.sources {
                    if layer < source.count && rays.len() < total {
                        rays.push(Ray {
                            id: rays.len() as u64,
                            ..generator
                                .rays(offset + layer..offset + layer + 1)
                                .next()
                                .unwrap()
                                .map_err(|e| e.to_string())?
                        });
                    }
                    offset += source.count;
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
        &geometry.prepare().map_err(|e| e.to_string())?,
        &rays,
        TraceOptions {
            keep_terminals: false,
            path_launch_count: rays.len() as u64,
        },
    )
    .paths)
}
