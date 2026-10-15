use crate::geometry::{dielectric, intersect, intersect_ordered, Hit, Miss};
use crate::math::V3;
use crate::model::{Interaction, Mode, PreparedScene, Ray};
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
        if let Some(existing) = self.diagnostics.iter_mut().find(|d| {
            d.reason == diagnostic.reason
                && d.surface == diagnostic.surface
                && d.expected_medium == diagnostic.expected_medium
                && d.actual_medium == diagnostic.actual_medium
        }) {
            existing.branch_count += diagnostic.branch_count;
            existing.weight += diagnostic.weight;
        } else if self.diagnostics.len() < MAX_DIAGNOSTIC_GROUPS {
            self.diagnostics.push(diagnostic);
        } else {
            self.diagnostic_overflow.branch_count += diagnostic.branch_count;
            self.diagnostic_overflow.weight += diagnostic.weight;
        }
    }
    fn diagnose(
        &mut self,
        ray: Ray,
        reason: &'static str,
        surface: Option<usize>,
        depth: usize,
        expected_medium: Option<usize>,
    ) {
        if let Some(existing) = self.diagnostics.iter_mut().find(|d| {
            d.reason == reason
                && d.surface == surface
                && d.expected_medium == expected_medium
                && d.actual_medium == ray.medium
        }) {
            existing.branch_count += 1;
            existing.weight += ray.weight;
            return;
        }
        if self.diagnostics.len() == MAX_DIAGNOSTIC_GROUPS {
            self.diagnostic_overflow.branch_count += 1;
            self.diagnostic_overflow.weight += ray.weight;
            return;
        }
        self.merge_diagnostic(TraceDiagnostic {
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
        });
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TraceOptions {
    pub keep_terminals: bool,
    /// Store every segment belonging to the first N launch IDs, including branches.
    pub path_launch_count: u64,
}

/// Rays must pass Ray::validate or originate from PreparedScene::generate_rays.
pub fn trace(scene: &PreparedScene, rays: &[Ray], options: TraceOptions) -> Report {
    trace_impl(scene, rays, options, None).expect("uncontrolled tracing cannot be cancelled")
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
    trace_impl(scene, rays, options, Some(control))
}

fn trace_impl(
    scene: &PreparedScene,
    rays: &[Ray],
    options: TraceOptions,
    control: Option<&TraceControl>,
) -> Option<Report> {
    // Fixed indexed chunks give a fixed reduction order independent of worker count.
    // At most 32 dense detector-map copies exist, including the final reduction.
    let chunk_size = rays.len().div_ceil(31).max(1);
    let reports: Vec<_> = rays
        .par_chunks(chunk_size)
        .map(|chunk| {
            let mut report = Report::new(scene);
            // Bound optical-data caches to ~64 KiB per chunk (at most 31 chunks).
            // Explicit rays with many distinct wavelengths fall back to direct evaluation.
            let capacity = (65536
                / (scene.spec.materials.len().max(1) * std::mem::size_of::<(f64, f64)>()))
            .clamp(1, 256);
            let mut cache = std::collections::HashMap::new();
            for ray in chunk {
                if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) {
                    return None;
                }
                let key = ray.wavelength_um.to_bits();
                if cache.len() < capacity && !cache.contains_key(&key) {
                    cache.insert(key, evaluate_optics(scene, ray.wavelength_um));
                }
                if let Some(optics) = cache.get(&key) {
                    trace_one(scene, *ray, &mut report, options, optics);
                } else {
                    trace_one(
                        scene,
                        *ray,
                        &mut report,
                        options,
                        &evaluate_optics(scene, ray.wavelength_um),
                    );
                }
                if let Some(c) = control {
                    c.completed.fetch_add(1, Ordering::Relaxed);
                }
            }
            Some(report)
        })
        .collect();
    reports
        .into_iter()
        .try_fold(Report::new(scene), |report, next| {
            next.map(|r| report.merge(r))
        })
}

#[derive(Clone, Copy)]
struct Work {
    ray: Ray,
    next_surface: usize,
    depth: usize,
}

fn evaluate_optics(scene: &PreparedScene, wavelength: f64) -> Vec<(f64, f64)> {
    // Every branch keeps its launch wavelength; evaluate dispersion and spectral
    // absorption once per material, rather than on every internal reflection.
    scene
        .spec
        .materials
        .iter()
        .map(|m| {
            (
                m.index(wavelength).expect("validated wavelength"),
                m.absorption_at(wavelength).expect("validated wavelength"),
            )
        })
        .collect()
}
fn trace_one(
    scene: &PreparedScene,
    launch: Ray,
    report: &mut Report,
    options: TraceOptions,
    optics: &[(f64, f64)],
) {
    report.budget.launched += launch.weight;
    let mut stack = vec![Work {
        ray: launch,
        next_surface: 0,
        depth: 0,
    }];
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
            report.diagnose(ray, "branch_cutoff", previous, depth, None);
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
            report.diagnose(ray, "interaction_limit", previous, depth, None);
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
            // Linear nearest-hit traversal is deliberate in v0.1. BVH is a future
            // acceleration layer and does not change interaction semantics.
            let mut nearest: Option<(usize, Hit)> = None;
            let mut numerical = None;
            for (i, s) in scene.surfaces.iter().enumerate() {
                match intersect(s, ray.position_mm, ray.direction) {
                    Ok(hit)
                        if nearest
                            .as_ref()
                            .is_none_or(|(_, h)| hit.distance_mm < h.distance_mm) =>
                    {
                        nearest = Some((i, hit))
                    }
                    Err(Miss::Numerical) => {
                        numerical.get_or_insert(i);
                    }
                    _ => (),
                }
            }
            if let Some(i) = numerical {
                Err((Miss::Numerical, Some(i)))
            } else {
                Ok(nearest)
            }
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
                report.diagnose(ray, "numerical_failure", surface, depth, None);
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
        if launch.id < options.path_launch_count && path_events < 256 {
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
        let before_bulk = ray.weight;
        ray.weight *= (-absorption * hit.distance_mm).exp();
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
                let reflectance = surface.spec.coating.as_ref().map_or(reflectance, |c| {
                    c.side(ray.direction.dot(hit.normal) > 0.0).evaluate(
                        ray.wavelength_um,
                        ray.direction.dot(hit.normal).abs(),
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
                ray.direction =
                    (ray.direction - hit.normal * (2.0 * ray.direction.dot(hit.normal))).unit();
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
                let (expected, after) = if ray.direction.dot(hit.normal) > 0.0 {
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
                let result = dielectric(ray.direction, hit.normal, n_before, n_after);
                let r = if result.transmitted.is_none() {
                    1.0
                } else {
                    reflectance.unwrap_or(result.reflectance)
                };
                let reflected_weight = ray.weight * r;
                let (reflected_weight, transmitted_weight) = if let Some(c) = &surface.spec.coating
                {
                    let [r, t] = c.side(ray.direction.dot(hit.normal) > 0.0).evaluate(
                        ray.wavelength_um,
                        ray.direction.dot(hit.normal).abs(),
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
                if scene.spec.mode == Mode::NonSequential || result.transmitted.is_none() {
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
