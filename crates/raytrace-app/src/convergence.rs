//! Controlled ray-count, seed, pixel-grid and CAD-tessellation comparisons.
use raytrace_core::{
    model::{Interaction, Shape},
    screen::ScreenAnalysis,
    trace::{trace_generated, DetectorMap, TraceOptions},
};
use raytrace_io::{project::Project, record::RunContext};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, time::Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConvergencePlan {
    pub ray_counts: Vec<usize>,
    pub seeds: Vec<u64>,
    pub pixels: Vec<[usize; 2]>,
    pub cad_deflections_mm: Vec<f64>,
    pub threads: usize,
    pub allow_uncoated: bool,
    pub wavelength_um: Option<f64>,
    pub save_runs: bool,
}
impl Default for ConvergencePlan {
    fn default() -> Self {
        Self {
            ray_counts: vec![256, 1024, 4096],
            seeds: vec![42, 43],
            pixels: vec![[64, 64]],
            cad_deflections_mm: vec![0.02],
            threads: 4,
            allow_uncoated: false,
            wavelength_um: None,
            save_runs: true,
        }
    }
}
impl ConvergencePlan {
    fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.threads)
            || self.ray_counts.iter().any(|&n| n == 0 || n > 2_000_000)
            || self
                .pixels
                .iter()
                .any(|p| p.contains(&0) || p[0].checked_mul(p[1]).is_none_or(|n| n > 1_000_000))
            || self.cad_deflections_mm.is_empty()
            || self
                .cad_deflections_mm
                .iter()
                .any(|d| !d.is_finite() || *d <= 0.0)
            || self
                .wavelength_um
                .is_some_and(|w| !w.is_finite() || w <= 0.0)
        {
            return Err("収束確認の光線数・画素数・CAD精度・波長・スレッド数が不正です".into());
        }
        let n = self
            .ray_counts
            .len()
            .max(1)
            .checked_mul(self.seeds.len().max(1))
            .and_then(|n| n.checked_mul(self.pixels.len().max(1)))
            .and_then(|n| n.checked_mul(self.cad_deflections_mm.len()));
        if n.is_none_or(|n| n > 256) {
            return Err("収束確認は256ケースまでです".into());
        }
        fn duplicates<T: PartialEq>(v: &[T]) -> bool {
            v.iter().enumerate().any(|(i, x)| v[..i].contains(x))
        }
        if duplicates(&self.ray_counts)
            || duplicates(&self.seeds)
            || duplicates(&self.pixels)
            || duplicates(&self.cad_deflections_mm)
        {
            return Err("収束確認の軸に重複した値があります".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseSettings {
    pub rays_per_source: Option<usize>,
    /// First source seed; later sources use consecutive seeds. None preserves originals.
    pub seed: Option<u64>,
    pub pixels: Option<[usize; 2]>,
    pub cad_deflection_mm: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectorSummary {
    pub name: String,
    pub received_percent: f64,
    pub pixel_pitch_mm: [f64; 2],
    pub fwhm_mm: [Option<f64>; 2],
    pub peak_position_mm: Option<[f64; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvergenceCase {
    pub index: usize,
    pub settings: CaseSettings,
    pub error: Option<String>,
    pub rays: usize,
    pub trace_seconds: f64,
    pub reference_plane: String,
    pub reference_weight: f64,
    pub detected_percent: f64,
    pub unresolved_percent: f64,
    pub budget_percent_of_reference: std::collections::BTreeMap<String, f64>,
    pub triangles: usize,
    pub cad_applied: bool,
    pub detectors: Vec<DetectorSummary>,
    pub run_file: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectorDifference {
    pub name: String,
    pub received_delta_percentage_points: f64,
    pub fwhm_delta_mm: [Option<f64>; 2],
    pub distribution_l1_percent_of_reference: f64,
    pub comparison_pixels: [usize; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvergenceComparison {
    pub axis: String,
    pub case: usize,
    pub reference_case: usize,
    pub detectors: Vec<DetectorDifference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvergenceReport {
    pub format: String,
    pub software_version: String,
    pub model: String,
    pub plan: ConvergencePlan,
    pub cases: Vec<ConvergenceCase>,
    pub comparisons: Vec<ConvergenceComparison>,
    pub distribution_definition: String,
    pub timing_definition: String,
}

/// Area-overlap rebinning conserves integrated pixel power. Values are uniform
/// within each original pixel for this comparison; no extra detail is inferred.
pub fn rebin(map: &DetectorMap, pixels: [usize; 2]) -> Result<DetectorMap, String> {
    ScreenAnalysis::from_detector(map)?;
    if pixels.contains(&0) || pixels[0] > map.pixels[0] || pixels[1] > map.pixels[1] {
        return Err("比較画素は元の画素数以下で指定してください".into());
    }
    let [nx, ny] = map.pixels;
    let mut weights = vec![0.0; pixels[0] * pixels[1]];
    for y in 0..ny {
        let ya = y as f64 * pixels[1] as f64 / ny as f64;
        let yb = (y + 1) as f64 * pixels[1] as f64 / ny as f64;
        for x in 0..nx {
            let xa = x as f64 * pixels[0] as f64 / nx as f64;
            let xb = (x + 1) as f64 * pixels[0] as f64 / nx as f64;
            let w = map.weights[y * nx + x];
            for j in ya.floor() as usize..(yb.ceil() as usize).min(pixels[1]) {
                let fy = (yb.min((j + 1) as f64) - ya.max(j as f64)).max(0.0) / (yb - ya);
                for i in xa.floor() as usize..(xb.ceil() as usize).min(pixels[0]) {
                    let fx = (xb.min((i + 1) as f64) - xa.max(i as f64)).max(0.0) / (xb - xa);
                    weights[j * pixels[0] + i] += w * fx * fy;
                }
            }
        }
    }
    Ok(DetectorMap {
        name: map.name.clone(),
        pixels,
        half_extent_mm: map.half_extent_mm,
        weights,
    })
}

fn axis_options<T: Copy>(values: &[T]) -> Vec<Option<T>> {
    if values.is_empty() {
        vec![None]
    } else {
        values.iter().copied().map(Some).collect()
    }
}

pub fn run(
    model: &Path,
    plan: &ConvergencePlan,
    output: &Path,
) -> Result<ConvergenceReport, String> {
    plan.validate()?;
    let native = model
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zmx"));
    if !native && plan.cad_deflections_mm.len() > 1 {
        return Err("CAD分割精度の比較には、STEPを参照する元のZMXを指定してください。JSON内のメッシュは保存済みの分割を使います".into());
    }
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(plan.threads)
        .build()
        .map_err(|e| e.to_string())?;
    let mut summary = ConvergenceReport {
        format: "raytrace-convergence-v1".into(), software_version: env!("CARGO_PKG_VERSION").into(),
        model: model.display().to_string(), plan: plan.clone(), cases: Vec::new(), comparisons: Vec::new(),
        distribution_definition: "L1 sum of differences in integrated pixel power / each launch reference. Area-overlap rebinning to a common grid of at most 64x64, assuming uniform power within original pixels. Reference is the largest ray/pixel count, smallest CAD deflection, or first seed, with other axes fixed. This is a sensitivity comparison, not an automatic convergence certificate.".into(),
        timing_definition: "One trace per case, including report accumulation; excludes model loading, tessellation, preparation, source generation and file output. No benchmark warmup or median is applied.".into(),
    };
    let mut maps: Vec<Vec<DetectorMap>> = Vec::new();
    let pixel_options = axis_options(&plan.pixels);
    for &cad in &plan.cad_deflections_mm {
        let loaded = Project::open_with_cad_tolerance(model, &[], cad);
        for count in axis_options(&plan.ray_counts) {
            for seed in axis_options(&plan.seeds) {
                for &pixels in &pixel_options {
                    let index = summary.cases.len();
                    let settings = CaseSettings {
                        rays_per_source: count,
                        seed,
                        pixels,
                        cad_deflection_mm: cad,
                    };
                    let mut row = ConvergenceCase {
                        index,
                        settings,
                        error: None,
                        rays: 0,
                        trace_seconds: 0.0,
                        reference_plane: String::new(),
                        reference_weight: 0.0,
                        detected_percent: 0.0,
                        unresolved_percent: 0.0,
                        budget_percent_of_reference: Default::default(),
                        triangles: 0,
                        cad_applied: false,
                        detectors: Vec::new(),
                        run_file: None,
                    };
                    let result = (|| -> Result<Vec<DetectorMap>, String> {
                        let mut project = loaded.as_ref().map_err(Clone::clone)?.clone();
                        project.allow_uncoated |= plan.allow_uncoated;
                        if plan.wavelength_um.is_some() {
                            project.monochromatic_override_um = plan.wavelength_um;
                        }
                        let mut scene = project.trace_scene()?;
                        for (i, source) in scene.sources.iter_mut().enumerate() {
                            if let Some(n) = count {
                                source.count = n;
                            }
                            if let Some(s) = seed {
                                source.seed = s.wrapping_add(i as u64);
                            }
                        }
                        for surface in &mut scene.surfaces {
                            if let Interaction::Detector { pixels: ref mut p } = surface.interaction
                            {
                                if let Some(pixels) = pixels {
                                    *p = pixels;
                                }
                            }
                            if let Shape::TriangleMesh { triangles, .. } = &surface.shape {
                                row.triangles += triangles.len();
                            }
                        }
                        row.cad_applied = native && project.cad_deflection_mm.is_some();
                        let total: u128 = scene.sources.iter().map(|s| s.count as u128).sum();
                        if total == 0 || total > 2_000_000 {
                            return Err(
                                "各ケースの光線数合計は1〜2,000,000本で指定してください".into()
                            );
                        }
                        let prepared = scene.clone().prepare()?;
                        let started = Instant::now();
                        let report =
                            pool.install(|| trace_generated(&prepared, TraceOptions::default()))?;
                        row.trace_seconds = started.elapsed().as_secs_f64();
                        row.rays = total as usize;
                        row.reference_weight = report.budget.launched;
                        let budget =
                            serde_json::to_value(&report.budget).map_err(|e| e.to_string())?;
                        row.budget_percent_of_reference = budget
                            .as_object()
                            .unwrap()
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.clone(),
                                    100.0 * v.as_f64().unwrap() / row.reference_weight,
                                )
                            })
                            .collect();
                        row.reference_plane = scene.reference_plane.clone();
                        row.detected_percent =
                            100.0 * report.budget.detected / row.reference_weight;
                        row.unresolved_percent = 100.0
                            * (report.budget.numerical_failure
                                + report.budget.medium_mismatch
                                + report.budget.branch_cutoff
                                + report.budget.interaction_limit)
                            / row.reference_weight;
                        let mut coarse = Vec::new();
                        for map in &report.detectors {
                            let analysis = ScreenAnalysis::from_detector(map)?;
                            row.detectors.push(DetectorSummary {
                                name: map.name.clone(),
                                received_percent: 100.0 * map.weights.iter().sum::<f64>()
                                    / row.reference_weight,
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
                            });
                            let minimum = [0, 1].map(|a| {
                                plan.pixels
                                    .iter()
                                    .map(|p| p[a])
                                    .min()
                                    .unwrap_or(map.pixels[a])
                                    .min(64)
                                    .min(map.pixels[a])
                            });
                            coarse.push(rebin(map, minimum)?);
                        }
                        if plan.save_runs {
                            let name = format!("case_{index:04}.raytrace-run.json");
                            RunContext::capture(
                                &project,
                                &scene,
                                None,
                                plan.threads,
                                TraceOptions::default(),
                            )
                            .save(
                                &report,
                                row.trace_seconds,
                                &output.join(&name),
                            )?;
                            row.run_file = Some(name);
                        }
                        Ok(coarse)
                    })();
                    match result {
                        Ok(m) => maps.push(m),
                        Err(e) => {
                            row.error = Some(e);
                            maps.push(Vec::new());
                        }
                    }
                    summary.cases.push(row);
                    fs::write(
                        output.join("convergence.json"),
                        serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    for case in &summary.cases {
        if case.error.is_some() {
            continue;
        }
        for axis in ["ray_count", "seed", "pixels", "cad_deflection"] {
            if axis == "cad_deflection" && !case.cad_applied {
                continue;
            }
            let same_other_axes = |other: &&ConvergenceCase| {
                let (a, b) = (&case.settings, &other.settings);
                other.error.is_none()
                    && (axis == "ray_count" || a.rays_per_source == b.rays_per_source)
                    && (axis == "seed" || a.seed == b.seed)
                    && (axis == "pixels" || a.pixels == b.pixels)
                    && (axis == "cad_deflection" || a.cad_deflection_mm == b.cad_deflection_mm)
            };
            let candidates: Vec<_> = summary.cases.iter().filter(same_other_axes).collect();
            let reference = candidates
                .into_iter()
                .max_by(|a, b| match axis {
                    "ray_count" => a.settings.rays_per_source.cmp(&b.settings.rays_per_source),
                    "pixels" => a
                        .settings
                        .pixels
                        .map(|p| p[0] * p[1])
                        .cmp(&b.settings.pixels.map(|p| p[0] * p[1])),
                    "cad_deflection" => b
                        .settings
                        .cad_deflection_mm
                        .total_cmp(&a.settings.cad_deflection_mm),
                    _ => b.index.cmp(&a.index),
                })
                .unwrap();
            if case.index == reference.index {
                continue;
            }
            let mut detectors = Vec::new();
            for (i, (a, b)) in maps[case.index]
                .iter()
                .zip(&maps[reference.index])
                .enumerate()
            {
                if a.pixels != b.pixels || a.half_extent_mm != b.half_extent_mm {
                    continue;
                }
                detectors.push(DetectorDifference {
                    name: a.name.clone(),
                    comparison_pixels: a.pixels,
                    received_delta_percentage_points: case.detectors[i].received_percent
                        - reference.detectors[i].received_percent,
                    fwhm_delta_mm: [0, 1].map(|a| {
                        case.detectors[i].fwhm_mm[a]
                            .zip(reference.detectors[i].fwhm_mm[a])
                            .map(|(x, y)| x - y)
                    }),
                    distribution_l1_percent_of_reference: 100.0
                        * a.weights
                            .iter()
                            .zip(&b.weights)
                            .map(|(x, y)| {
                                (x / case.reference_weight - y / reference.reference_weight).abs()
                            })
                            .sum::<f64>(),
                });
            }
            summary.comparisons.push(ConvergenceComparison {
                axis: axis.into(),
                case: case.index,
                reference_case: reference.index,
                detectors,
            });
        }
    }
    fs::write(
        output.join("convergence.json"),
        serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(summary)
}
