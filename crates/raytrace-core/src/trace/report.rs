use crate::math::V3;
use crate::model::{Interaction, PreparedScene, Ray};
use serde::{Deserialize, Serialize};

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
    pub(super) fn new(scene: &PreparedScene) -> Self {
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
    pub(super) fn merge(mut self, other: Self) -> Self {
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
    pub(super) fn terminal(
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
    pub(super) fn diagnose(
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
