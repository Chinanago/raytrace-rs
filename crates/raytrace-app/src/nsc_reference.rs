//! Native, independently constructed NSC reference fixtures and statistical comparisons.
use raytrace_core::{
    math::{Frame, Pose, V3},
    model::*,
    trace::{self, Budget, DetectorMap, TraceOptions},
};
use raytrace_io::assembly::{self, Assembly, Component, RodSize};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NscFixture {
    pub name: String,
    pub source: Source,
    pub optic: Option<Optic>,
    pub detectors: Vec<DetectorSpec>,
    pub runs: Vec<RunSpec>,
    pub settings: Settings,
    pub budget_rays: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Optic {
    Lens {
        pose: Pose,
        radii_mm: [f64; 2],
        thickness_mm: f64,
        semi_mm: f64,
    },
    Volume {
        pose: Pose,
        half_width_mm: [f64; 2],
        length_mm: f64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectorSpec {
    pub name: String,
    pub pose: Pose,
    pub half_extent_mm: [f64; 2],
    pub pixels: [usize; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunSpec {
    pub name: String,
    pub mode: String,
    pub wavelength_um: f64,
    pub detector: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub name: String,
    pub rays: usize,
    pub seed: u64,
    pub launched_weight: f64,
    pub detectors: Vec<DetectorMap>,
    pub budget: Budget,
    pub budget_rays: usize,
    pub small_run_budget: Budget,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RustResults {
    pub fixture: String,
    pub runs: Vec<RunResult>,
}
fn pose(p: [f64; 3], r: [f64; 3]) -> Pose {
    Pose {
        translation_mm: V3(p),
        rotation_deg: V3(r),
    }
}
fn without_absorption(material: Material) -> Material {
    match material {
        Material::WithAbsorption { dispersion, .. } => without_absorption(*dispersion),
        mut material => {
            match &mut material {
                Material::Catalog {
                    absorption_per_mm, ..
                }
                | Material::Constant {
                    absorption_per_mm, ..
                }
                | Material::Sellmeier {
                    absorption_per_mm, ..
                }
                | Material::Schott {
                    absorption_per_mm, ..
                }
                | Material::Tabulated {
                    absorption_per_mm, ..
                } => *absorption_per_mm = 0.,
                Material::WithAbsorption { .. } => unreachable!(),
            }
            material
        }
    }
}

impl NscFixture {
    /// Build geometry natively. Only the selected absorbing detector is present.
    pub fn scene(
        &self,
        run: &RunSpec,
        glass: Material,
        seed: u64,
        rays: usize,
    ) -> Result<Scene, Box<dyn std::error::Error>> {
        if !["geometric", "fresnel"].contains(&run.mode.as_str()) || rays == 0 {
            return Err("Invalid NSC mode or zero ray count".into());
        }
        let detector = self
            .detectors
            .get(run.detector)
            .ok_or("Detector index out of range")?;
        if self.source.diode.is_none() {
            return Err("NSC reference requires a diode source".into());
        }
        let mut scene = assembly::empty_scene(&self.name);
        scene.settings = self.settings.clone();
        // OpticStudio applies no bulk absorption with polarization off, so the
        // geometric mode is lossless on both sides; absorption is compared in fresnel.
        let (reflectance, glass) = if run.mode == "geometric" {
            (Some(0.), without_absorption(glass))
        } else {
            (None, glass)
        };
        if let Some(optic) = &self.optic {
            match optic {
                Optic::Volume {
                    pose,
                    half_width_mm: [x, y],
                    length_mm,
                } => {
                    let mut rod = Component::rod(glass, "N-BK7".into());
                    rod.pose = Frame::from_intrinsic_xyz(pose).pose();
                    rod.rod = Some(RodSize {
                        width_mm: 2. * x,
                        height_mm: 2. * y,
                        length_mm: *length_mm,
                        coatings: Default::default(),
                    });
                    scene = Assembly {
                        components: vec![rod],
                    }
                    .compose(&scene)?
                    .0;
                }
                Optic::Lens {
                    pose,
                    radii_mm,
                    thickness_mm,
                    semi_mm,
                } => {
                    scene.materials.push(glass);
                    let frame = Frame::from_intrinsic_xyz(pose);
                    for (i, &radius) in radii_mm.iter().enumerate() {
                        scene.surfaces.push(Surface {
                            name: format!("lens_{i}"),
                            coating: None,
                            pose: frame.transform_pose(&crate_pose(if i == 0 {
                                0.
                            } else {
                                *thickness_mm
                            })),
                            shape: Shape::Sphere { radius_mm: radius },
                            aperture: Aperture::Circle {
                                radius_mm: *semi_mm,
                            },
                            interaction: Interaction::Dielectric {
                                minus_medium: i,
                                plus_medium: 1 - i,
                                reflectance,
                            },
                        });
                    }
                    let sag = |r: f64| r.signum() * (r.abs() - (r * r - semi_mm * semi_mm).sqrt());
                    let z1 = sag(radii_mm[0]);
                    let z2 = thickness_mm + sag(radii_mm[1]);
                    scene.surfaces.push(Surface {
                        name: "barrel".into(),
                        coating: None,
                        pose: frame.transform_pose(&crate_pose(z1)),
                        shape: Shape::ConicalSide {
                            radius_start_mm: *semi_mm,
                            radius_end_mm: *semi_mm,
                            length_mm: z2 - z1,
                        },
                        aperture: Aperture::Circle {
                            radius_mm: *semi_mm,
                        },
                        interaction: Interaction::Dielectric {
                            minus_medium: 1,
                            plus_medium: 0,
                            reflectance,
                        },
                    });
                }
            }
        }
        for s in &mut scene.surfaces {
            if let Interaction::Dielectric { reflectance: r, .. } = &mut s.interaction {
                *r = reflectance;
            }
        }
        let d = detector;
        scene.surfaces.push(Surface {
            name: d.name.clone(),
            coating: None,
            pose: Frame::from_intrinsic_xyz(&d.pose).pose(),
            shape: Shape::Plane,
            aperture: Aperture::Rectangle {
                half_width_mm: d.half_extent_mm[0],
                half_height_mm: d.half_extent_mm[1],
            },
            interaction: Interaction::Detector { pixels: d.pixels },
        });
        let mut source = self.source.clone();
        source.seed = seed;
        source.count = rays;
        source.wavelength_um = run.wavelength_um;
        let frame = Frame::from_intrinsic_xyz(&pose(
            source.origin_mm.0,
            source.diode.as_ref().unwrap().rotation_deg.0,
        ));
        source.direction = frame.world_vector(V3([0., 0., 1.]));
        source.diode.as_mut().unwrap().rotation_deg = frame.pose().rotation_deg;
        scene.sources = vec![source];
        Ok(scene)
    }
    pub fn trace(
        &self,
        run: &RunSpec,
        glass: Material,
        seed: u64,
    ) -> Result<RunResult, Box<dyn std::error::Error>> {
        let scene = self
            .scene(run, glass.clone(), seed, self.source.count)?
            .prepare()?;
        let report = trace::trace_generated(&scene, TraceOptions::default())?;
        let small = self.scene(run, glass, seed, self.budget_rays)?.prepare()?;
        let small = trace::trace_generated(&small, TraceOptions::default())?;
        Ok(RunResult {
            name: run.name.clone(),
            rays: self.source.count,
            seed,
            launched_weight: report.budget.launched,
            detectors: report.detectors,
            budget: report.budget,
            budget_rays: self.budget_rays,
            small_run_budget: small.budget,
        })
    }
}
fn crate_pose(z: f64) -> Pose {
    pose([0., 0., z], [0.; 3])
}
/// Four owned fixture specifications. No vendor geometry or catalog data is embedded.
pub fn fixtures(rays: usize, budget_rays: usize) -> Vec<NscFixture> {
    let source = Source {
        pupil: None,
        spectrum: None,
        sampling: Sampling::Pseudorandom,
        count: rays,
        origin_mm: V3([0.5, -0.2, 0.]),
        direction: V3([0., 0., 1.]),
        disk_radius_mm: 0.,
        cone_half_angle_deg: 0.,
        wavelength_um: 0.55,
        medium: 0,
        total_weight: 1.,
        seed: 142,
        diode: Some(Diode {
            astigmatism_mm: 0.,
            angular_supergaussian: [1.; 2],
            spatial_supergaussian: [1.; 2],
            rotation_deg: V3([0.; 3]),
            divergence_deg: [8., 20.],
            array: [1; 2],
            spacing_mm: [0.; 2],
            half_width_mm: [0.02, 0.01],
            gaussian_width_mm: [0.01, 0.005],
        }),
    };
    let detector = |name: &str, z, h| DetectorSpec {
        name: name.into(),
        pose: crate_pose(z),
        half_extent_mm: h,
        pixels: [64; 2],
    };
    let run = |name: &str, mode: &str, w, detector| RunSpec {
        name: name.into(),
        mode: mode.into(),
        wavelength_um: w,
        detector,
    };
    let base = NscFixture {
        name: "diode_direct".into(),
        source,
        optic: None,
        detectors: vec![detector("screen", 50., [15., 30.])],
        runs: vec![run("geometric_550", "geometric", 0.55, 0)],
        settings: Settings {
            max_depth: 128,
            max_events_per_launch: 4096,
            min_relative_branch_weight: 1e-7,
        },
        budget_rays,
    };
    let mut lens = base.clone();
    lens.name = "lens_focus".into();
    lens.source.diode.as_mut().unwrap().divergence_deg = [0.5, 0.5];
    lens.source.diode.as_mut().unwrap().half_width_mm = [3., 2.];
    lens.source.diode.as_mut().unwrap().gaussian_width_mm = [3., 2.];
    lens.optic = Some(Optic::Lens {
        pose: crate_pose(10.),
        radii_mm: [50., -50.],
        thickness_mm: 6.,
        semi_mm: 10.,
    });
    lens.detectors = vec![detector("focus", 62., [2., 2.])];
    lens.runs = vec![
        run("geometric_550", "geometric", 0.55, 0),
        run("fresnel_550", "fresnel", 0.55, 0),
        run("geometric_450", "geometric", 0.45, 0),
        run("geometric_650", "geometric", 0.65, 0),
    ];
    let mut rod = base.clone();
    rod.name = "rod_homogenizer".into();
    rod.source.diode.as_mut().unwrap().divergence_deg = [10., 25.];
    rod.optic = Some(Optic::Volume {
        pose: crate_pose(1.),
        half_width_mm: [2., 2.],
        length_mm: 40.,
    });
    rod.detectors = vec![
        detector("exit", 41.001, [3., 3.]),
        detector("far", 61., [12., 20.]),
    ];
    rod.runs = vec![
        run("geometric_exit", "geometric", 0.55, 0),
        run("geometric_far", "geometric", 0.55, 1),
        run("fresnel_exit", "fresnel", 0.55, 0),
        run("fresnel_far", "fresnel", 0.55, 1),
    ];
    let mut window = base.clone();
    window.name = "tilted_window".into();
    window.source.diode.as_mut().unwrap().divergence_deg = [0., 0.];
    window.optic = Some(Optic::Volume {
        pose: pose([0., 0., 10.], [30., 0., 0.]),
        half_width_mm: [15., 15.],
        length_mm: 3.,
    });
    window.detectors = vec![detector("transmitted", 30., [4., 4.])];
    window.runs = vec![run("fresnel_550", "fresnel", 0.55, 0)];
    vec![base, lens, rod, window]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapMetrics {
    pub power: f64,
    pub centroid_mm: [f64; 2],
    pub rms_mm: [f64; 2],
    pub peak_pixel: [usize; 2],
}
/// Pixel-centre moments; values must be nonnegative integrated pixel powers.
pub fn metrics(map: &DetectorMap) -> Result<MapMetrics, String> {
    let [nx, ny] = map.pixels;
    if nx == 0
        || ny == 0
        || nx.checked_mul(ny) != Some(map.weights.len())
        || map.weights.iter().any(|v| !v.is_finite() || *v < 0.)
        || map
            .half_extent_mm
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.)
    {
        return Err("Invalid detector map".into());
    }
    let power: f64 = map.weights.iter().sum();
    if !power.is_finite() || power <= 0. {
        return Err("Empty detector map; shape metrics undefined".into());
    }
    let mut centroid = [0.; 2];
    let mut second = [0.; 2];
    let mut peak = 0;
    for (i, &w) in map.weights.iter().enumerate() {
        let c = [
            ((i % nx) as f64 + 0.5) * 2. * map.half_extent_mm[0] / nx as f64
                - map.half_extent_mm[0],
            ((i / nx) as f64 + 0.5) * 2. * map.half_extent_mm[1] / ny as f64
                - map.half_extent_mm[1],
        ];
        for j in 0..2 {
            centroid[j] += w * c[j] / power;
            second[j] += w * c[j] * c[j] / power;
        }
        if w > map.weights[peak] {
            peak = i;
        }
    }
    Ok(MapMetrics {
        power,
        centroid_mm: centroid,
        rms_mm: std::array::from_fn(|j| (second[j] - centroid[j] * centroid[j]).max(0.).sqrt()),
        peak_pixel: [peak % nx, peak / nx],
    })
}
pub fn normalized_l1(a: &DetectorMap, b: &DetectorMap) -> Result<f64, String> {
    if a.pixels != b.pixels || a.half_extent_mm != b.half_extent_mm {
        return Err("Detector geometry mismatch".into());
    }
    let (pa, pb) = (metrics(a)?.power, metrics(b)?.power);
    Ok(a.weights
        .iter()
        .zip(&b.weights)
        .map(|(x, y)| (x / pa - y / pb).abs())
        .sum())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Check {
    pub metric: String,
    pub value: f64,
    pub tolerance: f64,
    pub passed: bool,
}
impl Check {
    pub fn new(metric: impl Into<String>, value: f64, tolerance: f64) -> Self {
        Self {
            metric: metric.into(),
            value,
            tolerance,
            passed: value.is_finite() && tolerance.is_finite() && value <= tolerance,
        }
    }
}
pub fn power_tolerance(a: f64, b: f64, n: usize, m: usize) -> f64 {
    let sampling =
        (a.clamp(0., 1.) * (1. - a.clamp(0., 1.)) * (1. / n as f64 + 1. / m as f64)).sqrt();
    3. * sampling.max((a - b).abs() / 2_f64.sqrt()) + 0.001
}
pub fn compare_maps(
    a: &DetectorMap,
    b: &DetectorMap,
    os: &DetectorMap,
    n: usize,
    m: usize,
    launched: [f64; 3],
) -> Result<Vec<Check>, String> {
    if n == 0 || m == 0 || launched.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err("Invalid ray counts or launched power".into());
    }
    let (ma, mb, mo) = (metrics(a)?, metrics(b)?, metrics(os)?);
    let mut checks = vec![
        Check::new(
            "normalized_l1",
            normalized_l1(a, os)?,
            1.5 * normalized_l1(a, b)? + 0.005,
        ),
        Check::new(
            "power_fraction_difference",
            (ma.power / launched[0] - mo.power / launched[2]).abs(),
            power_tolerance(ma.power / launched[0], mb.power / launched[1], n, m),
        ),
    ];
    for j in 0..2 {
        let half_pixel = a.half_extent_mm[j] / a.pixels[j] as f64;
        for (name, aa, bb, oo) in [
            (
                "centroid_mm",
                ma.centroid_mm[j],
                mb.centroid_mm[j],
                mo.centroid_mm[j],
            ),
            ("rms_mm", ma.rms_mm[j], mb.rms_mm[j], mo.rms_mm[j]),
        ] {
            checks.push(Check::new(
                format!("{name}_{}", ["x", "y"][j]),
                (aa - oo).abs(),
                (3. * (aa - bb).abs() / 2_f64.sqrt()).max(half_pixel),
            ));
        }
    }
    Ok(checks)
}
/// Infinite parallel plate: unpolarized first interface, either independent average
/// (Rust) or separately conserved S/P (OpticStudio). Includes all internal bounces.
pub fn plate_expectation(n: f64, alpha: f64, thickness: f64, polarized: bool) -> [f64; 3] {
    let c = 30_f64.to_radians().cos();
    let ct = (1. - (1. - c * c) / (n * n)).sqrt();
    let rs = ((c - n * ct) / (c + n * ct)).powi(2);
    let rp = ((n * c - ct) / (n * c + ct)).powi(2);
    let a = (-alpha * thickness / ct).exp();
    let calc = |r: f64| {
        let t = (1. - r).powi(2) * a / (1. - r * r * a * a);
        let refl = r + (1. - r).powi(2) * r * a * a / (1. - r * r * a * a);
        [t, refl, 1. - t - refl]
    };
    if polarized {
        let s = calc(rs);
        let p = calc(rp);
        std::array::from_fn(|j| (s[j] + p[j]) / 2.)
    } else {
        calc((rs + rp) / 2.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn map(w: Vec<f64>) -> DetectorMap {
        DetectorMap {
            name: "test".into(),
            pixels: [4, 1],
            half_extent_mm: [2., 0.5],
            weights: w,
        }
    }
    #[test]
    fn moments_and_orientation() {
        let a = map(vec![1., 0., 0., 0.]);
        let b = map(vec![0., 1., 0., 0.]);
        let flip = map(vec![0., 0., 0., 1.]);
        assert_eq!(normalized_l1(&a, &a).unwrap(), 0.);
        assert_eq!(normalized_l1(&a, &flip).unwrap(), 2.);
        assert_eq!(
            metrics(&b).unwrap().centroid_mm[0] - metrics(&a).unwrap().centroid_mm[0],
            1.
        );
        assert!(metrics(&map(vec![0.; 4])).is_err());
        let asymmetric = DetectorMap {
            name: "2d".into(),
            pixels: [2, 2],
            half_extent_mm: [1., 1.],
            weights: vec![3., 1., 0., 0.],
        };
        let m = metrics(&asymmetric).unwrap();
        assert_eq!(m.centroid_mm, [-0.25, -0.5]);
        assert!((m.rms_mm[0] - 3_f64.sqrt() / 4.).abs() < 1e-14);
        assert_eq!(m.rms_mm[1], 0.);
        let flipped_y = DetectorMap {
            weights: vec![0., 0., 3., 1.],
            ..asymmetric.clone()
        };
        assert_eq!(normalized_l1(&asymmetric, &flipped_y).unwrap(), 2.);
        assert_eq!(
            metrics(&flipped_y).unwrap().centroid_mm[1] - m.centroid_mm[1],
            1.
        );
    }
    #[test]
    fn tolerances() {
        let a = map(vec![1., 0., 0., 0.]);
        let b = map(vec![0., 1., 0., 0.]);
        assert!(
            compare_maps(&a, &a, &a, 1000, 1000, [1.; 3])
                .unwrap()
                .iter()
                .all(|c| c.passed)
        );
        assert!(
            compare_maps(&a, &a, &b, 1000, 1000, [1.; 3])
                .unwrap()
                .iter()
                .any(|c| !c.passed)
        );
        assert_eq!(power_tolerance(1., 1., 1000, 1000), 0.001);
        assert!(!Check::new("nan", f64::NAN, 1.).passed);
    }
    #[test]
    fn geometric_mode_drops_bulk_absorption_only() {
        let glass = Material::WithAbsorption {
            dispersion: Box::new(Material::Constant {
                n: 1.5,
                absorption_per_mm: 0.2,
            }),
            samples: vec![[0.5, 0.1]],
        };
        let lossless = without_absorption(glass);
        assert_eq!(lossless.index(0.55), Some(1.5));
        assert_eq!(lossless.absorption_at(0.55), Some(0.));
    }
    #[test]
    fn construction_and_roundtrip() {
        for f in fixtures(100, 20) {
            let copy: NscFixture =
                serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
            assert_eq!(f.name, copy.name);
            assert_eq!(
                serde_json::to_value(&f).unwrap(),
                serde_json::to_value(&copy).unwrap()
            );
            let s = f
                .scene(
                    &f.runs[0],
                    Material::Constant {
                        n: 1.52,
                        absorption_per_mm: 0.,
                    },
                    1,
                    100,
                )
                .unwrap();
            assert_eq!(
                s.surfaces.len(),
                match f.optic {
                    None => 1,
                    Some(Optic::Lens { .. }) => 4,
                    _ => 7,
                }
            );
            assert_eq!(s.materials.len(), if f.optic.is_some() { 2 } else { 1 });
            assert!(matches!(
                s.surfaces.last().unwrap().interaction,
                Interaction::Detector { pixels: [64, 64] }
            ));
            s.prepare().unwrap();
        }
    }
    #[test]
    fn tilted_plate_analytic() {
        let f = fixtures(1000, 100).pop().unwrap();
        let n = 1.52;
        let alpha = 0.0001;
        let s = f
            .scene(
                &f.runs[0],
                Material::Constant {
                    n,
                    absorption_per_mm: alpha,
                },
                1,
                1000,
            )
            .unwrap()
            .prepare()
            .unwrap();
        let r = trace::trace_generated(&s, TraceOptions::default()).unwrap();
        let e = plate_expectation(n, alpha, 3., false);
        assert!(
            (r.budget.detected - e[0]).abs() < 2e-6,
            "{:?} {e:?}",
            r.budget
        );
        assert!((r.budget.escaped - e[1]).abs() < 2e-6);
        assert!((r.budget.bulk_absorbed - e[2]).abs() < 2e-6);
        assert!((r.budget.accounted() - r.budget.launched).abs() < 1e-12);
    }
}
