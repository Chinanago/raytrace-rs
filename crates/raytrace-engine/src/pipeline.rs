//! Calculation stages shared by `trace`, `replay`, parameter scans and convergence studies.
//!
//! The flow is: configure the design and scene, check the case limits, prepare the scene,
//! resolve the launch, trace (once or timed), capture the run context and analyse the report.
//! Each stage takes typed inputs and returns typed outputs without file access; callers
//! decide what to save. Cancellation is reported as [`Completion::Cancelled`].
use crate::{Error, Result};
use raytrace_core::{
    model::{Interaction, Mode, PreparedScene, Ray, Sampling, Scene},
    screen::ScreenAnalysis,
    trace::{
        DetectorMap, Report, TraceControl, TraceOptions, trace_controlled,
        trace_generated_controlled,
    },
};
use raytrace_io::{
    design::Design,
    record::{LaunchInput, RunContext},
};
use std::{collections::BTreeMap, collections::HashSet, time::Instant};

/// Outcome of a stage that can be cancelled by the [`TraceControl`].
#[derive(Clone, Debug)]
pub enum Completion<T> {
    Done(T),
    Cancelled,
}
impl<T> Completion<T> {
    /// The finished value, or [`Error::Cancelled`].
    pub fn into_result(self) -> Result<T> {
        match self {
            Self::Done(value) => Ok(value),
            Self::Cancelled => Err(Error::Cancelled),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Configure
// ---------------------------------------------------------------------------------------

/// Switches applied to the design before its scene is compiled.
#[derive(Clone, Copy, Debug, Default)]
pub struct DesignOverrides {
    /// Trace coated surfaces as uncoated Fresnel interfaces.
    pub allow_uncoated: bool,
    /// Replace the source spectra with one wavelength in micrometres.
    pub wavelength_um: Option<f64>,
}

/// Apply [`DesignOverrides`]; omitted fields keep the design.
pub fn configure_design(design: &mut Design, overrides: DesignOverrides) {
    design.conditions.allow_uncoated |= overrides.allow_uncoated;
    if let Some(wavelength) = overrides.wavelength_um {
        design.conditions.monochromatic_override_um = Some(wavelength);
    }
}

/// Changes applied to the composed scene. Omitted fields keep the scene.
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneOverrides {
    pub mode: Option<Mode>,
    pub sampling: Option<Sampling>,
    pub rays_per_source: Option<usize>,
    /// Upper limit of rays per source, applied after `rays_per_source`.
    pub max_rays_per_source: Option<usize>,
    /// Seed of the first source; source `i` gets `seed + i` (wrapping).
    pub seed: Option<u64>,
    /// Pixel count `[x, y]` of every detector.
    pub detector_pixels: Option<[usize; 2]>,
}

/// Apply [`SceneOverrides`]. Per source: sampling, count, count limit, seed.
pub fn configure_scene(scene: &mut Scene, overrides: &SceneOverrides) {
    for (index, source) in scene.sources.iter_mut().enumerate() {
        if let Some(sampling) = overrides.sampling {
            source.sampling = sampling;
        }
        if let Some(count) = overrides.rays_per_source {
            source.count = count;
        }
        if let Some(limit) = overrides.max_rays_per_source {
            source.count = source.count.min(limit);
        }
        if let Some(seed) = overrides.seed {
            source.seed = seed.wrapping_add(index as u64);
        }
    }
    if let Some(pixels) = overrides.detector_pixels {
        for surface in &mut scene.surfaces {
            if let Interaction::Detector { pixels: grid } = &mut surface.interaction {
                *grid = pixels;
            }
        }
    }
    if let Some(mode) = overrides.mode {
        scene.mode = mode;
    }
}

// ---------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------

/// Ray total of one scan or convergence case.
pub const MAX_CASE_RAYS: usize = 2_000_000;
/// Pixels of one screen in a scan case.
pub const MAX_SCREEN_PIXELS: usize = 1_000_000;

/// Upper bounds of one calculation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Launch rays summed over all sources; zero rays is also rejected.
    pub max_rays: usize,
    /// Pixels of one screen.
    pub max_screen_pixels: Option<usize>,
}

/// Which limit a scene exceeds. Callers word the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitViolation {
    Rays { total: u128, max: usize },
    ScreenPixels { max: usize },
}

/// Launch rays summed over all sources.
pub fn total_rays(scene: &Scene) -> u128 {
    scene.sources.iter().map(|s| s.count as u128).sum()
}

/// Check the ray total first, then the screen pixel counts.
pub fn check_limits(scene: &Scene, limits: Limits) -> std::result::Result<(), LimitViolation> {
    let total = total_rays(scene);
    if total == 0 || total > limits.max_rays as u128 {
        return Err(LimitViolation::Rays {
            total,
            max: limits.max_rays,
        });
    }
    if let Some(max) = limits.max_screen_pixels
        && scene.surfaces.iter().any(|s| {
            matches!(s.interaction, Interaction::Detector { pixels }
                if pixels[0].saturating_mul(pixels[1]) > max)
        })
    {
        return Err(LimitViolation::ScreenPixels { max });
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------------------

/// Launch rays before the scene is prepared.
#[derive(Clone, Debug)]
pub enum LaunchRequest {
    /// Rays generated from the scene sources.
    Generated,
    /// Generated input of a record; sampling revision and count must still match.
    RecordedGenerated(Box<RunContext>),
    /// Rays from a request or CSV; ids and rays are validated against the prepared scene.
    Explicit(Vec<Ray>),
    /// Rays of a record, already validated by [`RunContext::rays`].
    Recorded(Vec<Ray>),
}
impl LaunchRequest {
    /// The launch of a record, validating explicit rays.
    pub fn from_record(context: &RunContext) -> Result<Self> {
        Ok(match &context.input {
            LaunchInput::Explicit { .. } => Self::Recorded(context.rays()?),
            LaunchInput::Generated { .. } => Self::RecordedGenerated(Box::new(context.clone())),
        })
    }
}

/// A validated launch: generated by the scene (`rays()` is `None`) or an explicit list.
#[derive(Clone, Debug)]
pub struct Launch {
    rays: Option<Vec<Ray>>,
    count: usize,
}
impl Launch {
    /// Trusted rays that need no validation, such as light-path previews.
    pub fn from_rays(rays: Vec<Ray>) -> Self {
        Self {
            count: rays.len(),
            rays: Some(rays),
        }
    }
    /// Number of launched rays.
    pub fn count(&self) -> usize {
        self.count
    }
    /// The explicit rays, or `None` when the scene generates them.
    pub fn rays(&self) -> Option<&[Ray]> {
        self.rays.as_deref()
    }
}

/// Validate the launch against the prepared scene and count its rays.
pub fn resolve_launch(scene: &PreparedScene, request: LaunchRequest) -> Result<Launch> {
    let rays = match request {
        LaunchRequest::Generated => None,
        LaunchRequest::Explicit(mut rays) => {
            let mut ids = HashSet::new();
            for ray in &mut rays {
                if !ids.insert(ray.id) {
                    return Err(Error::InvalidRay("発射光線IDが重複しています".into()));
                }
                ray.validate(scene)?;
            }
            Some(rays)
        }
        LaunchRequest::Recorded(rays) => Some(rays),
        LaunchRequest::RecordedGenerated(context) => {
            context.validate_generated(scene)?;
            None
        }
    };
    let count = match &rays {
        Some(rays) => {
            if !rays.iter().map(|r| r.weight).sum::<f64>().is_finite()
                || rays.iter().all(|r| r.weight == 0.0)
            {
                return Err(Error::InvalidRay(
                    "発射光線の重み合計は正の有限値にしてください".into(),
                ));
            }
            rays.len()
        }
        None => scene.ray_generator()?.len(),
    };
    if count == 0 {
        return Err(Error::InvalidInput(
            "光源または明示入力の光線を指定してください".into(),
        ));
    }
    Ok(Launch { rays, count })
}

// ---------------------------------------------------------------------------------------
// Trace
// ---------------------------------------------------------------------------------------

/// Trace the launch once on the current rayon pool.
pub fn trace_once(
    scene: &PreparedScene,
    launch: &Launch,
    options: TraceOptions,
    control: &TraceControl,
) -> Result<Completion<Report>> {
    let report = match launch.rays() {
        Some(rays) => trace_controlled(scene, rays, options, control),
        None => trace_generated_controlled(scene, options, control)?,
    };
    Ok(report.map_or(Completion::Cancelled, Completion::Done))
}

/// Warmup and repetitions of a timed trace.
#[derive(Clone, Copy, Debug)]
pub struct Schedule {
    warmup: bool,
    repeat: usize,
}
impl Schedule {
    /// One untimed trace first when `warmup`, then `repeat` timed traces (at least one).
    pub fn new(warmup: bool, repeat: usize) -> Result<Self> {
        if repeat == 0 {
            return Err(Error::InvalidInput(
                "繰返し回数は正の値で指定してください".into(),
            ));
        }
        Ok(Self { warmup, repeat })
    }
    /// A single timed trace without warmup.
    pub fn once() -> Self {
        Self {
            warmup: false,
            repeat: 1,
        }
    }
    pub fn warmup(&self) -> bool {
        self.warmup
    }
    pub fn repeat(&self) -> usize {
        self.repeat
    }
}

/// Wall-clock seconds of the timed traces, sorted ascending.
#[derive(Clone, Debug)]
pub struct Timings {
    seconds: Vec<f64>,
}
impl Timings {
    pub fn minimum(&self) -> f64 {
        self.seconds.first().copied().unwrap_or(0.0)
    }
    /// Mean of the two middle values for an even count.
    pub fn median(&self) -> f64 {
        let n = self.seconds.len();
        match n {
            0 => 0.0,
            _ if n.is_multiple_of(2) => (self.seconds[n / 2 - 1] + self.seconds[n / 2]) * 0.5,
            _ => self.seconds[n / 2],
        }
    }
}

/// The report of the last timed trace and the timings of all of them.
#[derive(Clone, Debug)]
pub struct Timed {
    pub report: Report,
    pub timings: Timings,
}

/// Trace on `pool` following `schedule`. Each timed trace includes ray generation.
pub fn run_timed(
    pool: &rayon::ThreadPool,
    scene: &PreparedScene,
    launch: &Launch,
    options: TraceOptions,
    control: &TraceControl,
    schedule: Schedule,
) -> Result<Completion<Timed>> {
    let once = || trace_once(scene, launch, options, control);
    if schedule.warmup {
        match pool.install(once)? {
            Completion::Done(report) => {
                std::hint::black_box(report);
            }
            Completion::Cancelled => return Ok(Completion::Cancelled),
        }
    }
    let mut seconds = Vec::with_capacity(schedule.repeat);
    let mut last = None;
    for _ in 0..schedule.repeat {
        let start = Instant::now();
        match pool.install(once)? {
            Completion::Done(report) => last = Some(report),
            Completion::Cancelled => return Ok(Completion::Cancelled),
        }
        seconds.push(start.elapsed().as_secs_f64());
    }
    let report =
        last.ok_or_else(|| Error::InvalidInput("繰返し回数は正の値で指定してください".into()))?;
    seconds.sort_by(f64::total_cmp);
    Ok(Completion::Done(Timed {
        report,
        timings: Timings { seconds },
    }))
}

// ---------------------------------------------------------------------------------------
// Record
// ---------------------------------------------------------------------------------------

/// Freeze the conditions of a finished trace for saving and replay.
pub fn capture(
    design: &Design,
    scene: &Scene,
    launch: &Launch,
    worker_threads: usize,
    options: TraceOptions,
) -> RunContext {
    RunContext::capture(design, scene, launch.rays(), worker_threads, options)
}

// ---------------------------------------------------------------------------------------
// Analyses
// ---------------------------------------------------------------------------------------

/// The launched weight every percentage divides by, if it is usable.
pub fn launched_power(report: &Report) -> Option<f64> {
    let power = report.budget.launched;
    (power.is_finite() && power > 0.0).then_some(power)
}

/// Each `report.budget` entry as a percentage of `reference`, by field name.
pub fn budget_percent(report: &Report, reference: f64) -> Result<BTreeMap<String, f64>> {
    let budget =
        serde_json::to_value(&report.budget).map_err(|e| raytrace_io::Error::json(None, e))?;
    Ok(budget
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        100.0 * value.as_f64().unwrap_or(0.0) / reference,
                    )
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Detected weight as a percentage of `reference`.
pub fn detected_percent(report: &Report, reference: f64) -> f64 {
    100.0 * report.budget.detected / reference
}

/// Integrated power of one detector as a percentage of `reference`.
pub fn received_percent(map: &DetectorMap, reference: f64) -> f64 {
    100.0 * map.weights.iter().sum::<f64>() / reference
}

/// Integrated power of every detector by name, as percentages of `reference`.
pub fn detector_percents(report: &Report, reference: f64) -> BTreeMap<String, f64> {
    report
        .detectors
        .iter()
        .map(|d| (d.name.clone(), received_percent(d, reference)))
        .collect()
}

/// Received power and the shape of one detector image.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectorStats {
    pub name: String,
    pub received_percent: f64,
    pub pixel_pitch_mm: [f64; 2],
    pub peak_position_mm: Option<[f64; 2]>,
    /// Full width at half maximum of the X and Y slices through the peak.
    pub fwhm_mm: [Option<f64>; 2],
}

/// [`DetectorStats`] of one detector.
pub fn detector_stats(map: &DetectorMap, reference: f64) -> Result<DetectorStats> {
    let analysis = ScreenAnalysis::from_detector(map)?;
    Ok(DetectorStats {
        name: map.name.clone(),
        received_percent: received_percent(map, reference),
        pixel_pitch_mm: analysis.pixel_pitch_mm,
        peak_position_mm: analysis.peak_position_mm,
        fwhm_mm: [
            analysis
                .x_profile
                .as_ref()
                .and_then(|p| p.half_maximum.width_mm()),
            analysis
                .y_profile
                .as_ref()
                .and_then(|p| p.half_maximum.width_mm()),
        ],
    })
}
