//! `trace` and `replay`: one calculation, its record and its summary.
use super::{
    EngineError, ModelInput, RaysInput, ReplayRequest, TraceRequest, TraceResult, create_parent,
    load, load_record,
};
use raytrace_core::{
    math::V3,
    model::{Interaction, Mode, PreparedScene, Ray, Scene},
    trace::{
        Report, TraceControl, TraceOptions, trace as trace_rays, trace_controlled,
        trace_generated_controlled,
    },
};
use raytrace_io::{
    project::Project,
    record::{LaunchInput, RUN_FORMAT, RunContext, RunRecord},
    zemax,
};
use serde::Serialize;
use std::{collections::BTreeMap, collections::HashSet, fs, path::Path, time::Instant};

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

/// Calculation record written by `trace` and `replay`.
/// `replay` reruns `calculation` with the recorded optical conditions.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct RunOutput {
    /// Always `raytrace-run-v1`.
    pub format: String,
    /// Output layout version of the record.
    pub schema_version: u32,
    /// Frozen project, effective scene, launch input and software version used for replay.
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
}

/// Launch rays before the scene is prepared and validated.
enum Launch {
    Generated,
    /// Rays from a request or CSV; validated against the prepared scene.
    Explicit(Vec<Ray>),
    /// Rays of an explicit-input record, validated when the record was opened.
    Recorded(Vec<Ray>),
    /// Generated input of a record; the source settings must still match.
    RecordedGenerated(Box<RunContext>),
}

struct Calculation {
    project: Project,
    scene: Scene,
    launch: Launch,
    options: TraceOptions,
    threads: Option<usize>,
    repeat: usize,
    warmup: bool,
    preview_paths: usize,
}

pub(super) fn trace(r: TraceRequest, control: &TraceControl) -> Result<TraceResult, EngineError> {
    let model_path = match &r.model {
        ModelInput::Path(path) => Some(path.clone()),
        _ => None,
    };
    let mut project = load(r.model, &r.catalogs)?;
    let o = r.overrides;
    project.allow_uncoated |= o.ignore_coatings;
    if let Some(w) = o.wavelength_um {
        project.monochromatic_override_um = Some(w);
    }
    if let Some(path) = &r.export_zmx {
        zemax::export(&project, path)?;
    }
    let mut scene = project.trace_scene()?;
    for source in &mut scene.sources {
        if let Some(sampling) = o.sampling {
            source.sampling = sampling;
        }
        if let Some(count) = o.rays_per_source {
            source.count = count;
        }
        if let Some(limit) = o.max_rays_per_source {
            source.count = source.count.min(limit);
        }
    }
    if let Some(mode) = o.mode {
        if project.assembly.is_some() && mode == Mode::Sequential {
            return Err(EngineError::invalid(
                "組立光学系は非シーケンシャルで追跡してください",
            ));
        }
        scene.mode = mode;
    }
    reject_overwrite(r.output.as_deref(), model_path.as_deref())?;
    let launch = match r.rays {
        Some(RaysInput::Csv(path)) => Launch::Explicit(read_csv(&path)?),
        Some(RaysInput::Rays(rays)) => Launch::Explicit(rays),
        None => {
            if let Some(max) = r.max_total_rays {
                let total: u128 = scene.sources.iter().map(|s| s.count as u128).sum();
                if total == 0 || total > max as u128 {
                    return Err(EngineError::new(
                        "limit",
                        format!("光線数は合計1〜{}本で指定してください", grouped(max)),
                    ));
                }
            }
            Launch::Generated
        }
    };
    let output = run(
        Calculation {
            project,
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

pub(super) fn replay(r: ReplayRequest, control: &TraceControl) -> Result<TraceResult, EngineError> {
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
    let launch = match &calculation.input {
        LaunchInput::Explicit { .. } => Launch::Recorded(calculation.rays()?),
        LaunchInput::Generated { .. } => Launch::RecordedGenerated(Box::new(calculation.clone())),
    };
    let output = run(
        Calculation {
            project: calculation.project,
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
    if c.repeat == 0 {
        return Err(EngineError::invalid("繰返し回数は正の値で指定してください"));
    }
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
    let rays = match c.launch {
        Launch::Generated => None,
        Launch::Explicit(mut rays) => {
            let mut ids = HashSet::new();
            for ray in &mut rays {
                if !ids.insert(ray.id) {
                    return Err(EngineError::new(
                        "invalid_ray",
                        "発射光線IDが重複しています",
                    ));
                }
                ray.validate(&scene)?;
            }
            Some(rays)
        }
        Launch::Recorded(rays) => Some(rays),
        Launch::RecordedGenerated(calculation) => {
            calculation.validate_generated(&scene)?;
            None
        }
    };
    let input_count = match &rays {
        Some(rays) => {
            if !rays.iter().map(|r| r.weight).sum::<f64>().is_finite()
                || rays.iter().all(|r| r.weight == 0.0)
            {
                return Err(EngineError::new(
                    "invalid_ray",
                    "発射光線の重み合計は正の有限値にしてください",
                ));
            }
            rays.len()
        }
        None => scene.ray_generator()?.len(),
    };
    if input_count == 0 {
        return Err(EngineError::invalid(
            "光源または明示入力の光線を指定してください",
        ));
    }
    let once = || -> Result<Report, EngineError> {
        match &rays {
            Some(rays) => trace_controlled(&scene, rays, c.options, control),
            None => trace_generated_controlled(&scene, c.options, control)?,
        }
        .ok_or_else(EngineError::cancelled)
    };
    if c.warmup {
        std::hint::black_box(pool.install(once)?);
    }
    let mut timings = Vec::with_capacity(c.repeat);
    let mut report = None;
    for _ in 0..c.repeat {
        let start = Instant::now();
        let result = pool.install(once)?;
        timings.push(start.elapsed().as_secs_f64());
        report = Some(result);
    }
    let mut report = report.expect("repeat is positive");
    let reference = report.budget.launched;
    if !reference.is_finite() || reference <= 0. {
        return Err(EngineError::new(
            "invalid_ray",
            "発射光線の重み合計は正の有限値にしてください",
        ));
    }
    timings.sort_by(f64::total_cmp);
    let median = if c.repeat.is_multiple_of(2) {
        (timings[c.repeat / 2 - 1] + timings[c.repeat / 2]) * 0.5
    } else {
        timings[c.repeat / 2]
    };
    let threads = pool.current_num_threads();
    let warmup = if c.warmup { ", and one warmup" } else { "" };
    let benchmark = Benchmark {
        repetitions: c.repeat,
        worker_threads: threads,
        input_rays: input_count,
        minimum_seconds: timings[0],
        median_seconds: median,
        launched_rays_per_second: input_count as f64 / median,
        surface_intersections_per_second: report.surface_intersections as f64 / median,
        timed_scope: if rays.is_some() {
            format!(
                "Trace, branch traversal, and report accumulation; excludes scene/ray input, JSON output{warmup}"
            )
        } else {
            format!(
                "Source ray generation, trace, branch traversal, and report accumulation; excludes scene input, JSON output{warmup}"
            )
        },
    };
    let calculation =
        RunContext::capture(&c.project, &scene.spec, rays.as_deref(), threads, c.options);
    if c.preview_paths > 0 && rays.is_none() {
        report.paths = pool.install(|| preview_paths(&scene, c.preview_paths))?;
    }
    let budget =
        serde_json::to_value(&report.budget).map_err(|e| raytrace_io::Error::json(None, e))?;
    let budget_percent_of_reference = budget
        .as_object()
        .expect("the budget serializes as an object")
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                100.0 * value.as_f64().unwrap_or(0.0) / reference,
            )
        })
        .collect();
    let detector_percent_of_reference = report
        .detectors
        .iter()
        .map(|d| {
            (
                d.name.clone(),
                100.0 * d.weights.iter().sum::<f64>() / reference,
            )
        })
        .collect();
    let surface_encounter_percent_of_reference = report
        .surfaces
        .iter()
        .map(|s| SurfacePercent {
            name: s.name.clone(),
            incident: 100.0 * s.incident_weight / reference,
            transmitted: 100.0 * s.transmitted_weight / reference,
            reflected: 100.0 * s.reflected_weight / reference,
            absorbed: 100.0 * s.absorbed_weight / reference,
            detected: 100.0 * s.detected_weight / reference,
            geometric_miss: 100.0 * s.geometric_miss_weight / reference,
        })
        .collect();
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
        relative_budget_closure_error: (report.budget.accounted() - reference) / reference,
        benchmark,
        report,
    })
}

fn finish(
    record: RunOutput,
    output: Option<std::path::PathBuf>,
    exported_zmx: Option<std::path::PathBuf>,
    include_record: bool,
) -> Result<TraceResult, EngineError> {
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
    Ok(TraceResult {
        output,
        exported_zmx,
        reference_plane: record.scene.reference_plane.clone(),
        launched_rays: record.benchmark.input_rays,
        detected_percent_of_reference: 100.0 * budget.detected / record.reference_weight,
        budget_percent_of_reference: record.budget_percent_of_reference.clone(),
        detector_percent_of_reference: record.detector_percent_of_reference.clone(),
        relative_budget_closure_error: record.relative_budget_closure_error,
        median_seconds: record.trace_seconds,
        worker_threads: record.benchmark.worker_threads,
        warnings,
        record: include_record.then(|| Box::new(record)),
    })
}

/// Up to `count` sample rays taken as prefixes of every source in turn.
/// Striding a Sobol sequence would destroy its balance, so each source contributes a prefix.
fn preview_paths(
    scene: &PreparedScene,
    count: usize,
) -> Result<Vec<raytrace_core::trace::PathEvent>, EngineError> {
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
                    .expect("the range holds one ray")?;
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
    Ok(trace_rays(
        scene,
        &preview,
        TraceOptions {
            keep_terminals: false,
            path_launch_count: n as u64,
        },
    )
    .paths)
}

/// Light paths of up to 64 sample rays of a saved record, for drawing.
pub(super) fn viewer_paths(
    record: &RunRecord,
) -> Result<Vec<raytrace_core::trace::PathEvent>, EngineError> {
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
    Ok(trace_rays(
        &geometry.prepare()?,
        &rays,
        TraceOptions {
            keep_terminals: false,
            path_launch_count: rays.len() as u64,
        },
    )
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
