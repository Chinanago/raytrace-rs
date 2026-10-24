use crate::acceleration::SurfaceIndex;
use crate::geometry::{dielectric_with_reflectance, intersect_ordered, Miss};
use crate::math::V3;
use crate::model::{Interaction, Mode, PreparedScene, Ray};
use crate::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
pub struct TraceControl {
    pub cancel: AtomicBool,
    pub completed: AtomicUsize,
    pub total: AtomicUsize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Budget {
    pub launched: f64,
    pub detected: f64,
    pub escaped: f64,
    pub geometric_miss: f64,
    pub surface_absorbed: f64,
    pub bulk_absorbed: f64,
    pub sequential_reflection_not_followed: f64,
    #[serde(default)]
    pub sequential_tir: f64,
    pub branch_cutoff: f64,
    pub interaction_limit: f64,
    pub numerical_failure: f64,
    pub medium_mismatch: f64,
}
impl Budget {
    pub fn accounted(&self) -> f64 {
        self.detected
            + self.escaped
            + self.geometric_miss
            + self.surface_absorbed
            + self.bulk_absorbed
            + self.sequential_reflection_not_followed
            + self.sequential_tir
            + self.branch_cutoff
            + self.interaction_limit
            + self.numerical_failure
            + self.medium_mismatch
    }
    pub fn merge(&mut self, b: &Self) {
        self.launched += b.launched;
        self.detected += b.detected;
        self.escaped += b.escaped;
        self.geometric_miss += b.geometric_miss;
        self.surface_absorbed += b.surface_absorbed;
        self.bulk_absorbed += b.bulk_absorbed;
        self.sequential_reflection_not_followed += b.sequential_reflection_not_followed;
        self.sequential_tir += b.sequential_tir;
        self.branch_cutoff += b.branch_cutoff;
        self.interaction_limit += b.interaction_limit;
        self.numerical_failure += b.numerical_failure;
        self.medium_mismatch += b.medium_mismatch;
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SurfaceStats {
    pub name: String,
    pub encounters: u64,
    pub incident_weight: f64,
    pub transmitted_weight: f64,
    pub reflected_weight: f64,
    pub absorbed_weight: f64,
    pub detected_weight: f64,
    pub geometric_miss_weight: f64,
    #[serde(default, skip_serializing_if = "zero")]
    pub normal_fallbacks: u64,
}
fn zero(n: &u64) -> bool {
    *n == 0
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectorMap {
    pub name: String,
    pub pixels: [usize; 2],
    pub half_extent_mm: [f64; 2],
    /// Row-major: row y, column x. Values are integrated pixel power, not density.
    pub weights: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Terminal {
    pub ray: Ray,
    pub reason: String,
    pub surface: Option<usize>,
    pub depth: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathEvent {
    pub id: u64,
    pub depth: usize,
    pub surface: usize,
    pub start_mm: V3,
    pub end_mm: V3,
    pub incident_weight: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceDiagnostic {
    pub reason: String,
    pub surface: Option<usize>,
    pub expected_medium: Option<usize>,
    pub actual_medium: usize,
    /// Counts terminal branches, not unique launch IDs.
    pub branch_count: u64,
    pub weight: f64,
    pub example: Terminal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intersection: Option<IntersectionContext>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntersectionContext {
    /// Position from which the failed interface search started.
    pub origin_mm: V3,
    pub distance_mm: f64,
    pub normal: V3,
    #[serde(default)]
    pub geometric_normal: V3,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiagnosticOverflow {
    pub branch_count: u64,
    pub weight: f64,
}
const MAX_DIAGNOSTIC_GROUPS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub budget: Budget,
    pub surfaces: Vec<SurfaceStats>,
    pub detectors: Vec<DetectorMap>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<Terminal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathEvent>,
    pub surface_intersections: u64,
    pub total_internal_reflections: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<TraceDiagnostic>,
    #[serde(default)]
    pub diagnostic_overflow: DiagnosticOverflow,
}
impl Report {
    fn new(scene: &PreparedScene) -> Self {
        Self {
            budget: Budget::default(),
            surfaces: scene
                .surfaces
                .iter()
                .map(|s| SurfaceStats {
                    name: s.spec.name.clone(),
                    ..SurfaceStats::default()
                })
                .collect(),
            detectors: scene
                .detector_surfaces
                .iter()
                .map(|&i| {
                    let s = &scene.surfaces[i].spec;
                    let Interaction::Detector { pixels } = s.interaction else {
                        unreachable!()
                    };
                    let (hx, hy) = s.aperture.bounds();
                    DetectorMap {
                        name: s.name.clone(),
                        pixels,
                        half_extent_mm: [hx, hy],
                        weights: vec![0.0; pixels[0] * pixels[1]],
                    }
                })
                .collect(),
            terminals: Vec::new(),
            paths: Vec::new(),
            surface_intersections: 0,
            total_internal_reflections: 0,
            diagnostics: Vec::new(),
            diagnostic_overflow: DiagnosticOverflow::default(),
        }
    }
    fn merge(mut self, other: Self) -> Self {
        self.budget.merge(&other.budget);
        self.surface_intersections += other.surface_intersections;
        self.total_internal_reflections += other.total_internal_reflections;
        for (a, b) in self.surfaces.iter_mut().zip(other.surfaces) {
            a.encounters += b.encounters;
            a.incident_weight += b.incident_weight;
            a.transmitted_weight += b.transmitted_weight;
            a.reflected_weight += b.reflected_weight;
            a.absorbed_weight += b.absorbed_weight;
            a.detected_weight += b.detected_weight;
            a.geometric_miss_weight += b.geometric_miss_weight;
            a.normal_fallbacks += b.normal_fallbacks;
        }
        for (a, b) in self.detectors.iter_mut().zip(other.detectors) {
            for (wa, wb) in a.weights.iter_mut().zip(b.weights) {
                *wa += wb;
            }
        }
        self.terminals.extend(other.terminals);
        self.paths.extend(other.paths);
        self.diagnostic_overflow.branch_count += other.diagnostic_overflow.branch_count;
        self.diagnostic_overflow.weight += other.diagnostic_overflow.weight;
        for d in other.diagnostics {
            self.merge_diagnostic(d);
        }
        self
    }
    fn terminal(
        &mut self,
        ray: Ray,
        reason: &'static str,
        surface: Option<usize>,
        depth: usize,
        keep: bool,
    ) {
        if keep {
            self.terminals.push(Terminal {
                ray,
                reason: reason.into(),
                surface,
                depth,
            });
        }
    }
    fn merge_diagnostic(&mut self, diagnostic: TraceDiagnostic) {
        let reason = diagnostic.reason.clone();
        let key = (
            reason.as_str(),
            diagnostic.surface,
            diagnostic.expected_medium,
            diagnostic.actual_medium,
        );
        self.group_diagnostic(key, diagnostic.branch_count, diagnostic.weight, || {
            diagnostic
        });
    }
    fn group_diagnostic(
        &mut self,
        key: (&str, Option<usize>, Option<usize>, usize),
        count: u64,
        weight: f64,
        make: impl FnOnce() -> TraceDiagnostic,
    ) {
        if let Some(existing) = self.diagnostics.iter_mut().find(|d| {
            d.reason == key.0
                && d.surface == key.1
                && d.expected_medium == key.2
                && d.actual_medium == key.3
        }) {
            existing.branch_count += count;
            existing.weight += weight;
        } else if self.diagnostics.len() < MAX_DIAGNOSTIC_GROUPS {
            self.diagnostics.push(make());
        } else {
            self.diagnostic_overflow.branch_count += count;
            self.diagnostic_overflow.weight += weight;
        }
    }
    fn diagnose(
        &mut self,
        ray: Ray,
        reason: &'static str,
        surface: Option<usize>,
        depth: usize,
        expected_medium: Option<usize>,
        intersection: Option<IntersectionContext>,
    ) {
        self.group_diagnostic(
            (reason, surface, expected_medium, ray.medium),
            1,
            ray.weight,
            || TraceDiagnostic {
                intersection,
                reason: reason.into(),
                surface,
                expected_medium,
                actual_medium: ray.medium,
                branch_count: 1,
                weight: ray.weight,
                example: Terminal {
                    ray,
                    reason: reason.into(),
                    surface,
                    depth,
                },
            },
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TraceOptions {
    pub keep_terminals: bool,
    /// Store every segment belonging to the first N launch rays in input order, including branches.
    pub path_launch_count: u64,
}

/// Rays must pass Ray::validate or originate from PreparedScene::generate_rays.
pub fn trace(scene: &PreparedScene, rays: &[Ray], options: TraceOptions) -> Report {
    trace_impl(
        scene,
        rays.len(),
        |range| rays[range].iter().copied().map(Ok),
        options,
        None,
    )
    .expect("explicit rays cannot fail generation")
    .expect("uncontrolled tracing cannot be cancelled")
}

/// A cancelled calculation returns no partial report; callers retain completed runs.
/// Progress advances after complete launch rays, including all their branches.
pub fn trace_controlled(
    scene: &PreparedScene,
    rays: &[Ray],
    options: TraceOptions,
    control: &TraceControl,
) -> Option<Report> {
    control.completed.store(0, Ordering::Relaxed);
    control.total.store(rays.len(), Ordering::Relaxed);
    trace_impl(
        scene,
        rays.len(),
        |range| rays[range].iter().copied().map(Ok),
        options,
        Some(control),
    )
    .expect("explicit rays cannot fail generation")
}

/// Trace source rays without retaining the launch array.
pub fn trace_generated(scene: &PreparedScene, options: TraceOptions) -> Result<Report> {
    generated_impl(scene, options, None)
        .map(|r| r.expect("uncontrolled tracing cannot be cancelled"))
}

pub fn trace_generated_controlled(
    scene: &PreparedScene,
    options: TraceOptions,
    control: &TraceControl,
) -> Result<Option<Report>> {
    generated_impl(scene, options, Some(control))
}

fn generated_impl(
    scene: &PreparedScene,
    options: TraceOptions,
    control: Option<&TraceControl>,
) -> Result<Option<Report>> {
    let generator = scene.ray_generator()?;
    if let Some(c) = control {
        c.completed.store(0, Ordering::Relaxed);
        c.total.store(generator.len(), Ordering::Relaxed);
        if c.cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
    }
    trace_impl(
        scene,
        generator.len(),
        |range| generator.rays(range),
        options,
        control,
    )
}

fn trace_impl<I: Iterator<Item = Result<Ray>>>(
    scene: &PreparedScene,
    len: usize,
    rays: impl Fn(std::ops::Range<usize>) -> I + Sync,
    options: TraceOptions,
    control: Option<&TraceControl>,
) -> Result<Option<Report>> {
    let surface_index = (scene.spec.mode == Mode::NonSequential)
        .then(|| SurfaceIndex::new(&scene.surfaces))
        .flatten();
    // Fixed indexed chunks give a fixed reduction order independent of worker count.
    // At most 32 dense detector-map copies exist, including the final reduction.
    let chunk_size = len.div_ceil(31).max(1);
    let reports: Vec<_> = (0..len.div_ceil(chunk_size))
        .into_par_iter()
        .map(|chunk_index| -> Result<Option<Report>> {
            let start = chunk_index * chunk_size;
            let chunk = rays(start..start.saturating_add(chunk_size).min(len));
            let mut report = Report::new(scene);
            let mut cache = OpticalCache::new(scene);
            let mut traversal = Traversal {
                stack: WorkQueue::new(scene.spec.mode),
                surface_index: surface_index.as_ref(),
            };
            let mut completed = 0;
            for (position, ray) in chunk.enumerate() {
                let record_paths =
                    ((chunk_index * chunk_size + position) as u64) < options.path_launch_count;
                if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) {
                    return Ok(None);
                }
                let ray = ray?;
                let optics = cache.get(scene, ray.wavelength_um);
                trace_one(
                    scene,
                    ray,
                    &mut report,
                    options,
                    record_paths,
                    optics,
                    &mut traversal,
                );
                if let Some(c) = control {
                    completed += 1;
                    if completed == 256 {
                        c.completed.fetch_add(completed, Ordering::Relaxed);
                        completed = 0;
                    }
                }
            }
            if let Some(c) = control {
                c.completed.fetch_add(completed, Ordering::Relaxed);
            }
            Ok(Some(report))
        })
        .collect();
    let mut report = Some(Report::new(scene));
    // Inspect every chunk in launch order so the earliest generation error wins.
    for next in reports {
        let next = next?;
        report = report.zip(next).map(|(a, b)| a.merge(b));
    }
    Ok(report)
}

#[derive(Clone, Copy)]
struct Work {
    ray: Ray,
    next_surface: usize,
    depth: usize,
}
struct Traversal<'a> {
    stack: WorkQueue,
    surface_index: Option<&'a SurfaceIndex>,
}
struct WeightedWork {
    work: Work,
    order: u64,
}
impl PartialEq for WeightedWork {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for WeightedWork {}
impl PartialOrd for WeightedWork {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for WeightedWork {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.work
            .ray
            .weight
            .total_cmp(&other.work.ray.weight)
            .then(self.order.cmp(&other.order))
    }
}
enum WorkQueue {
    Sequential(Option<Work>),
    NonSequential(std::collections::BinaryHeap<WeightedWork>, u64),
}
impl WorkQueue {
    fn new(mode: Mode) -> Self {
        if mode == Mode::Sequential {
            Self::Sequential(None)
        } else {
            Self::NonSequential(std::collections::BinaryHeap::new(), 0)
        }
    }
    fn start(&mut self, first: Work) {
        if let Self::NonSequential(q, order) = self {
            debug_assert!(q.is_empty());
            *order = 0;
        }
        self.push(first);
    }
    fn push(&mut self, work: Work) {
        match self {
            Self::Sequential(q) => {
                debug_assert!(q.is_none());
                *q = Some(work);
            }
            Self::NonSequential(q, order) => {
                q.push(WeightedWork {
                    work,
                    order: *order,
                });
                *order += 1;
            }
        }
    }
    fn pop(&mut self) -> Option<Work> {
        match self {
            Self::Sequential(q) => q.take(),
            Self::NonSequential(q, _) => q.pop().map(|w| w.work),
        }
    }
}

struct OpticalCache {
    primary_key: u64,
    primary: Vec<(f64, f64)>,
    other: std::collections::HashMap<u64, Vec<(f64, f64)>>,
    uncached: Vec<(f64, f64)>,
    capacity: usize,
}
impl OpticalCache {
    fn new(scene: &PreparedScene) -> Self {
        Self {
            // Positive finite wavelengths cannot have the all-zero bit pattern.
            primary_key: 0,
            primary: Vec::new(),
            other: std::collections::HashMap::new(),
            uncached: Vec::new(),
            // Bound cached material data to ~64 KiB per chunk, up to 256 wavelengths.
            capacity: (65536
                / (scene.spec.materials.len().max(1) * std::mem::size_of::<(f64, f64)>()))
            .clamp(1, 256),
        }
    }
    fn get(&mut self, scene: &PreparedScene, wavelength: f64) -> &[(f64, f64)] {
        let key = wavelength.to_bits();
        if key == self.primary_key {
            return &self.primary;
        }
        if self.primary.is_empty() {
            self.primary_key = key;
            self.primary = evaluate_optics(scene, wavelength).collect();
            return &self.primary;
        }
        if self.other.len() + 1 < self.capacity {
            self.other
                .entry(key)
                .or_insert_with(|| evaluate_optics(scene, wavelength).collect())
        } else if let Some(optics) = self.other.get(&key) {
            optics
        } else {
            // Preserve exact wavelength evaluation beyond the cache budget; reuse storage.
            self.uncached.clear();
            self.uncached.extend(evaluate_optics(scene, wavelength));
            &self.uncached
        }
    }
}

fn evaluate_optics(
    scene: &PreparedScene,
    wavelength: f64,
) -> impl Iterator<Item = (f64, f64)> + '_ {
    // Every branch keeps its launch wavelength; evaluate dispersion and spectral
    // absorption once per material, rather than on every internal reflection.
    scene.spec.materials.iter().map(move |m| {
        (
            m.index(wavelength).expect("validated wavelength"),
            m.absorption_at(wavelength).expect("validated wavelength"),
        )
    })
}
fn trace_one(
    scene: &PreparedScene,
    launch: Ray,
    report: &mut Report,
    options: TraceOptions,
    record_paths: bool,
    optics: &[(f64, f64)],
    traversal: &mut Traversal<'_>,
) {
    let stack = &mut traversal.stack;
    report.budget.launched += launch.weight;
    // Resolve the strongest pending NSC branches first. A launch-wide event
    // allowance must not strand a strong reflection behind many weak descendants.
    stack.start(Work {
        ray: launch,
        next_surface: 0,
        depth: 0,
    });
    let mut events = 0;
    let mut path_events = 0;
    while let Some(work) = stack.pop() {
        let Work {
            mut ray,
            next_surface,
            depth,
        } = work;
        if ray.weight == 0.0 {
            continue;
        }
        if ray.weight < launch.weight * scene.spec.settings.min_relative_branch_weight {
            report.budget.branch_cutoff += ray.weight;
            let previous = next_surface.checked_sub(1);
            report.diagnose(ray, "branch_cutoff", previous, depth, None, None);
            report.terminal(
                ray,
                "branch_cutoff",
                previous,
                depth,
                options.keep_terminals,
            );
            continue;
        }
        if depth >= scene.spec.settings.max_depth
            || events >= scene.spec.settings.max_events_per_launch
        {
            report.budget.interaction_limit += ray.weight;
            let previous = next_surface.checked_sub(1);
            report.diagnose(ray, "interaction_limit", previous, depth, None, None);
            report.terminal(
                ray,
                "interaction_limit",
                previous,
                depth,
                options.keep_terminals,
            );
            continue;
        }
        let selection = if scene.spec.mode == Mode::Sequential {
            match scene.surfaces.get(next_surface) {
                Some(s) => intersect_ordered(s, ray.position_mm, ray.direction)
                    .map(|h| Some((next_surface, h)))
                    .map_err(|e| (e, Some(next_surface))),
                None => Ok(None),
            }
        } else {
            crate::acceleration::nearest(
                &scene.surfaces,
                traversal.surface_index,
                ray.position_mm,
                ray.direction,
            )
        };
        let (index, hit) = match selection {
            Ok(Some(value)) => value,
            Ok(None) => {
                report.budget.escaped += ray.weight;
                report.terminal(ray, "escaped", None, depth, options.keep_terminals);
                continue;
            }
            Err((Miss::Geometry, _)) => {
                report.budget.geometric_miss += ray.weight;
                report.surfaces[next_surface].geometric_miss_weight += ray.weight;
                report.terminal(
                    ray,
                    "geometric_miss",
                    Some(next_surface),
                    depth,
                    options.keep_terminals,
                );
                continue;
            }
            Err((Miss::Numerical, surface)) => {
                report.budget.numerical_failure += ray.weight;
                report.diagnose(ray, "numerical_failure", surface, depth, None, None);
                report.terminal(
                    ray,
                    "numerical_failure",
                    surface,
                    depth,
                    options.keep_terminals,
                );
                continue;
            }
        };
        events += 1;
        report.surface_intersections += 1;
        if record_paths && path_events < 256 {
            report.paths.push(PathEvent {
                id: ray.id,
                depth,
                surface: index,
                start_mm: ray.position_mm,
                end_mm: hit.point,
                incident_weight: ray.weight,
            });
            path_events += 1;
        }
        let (n_before, absorption) = optics[ray.medium];
        let origin_mm = ray.position_mm;
        let before_bulk = ray.weight;
        if absorption != 0.0 {
            ray.weight *= (-absorption * hit.distance_mm).exp();
        }
        report.budget.bulk_absorbed += before_bulk - ray.weight;
        ray.optical_path_mm += n_before * hit.distance_mm;
        ray.position_mm = hit.point;
        let surface = &scene.surfaces[index];
        report.surfaces[index].encounters += 1;
        report.surfaces[index].incident_weight += ray.weight;
        match surface.spec.interaction {
            Interaction::Absorb => {
                report.budget.surface_absorbed += ray.weight;
                report.surfaces[index].absorbed_weight += ray.weight;
                report.terminal(
                    ray,
                    "absorbed",
                    Some(index),
                    depth + 1,
                    options.keep_terminals,
                );
            }
            Interaction::Detector { pixels } => {
                let detector = &mut report.detectors[surface.detector_index.unwrap()];
                let [hx, hy] = detector.half_extent_mm;
                let x = (((hit.local_point.0[0] / hx + 1.0) * 0.5 * pixels[0] as f64) as usize)
                    .min(pixels[0] - 1);
                let y = (((hit.local_point.0[1] / hy + 1.0) * 0.5 * pixels[1] as f64) as usize)
                    .min(pixels[1] - 1);
                detector.weights[y * pixels[0] + x] += ray.weight;
                report.budget.detected += ray.weight;
                report.surfaces[index].detected_weight += ray.weight;
                report.terminal(
                    ray,
                    "detected",
                    Some(index),
                    depth + 1,
                    options.keep_terminals,
                );
            }
            Interaction::Mirror { reflectance } => {
                let mut normal = hit.normal;
                if !normal.finite()
                    || ray.direction.dot(normal) * ray.direction.dot(hit.geometric_normal) < 0.
                    || (ray.direction.dot(hit.geometric_normal)
                        - 2. * ray.direction.dot(normal) * normal.dot(hit.geometric_normal))
                        * ray.direction.dot(hit.geometric_normal)
                        > 1e-14
                {
                    normal = hit.geometric_normal;
                    report.surfaces[index].normal_fallbacks += 1;
                }
                let reflectance = surface.spec.coating.as_ref().map_or(reflectance, |c| {
                    c.side(ray.direction.dot(hit.geometric_normal) > 0.0)
                        .evaluate(
                            ray.wavelength_um,
                            ray.direction.dot(normal).abs(),
                            n_before,
                            n_before,
                            false,
                        )[0]
                });
                let absorbed = ray.weight * (1.0 - reflectance);
                report.budget.surface_absorbed += absorbed;
                report.surfaces[index].absorbed_weight += absorbed;
                report.surfaces[index].reflected_weight += ray.weight * reflectance;
                ray.weight *= reflectance;
                ray.direction = (ray.direction - normal * (2.0 * ray.direction.dot(normal))).unit();
                stack.push(Work {
                    ray,
                    next_surface: index + 1,
                    depth: depth + 1,
                });
            }
            Interaction::Dielectric {
                minus_medium,
                plus_medium,
                reflectance,
            } => {
                let (expected, after) = if ray.direction.dot(hit.geometric_normal) > 0.0 {
                    (minus_medium, plus_medium)
                } else {
                    (plus_medium, minus_medium)
                };
                if ray.medium != expected {
                    report.budget.medium_mismatch += ray.weight;
                    report.diagnose(
                        ray,
                        "medium_mismatch",
                        Some(index),
                        depth + 1,
                        Some(expected),
                        Some(IntersectionContext {
                            origin_mm,
                            distance_mm: hit.distance_mm,
                            normal: hit.normal,
                            geometric_normal: hit.geometric_normal,
                        }),
                    );
                    report.terminal(
                        ray,
                        "medium_mismatch",
                        Some(index),
                        depth + 1,
                        options.keep_terminals,
                    );
                    continue;
                }
                let n_after = optics[after].0;
                let mut normal = hit.normal;
                let bare_power = if surface.spec.coating.is_some() {
                    Some(0.0)
                } else {
                    reflectance
                };
                let mut result = dielectric_with_reflectance(
                    ray.direction,
                    normal,
                    n_before,
                    n_after,
                    bare_power,
                );
                let incoming = ray.direction.dot(hit.geometric_normal);
                if !normal.finite()
                    || ray.direction.dot(normal) * incoming < 0.
                    || result.reflected.dot(hit.geometric_normal) * incoming > 1e-14
                    || result
                        .transmitted
                        .is_some_and(|t| t.dot(hit.geometric_normal) * incoming < -1e-14)
                {
                    normal = hit.geometric_normal;
                    result = dielectric_with_reflectance(
                        ray.direction,
                        normal,
                        n_before,
                        n_after,
                        bare_power,
                    );
                    report.surfaces[index].normal_fallbacks += 1;
                }
                let r = if result.transmitted.is_none() {
                    1.0
                } else {
                    reflectance.unwrap_or(result.reflectance)
                };
                let reflected_weight = ray.weight * r;
                let (reflected_weight, transmitted_weight) = if let Some(c) = &surface.spec.coating
                {
                    let [r, t] = c.side(incoming > 0.0).evaluate(
                        ray.wavelength_um,
                        ray.direction.dot(normal).abs(),
                        n_before,
                        n_after,
                        result.transmitted.is_none(),
                    );
                    let rw = ray.weight * r;
                    let tw = (ray.weight * t).min(ray.weight - rw);
                    let absorbed = (ray.weight - rw - tw).max(0.0);
                    report.budget.surface_absorbed += absorbed;
                    report.surfaces[index].absorbed_weight += absorbed;
                    (rw, tw)
                } else {
                    (reflected_weight, ray.weight - reflected_weight)
                };
                report.surfaces[index].reflected_weight += reflected_weight;
                report.surfaces[index].transmitted_weight += transmitted_weight;
                if result.transmitted.is_none() {
                    report.total_internal_reflections += 1;
                }
                if scene.spec.mode == Mode::Sequential && result.transmitted.is_none() {
                    ray.weight = reflected_weight;
                    report.budget.sequential_tir += ray.weight;
                    report.diagnose(ray, "sequential_tir", Some(index), depth + 1, None, None);
                    report.terminal(
                        ray,
                        "sequential_tir",
                        Some(index),
                        depth + 1,
                        options.keep_terminals,
                    );
                } else if scene.spec.mode == Mode::NonSequential {
                    if reflected_weight > 0.0 {
                        stack.push(Work {
                            ray: Ray {
                                direction: result.reflected,
                                weight: reflected_weight,
                                ..ray
                            },
                            next_surface: index + 1,
                            depth: depth + 1,
                        });
                    }
                } else {
                    report.budget.sequential_reflection_not_followed += reflected_weight;
                }
                if let Some(direction) = result.transmitted {
                    if transmitted_weight > 0.0 {
                        stack.push(Work {
                            ray: Ray {
                                direction,
                                medium: after,
                                weight: transmitted_weight,
                                ..ray
                            },
                            next_surface: index + 1,
                            depth: depth + 1,
                        });
                    }
                }
            }
        }
    }
}
