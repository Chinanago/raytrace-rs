//! `analysis.trace` and `run.replay`: one calculation, its record and its summary.
use super::{
    EngineError, RaysInput, ReplayRequest, TraceRequest, TraceResult, create_parent, load_record,
};
use crate::pipeline::{
    self, DesignOverrides, Launch, LaunchRequest, Limits, SceneOverrides, Schedule,
};
use raytrace_core::{
    math::V3,
    model::{Interaction, Mode, PreparedScene, Ray, Scene},
    trace::{PathEvent, Report, TraceControl, TraceOptions},
};
use raytrace_io::{
    design::Design,
    record::{LaunchInput, RUN_FORMAT, RunContext, RunRecord},
    zemax,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Timing of the repeated trace; see `timed_scope` for what is measured.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct Benchmark {
    pub repetitions: usize,
    pub worker_threads: usize,
    pub input_rays: usize,
    pub minimum_seconds: f64,
    pub median_seconds: f64,
    pub launched_rays_per_second: f64,
    pub surface_intersections_per_second: f64,
    pub timed_scope: String,
}

/// Per-surface encounter totals as percentages of the launched weight.
/// Repeated encounters with the same surface are summed.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct SurfacePercent {
    pub name: String,
    pub incident: f64,
    pub transmitted: f64,
    pub reflected: f64,
    pub absorbed: f64,
    pub detected: f64,
    pub geometric_miss: f64,
}

impl SurfacePercent {
    /// The encounter totals of every surface of `report` as percentages of `reference`.
    pub fn of_report(report: &Report, reference: f64) -> Vec<Self> {
        report
            .surfaces
            .iter()
            .map(|s| Self {
                name: s.name.clone(),
                incident: 100.0 * s.incident_weight / reference,
                transmitted: 100.0 * s.transmitted_weight / reference,
                reflected: 100.0 * s.reflected_weight / reference,
                absorbed: 100.0 * s.absorbed_weight / reference,
                detected: 100.0 * s.detected_weight / reference,
                geometric_miss: 100.0 * s.geometric_miss_weight / reference,
            })
            .collect()
    }
}

/// Calculation record written by `trace` and `replay`.
/// `replay` reruns `calculation` with the recorded optical conditions.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct RunOutput {
    /// Always `raytrace-run-v1`.
    pub format: String,
    /// Output layout version of the record.
    pub schema_version: u32,
    /// Frozen design, effective scene, launch input and software version used for replay.
    pub calculation: RunContext,
    /// Median trace time in seconds (same as `benchmark.median_seconds`).
    pub trace_seconds: f64,
    /// Effective scene after overrides.
    pub scene: Scene,
    /// Launched weight sum; every `*_percent_of_reference` field divides by this value.
    pub reference_weight: f64,
    /// `report.budget` entries as percentages of `reference_weight`.
    pub budget_percent_of_reference: BTreeMap<String, f64>,
    /// Integrated power on each detector, by detector name, as a percentage of `reference_weight`.
    pub detector_percent_of_reference: BTreeMap<String, f64>,
    pub surface_encounter_percent_of_reference: Vec<SurfacePercent>,
    /// `(accounted - launched) / launched` of the power budget.
    pub relative_budget_closure_error: f64,
    pub benchmark: Benchmark,
    /// Raw trace report: budget, detector pixel weights, surface totals and diagnostics.
    pub report: Report,
    /// Display samples derived from the scene, never part of a saved calculation report.
    #[serde(skip)]
    pub viewer_paths: Vec<PathEvent>,
}

struct Calculation {
    design: Design,
    scene: Scene,
    launch: LaunchRequest,
    options: TraceOptions,
    threads: Option<usize>,
    repeat: usize,
    warmup: bool,
    preview_paths: usize,
}

/// A finished calculation: the summary for the caller and the record for the session.
pub(crate) struct Traced {
    pub result: TraceResult,
    pub record: RunRecord,
    pub viewer_paths: Vec<PathEvent>,
}

/// Trace `design`, the model of the request (or the session document). `model_path` is the file
/// it came from, which `output` must not overwrite.
pub(crate) fn trace(
    r: TraceRequest,
    mut design: Design,
    model_path: Option<PathBuf>,
    control: &TraceControl,
) -> Result<Traced, EngineError> {
    let o = r.overrides;
    pipeline::configure_design(
        &mut design,
        DesignOverrides {
            allow_uncoated: o.ignore_coatings,
            wavelength_um: o.wavelength_um,
        },
    );
    if let Some(path) = &r.export_zmx {
        zemax::export(&design, path)?;
    }
    let mut scene = design.trace_scene()?;
    if o.mode == Some(Mode::Sequential) && design.is_assembly() {
        return Err(EngineError::invalid(
            "組立光学系は非シーケンシャルで追跡してください",
        ));
    }
    pipeline::configure_scene(
        &mut scene,
        &SceneOverrides {
            mode: o.mode,
            sampling: o.sampling,
            rays_per_source: o.rays_per_source,
            max_rays_per_source: o.max_rays_per_source,
            ..SceneOverrides::default()
        },
    );
    reject_overwrite(r.output.as_deref(), model_path.as_deref())?;
    let launch = match r.rays {
        Some(RaysInput::Csv(path)) => LaunchRequest::Explicit(read_csv(&path)?),
        Some(RaysInput::Rays(rays)) => LaunchRequest::Explicit(rays),
        None => {
            if let Some(max) = r.max_total_rays
                && pipeline::check_limits(
                    &scene,
                    Limits {
                        max_rays: max,
                        max_screen_pixels: None,
                    },
                )
                .is_err()
            {
                return Err(EngineError::new(
                    "limit",
                    format!("光線数は合計1〜{}本で指定してください", grouped(max)),
                ));
            }
            LaunchRequest::Generated
        }
    };
    let output = run(
        Calculation {
            design,
            scene,
            launch,
            options: TraceOptions {
                keep_terminals: r.terminals,
                path_launch_count: r.paths,
            },
            threads: r.threads,
            repeat: r.repeat,
            warmup: r.warmup,
            preview_paths: r.preview_paths,
        },
        control,
    )?;
    finish(output, r.output, r.export_zmx, r.include_record)
}

pub(crate) fn replay(r: ReplayRequest, control: &TraceControl) -> Result<Traced, EngineError> {
    let (record, path) = load_record(r.record)?;
    reject_overwrite(r.output.as_deref(), path.as_deref())?;
    let mut options = record.calculation.trace_options;
    if let Some(terminals) = r.terminals {
        options.keep_terminals = terminals;
    }
    if let Some(paths) = r.paths {
        options.path_launch_count = paths;
    }
    let calculation = record.calculation;
    let launch = LaunchRequest::from_record(&calculation)?;
    let output = run(
        Calculation {
            design: calculation.design,
            scene: calculation.scene,
            launch,
            options,
            threads: r.threads.or(Some(calculation.worker_threads)),
            repeat: r.repeat,
            warmup: r.warmup,
            preview_paths: 0,
        },
        control,
    )?;
    finish(output, r.output, None, r.include_record)
}

fn reject_overwrite(output: Option<&Path>, input: Option<&Path>) -> Result<(), EngineError> {
    let same = output
        .and_then(|o| fs::canonicalize(o).ok())
        .zip(input.and_then(|i| fs::canonicalize(i).ok()))
        .is_some_and(|(a, b)| a == b);
    if same {
        return Err(EngineError::invalid(
            "出力先は入力モデル・計算記録と異なるパスを指定してください",
        ));
    }
    Ok(())
}

fn run(c: Calculation, control: &TraceControl) -> Result<RunOutput, EngineError> {
    let schedule = Schedule::new(c.warmup, c.repeat)?;
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(n) = c.threads {
        if n == 0 {
            return Err(EngineError::invalid("スレッド数は正の値で指定してください"));
        }
        builder = builder.num_threads(n);
    }
    let pool = builder
        .build()
        .map_err(|e| EngineError::new("limit", format!("スレッドプールを作成できません: {e}")))?;
    let scene = c.scene.prepare()?;
    let launch = pipeline::resolve_launch(&scene, c.launch)?;
    let timed =
        pipeline::run_timed(&pool, &scene, &launch, c.options, control, schedule)?.into_result()?;
    let report = timed.report;
    let timings = timed.timings;
    let input_count = launch.count();
    let reference = pipeline::launched_power(&report).ok_or_else(|| {
        EngineError::new(
            "invalid_ray",
            "発射光線の重み合計は正の有限値にしてください",
        )
    })?;
    let median = timings.median();
    let threads = pool.current_num_threads();
    let warmup = if c.warmup { ", and one warmup" } else { "" };
    let benchmark = Benchmark {
        repetitions: c.repeat,
        worker_threads: threads,
        input_rays: input_count,
        minimum_seconds: timings.minimum(),
        median_seconds: median,
        launched_rays_per_second: input_count as f64 / median,
        surface_intersections_per_second: report.surface_intersections as f64 / median,
        timed_scope: if launch.rays().is_some() {
            format!(
                "Trace, branch traversal, and report accumulation; excludes scene/ray input, JSON output{warmup}"
            )
        } else {
            format!(
                "Source ray generation, trace, branch traversal, and report accumulation; excludes scene input, JSON output{warmup}"
            )
        },
    };
    let calculation = pipeline::capture(&c.design, &scene.spec, &launch, threads, c.options);
    let viewer_paths = if c.preview_paths > 0 && launch.rays().is_none() {
        pool.install(|| preview_paths(&scene, c.preview_paths))?
    } else {
        Vec::new()
    };
    let budget_percent_of_reference = pipeline::budget_percent(&report, reference)?;
    let detector_percent_of_reference = pipeline::detector_percents(&report, reference);
    let surface_encounter_percent_of_reference = SurfacePercent::of_report(&report, reference);
    Ok(RunOutput {
        format: RUN_FORMAT.into(),
        schema_version: 2,
        calculation,
        trace_seconds: median,
        scene: scene.spec.clone(),
        reference_weight: reference,
        budget_percent_of_reference,
        detector_percent_of_reference,
        surface_encounter_percent_of_reference,
        relative_budget_closure_error: pipeline::closure_error(&report, reference),
        benchmark,
        report,
        viewer_paths,
    })
}

fn finish(
    record: RunOutput,
    output: Option<PathBuf>,
    exported_zmx: Option<PathBuf>,
    include_record: bool,
) -> Result<Traced, EngineError> {
    let viewer_paths = record.viewer_paths.clone();
    if let Some(path) = &output {
        create_parent(path)?;
        let bytes = serde_json::to_vec_pretty(&record)
            .map_err(|e| raytrace_io::Error::json(Some(path.clone()), e))?;
        fs::write(path, bytes).map_err(|e| EngineError::io(path, e))?;
    }
    let budget = &record.report.budget;
    let mut warnings = Vec::new();
    if [
        budget.numerical_failure,
        budget.medium_mismatch,
        budget.interaction_limit,
    ]
    .iter()
    .any(|&weight| weight > 0.0)
    {
        warnings.push("未解決の光線重みがあります。計算記録のnumerical_failure・medium_mismatch・interaction_limitを確認してください。".into());
    }
    let mut result = TraceResult {
        output,
        exported_zmx,
        reference_plane: record.scene.reference_plane.clone(),
        launched_rays: record.benchmark.input_rays,
        detected_percent_of_reference: pipeline::detected_percent(
            &record.report,
            record.reference_weight,
        ),
        budget_percent_of_reference: record.budget_percent_of_reference.clone(),
        detector_percent_of_reference: record.detector_percent_of_reference.clone(),
        relative_budget_closure_error: record.relative_budget_closure_error,
        median_seconds: record.trace_seconds,
        worker_threads: record.benchmark.worker_threads,
        warnings,
        viewer_paths: viewer_paths.clone(),
        record: None,
    };
    // The session keeps the record for the analyses. Only a caller that asked for the full
    // record in the result pays for a second copy of it.
    let kept = if include_record {
        let kept = RunRecord {
            format: record.format.clone(),
            calculation: record.calculation.clone(),
            trace_seconds: record.trace_seconds,
            report: record.report.clone(),
        };
        result.record = Some(Box::new(record));
        kept
    } else {
        RunRecord {
            format: record.format,
            calculation: record.calculation,
            trace_seconds: record.trace_seconds,
            report: record.report,
        }
    };
    Ok(Traced {
        result,
        record: kept,
        viewer_paths,
    })
}

/// Up to `count` sample rays taken as prefixes of every source in turn.
/// Striding a Sobol sequence would destroy its balance, so each source contributes a prefix.
fn preview_paths(scene: &PreparedScene, count: usize) -> Result<Vec<PathEvent>, EngineError> {
    let generator = scene.ray_generator()?;
    let n = generator.len().min(count);
    let mut preview = Vec::with_capacity(n);
    let mut layer = 0;
    while preview.len() < n {
        let mut offset = 0;
        for source in &scene.spec.sources {
            if layer < source.count {
                let ray = generator
                    .rays(offset + layer..offset + layer + 1)
                    .next()
                    .ok_or_else(|| {
                        raytrace_core::Error::Internal("プレビューの光線を生成できません".into())
                    })??;
                preview.push(Ray {
                    id: preview.len() as u64,
                    ..ray
                });
            }
            offset += source.count;
            if preview.len() == n {
                break;
            }
        }
        layer += 1;
    }
    Ok(pipeline::trace_once(
        scene,
        &Launch::from_rays(preview),
        TraceOptions {
            keep_terminals: false,
            path_launch_count: n as u64,
        },
        &TraceControl::default(),
    )?
    .into_result()?
    .paths)
}

/// Light paths of up to 64 sample rays of a saved record, for drawing.
pub(crate) fn viewer_paths(record: &RunRecord) -> Result<Vec<PathEvent>, EngineError> {
    let scene = &record.calculation.scene;
    let rays = match &record.calculation.input {
        LaunchInput::Explicit { rays } => rays
            .iter()
            .take(64)
            .enumerate()
            .map(|(id, r)| Ray {
                id: id as u64,
                ..*r
            })
            .collect::<Vec<_>>(),
        LaunchInput::Generated { .. } => {
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
    let path_launch_count = rays.len() as u64;
    Ok(pipeline::trace_once(
        &geometry.prepare()?,
        &Launch::from_rays(rays),
        TraceOptions {
            keep_terminals: false,
            path_launch_count,
        },
        &TraceControl::default(),
    )?
    .into_result()?
    .paths)
}

/// `2000000` → `2,000,000`.
fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

const CSV_HEADER: &str = "id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium";

/// Rays are validated against the prepared scene later, with inline rays.
fn read_csv(path: &Path) -> Result<Vec<Ray>, EngineError> {
    let bad = |detail: String| EngineError::new("invalid_ray", detail);
    let text = fs::read_to_string(path).map_err(|e| EngineError::io(path, e))?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| bad("CSVが空です".into()))?
        .trim_start_matches('\u{feff}')
        .trim();
    if header != CSV_HEADER {
        return Err(bad(format!(
            "光線CSVのヘッダーが不正です（{CSV_HEADER} を指定してください）"
        )));
    }
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: Vec<_> = line.split(',').map(str::trim).collect();
            if v.len() != 10 {
                return Err(bad("CSVは10列で指定してください".into()));
            }
            let number = |i: usize| -> Result<f64, EngineError> {
                v[i].parse()
                    .map_err(|e| bad(format!("CSVの数値が不正です: {e}")))
            };
            Ok(Ray {
                id: v[0]
                    .parse()
                    .map_err(|e| bad(format!("CSVの数値が不正です: {e}")))?,
                position_mm: V3([number(1)?, number(2)?, number(3)?]),
                direction: V3([number(4)?, number(5)?, number(6)?]),
                weight: number(7)?,
                wavelength_um: number(8)?,
                medium: v[9]
                    .parse()
                    .map_err(|e| bad(format!("CSVの数値が不正です: {e}")))?,
                optical_path_mm: 0.0,
            })
        })
        .collect()
}
