//! Deterministic, redistributable fixtures with explicit independent checks.
use crate::Result;
use raytrace_core::{
    math::{Frame, Pose, V3},
    model::*,
    trace::{trace, Report, TraceOptions},
};
use raytrace_io::{project::Project, record::RunContext, zemax};
use serde::Serialize;
use std::{fs, path::Path};

pub struct ValidationCase {
    pub name: &'static str,
    pub scene: Scene,
    pub rays: Vec<Ray>,
    oracle: Oracle,
    exportable: bool,
}
enum Oracle {
    NativeRoundtrip,
    Slab,
    Rod(Option<Frame>),
    Mirror,
}
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub metric: String,
    pub maximum_error: f64,
    pub tolerance: f64,
    pub passed: bool,
    pub reference: String,
}
fn check(metric: &str, error: f64, tolerance: f64, reference: &str) -> Check {
    Check {
        metric: metric.into(),
        maximum_error: error,
        tolerance,
        passed: error.is_finite() && error <= tolerance,
        reference: reference.into(),
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct CaseResult {
    pub name: String,
    pub ray_count: usize,
    pub checks: Vec<Check>,
    pub zemax_file: Option<String>,
    pub opticstudio_verification: String,
}
fn launch(id: u64, position: V3, direction: V3, wavelength_um: f64) -> Ray {
    Ray {
        id,
        position_mm: position,
        direction: direction.unit(),
        weight: 1.0,
        wavelength_um,
        medium: 0,
        optical_path_mm: 0.0,
    }
}
fn grid_rays(z: f64) -> Vec<Ray> {
    let xy = [
        [0.0, 0.0],
        [0.1, 0.0],
        [0.5, 0.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [0.5, 0.5],
        [-1.0, -0.5],
    ];
    [0.45, 0.55, 0.635]
        .into_iter()
        .flat_map(|w| xy.into_iter().map(move |[x, y]| (w, x, y)))
        .enumerate()
        .map(|(i, (w, x, y))| launch(i as u64, V3([x, y, z]), V3([0.0, 0.0, 1.0]), w))
        .collect()
}
pub fn cases() -> Vec<ValidationCase> {
    let mut lens: Scene =
        serde_json::from_str(include_str!("../../../examples/sequential_lens.json")).unwrap();
    lens.surfaces.last_mut().unwrap().aperture = Aperture::Rectangle {
        half_width_mm: 5.0,
        half_height_mm: 5.0,
    };
    let source = lens.sources[0].clone();
    lens.sources = [0.45, 0.55, 0.635]
        .into_iter()
        .map(|w| Source {
            pupil: None,
            spectrum: None,
            count: 1,
            wavelength_um: w,
            ..source.clone()
        })
        .collect();
    let mut asphere = lens.clone();
    asphere.surfaces[0].shape = Shape::EvenAsphere {
        radius_mm: 50.0,
        conic: -0.4,
        coefficients: vec![2e-6, -1e-9],
    };
    let mut slab: Scene =
        serde_json::from_str(include_str!("../../../examples/nonsequential_slab.json")).unwrap();
    slab.settings.min_relative_branch_weight = 1e-14;
    let mut plate = slab.clone();
    plate.mode = Mode::Sequential;
    plate.surfaces = [1, 3, 0].map(|i| slab.surfaces[i].clone()).to_vec();
    for s in &mut plate.surfaces {
        s.pose.translation_mm.0[2] += 5.0;
    }
    plate.sources = lens.sources.clone();
    let rod: Scene =
        serde_json::from_str(include_str!("../../../examples/integrator_rod.json")).unwrap();
    let rod_rays: Vec<_> = [0.45, 0.55, 0.635]
        .into_iter()
        .flat_map(|w| [5.0f64, 10.0].into_iter().map(move |a| (w, a.to_radians())))
        .enumerate()
        .map(|(i, (w, a))| {
            launch(
                i as u64,
                V3([0.05, 0.0, -2.0]),
                V3([a.sin(), 0.0, a.cos()]),
                w,
            )
        })
        .collect();
    let frame = Frame::new(&Pose {
        translation_mm: V3([3.0, -2.0, 8.0]),
        rotation_deg: V3([12.0, 23.0, 7.0]),
    });
    let mut rotated = rod.clone();
    for surface in &mut rotated.surfaces {
        surface.pose = frame.transform_pose(&surface.pose);
    }
    for s in &mut rotated.sources {
        s.origin_mm = frame.world_point(s.origin_mm);
        s.direction = frame.world_vector(s.direction);
    }
    let rotated_rays = rod_rays
        .iter()
        .map(|r| Ray {
            position_mm: frame.world_point(r.position_mm),
            direction: frame.world_vector(r.direction),
            ..*r
        })
        .collect();
    let mut mirror: Scene =
        serde_json::from_str(include_str!("../../../examples/folded_mirror.json")).unwrap();
    let mut mirror_seq = mirror.clone();
    mirror_seq.mode = Mode::Sequential;
    let mirror_rays = grid_rays(-20.0);
    // Non-sequential slab oracle uses axial rays, for which Rs == Rp exactly.
    let axial = [0.45, 0.55, 0.635]
        .into_iter()
        .enumerate()
        .map(|(i, w)| launch(i as u64, V3([0.0, 0.0, -5.0]), V3([0.0, 0.0, 1.0]), w))
        .collect();
    mirror.settings.min_relative_branch_weight = 1e-14;
    vec![
        ValidationCase {
            name: "sequential_lens",
            scene: lens,
            rays: grid_rays(0.0),
            oracle: Oracle::NativeRoundtrip,
            exportable: true,
        },
        ValidationCase {
            name: "even_asphere",
            scene: asphere,
            rays: grid_rays(0.0),
            oracle: Oracle::NativeRoundtrip,
            exportable: true,
        },
        ValidationCase {
            name: "sequential_plate",
            scene: plate,
            rays: grid_rays(0.0),
            oracle: Oracle::NativeRoundtrip,
            exportable: true,
        },
        ValidationCase {
            name: "multiple_reflection_slab",
            scene: slab,
            rays: axial,
            oracle: Oracle::Slab,
            exportable: false,
        },
        ValidationCase {
            name: "dispersive_rod",
            scene: rod,
            rays: rod_rays,
            oracle: Oracle::Rod(None),
            exportable: false,
        },
        ValidationCase {
            name: "rotated_rod",
            scene: rotated,
            rays: rotated_rays,
            oracle: Oracle::Rod(Some(frame)),
            exportable: false,
        },
        ValidationCase {
            name: "folded_mirror_sequential",
            scene: mirror_seq,
            rays: mirror_rays.clone(),
            oracle: Oracle::Mirror,
            exportable: false,
        },
        ValidationCase {
            name: "folded_mirror_nonsequential",
            scene: mirror,
            rays: mirror_rays,
            oracle: Oracle::Mirror,
            exportable: false,
        },
    ]
}

fn primary(report: &Report, id: u64, surface: usize) -> Option<&Ray> {
    report
        .terminals
        .iter()
        .filter(|t| t.ray.id == id && t.surface == Some(surface) && t.reason == "detected")
        .max_by(|a, b| a.ray.weight.total_cmp(&b.ray.weight))
        .map(|t| &t.ray)
}

impl ValidationCase {
    pub fn run(&self, directory: &Path) -> Result<CaseResult> {
        fs::create_dir_all(directory)
            .map_err(|e| crate::Error::from(raytrace_io::Error::io(directory, e)))?;
        let project = Project::new(self.scene.clone());
        let prepared = self.scene.clone().prepare()?;
        let mut rays = self.rays.clone();
        for ray in &mut rays {
            ray.validate(&prepared)?;
        }
        let options = TraceOptions {
            keep_terminals: true,
            path_launch_count: rays.len() as u64,
        };
        let started = std::time::Instant::now();
        let report = trace(&prepared, &rays, options);
        let seconds = started.elapsed().as_secs_f64();
        RunContext::capture(&project, &self.scene, Some(&rays), 1, options).save(
            &report,
            seconds,
            &directory.join("run.json"),
        )?;
        let mut checks = vec![
            check(
                "relative_power_balance",
                (report.budget.accounted() - report.budget.launched).abs() / report.budget.launched,
                1e-12,
                "power conservation",
            ),
            check(
                "unresolved_weight",
                report.budget.medium_mismatch
                    + report.budget.numerical_failure
                    + report.budget.interaction_limit,
                0.0,
                "no unresolved branches",
            ),
        ];
        let mut zemax_file = None;
        if self.exportable {
            let path = directory.join(format!("{}.zmx", self.name));
            zemax::export(&project, &path)?;
            let imported = Project::open(&path, &[])?;
            let mut scene = imported.trace_scene()?;
            scene.surfaces.remove(0); // ZMX export's dummy starting surface.
            let other = trace(&scene.prepare()?, &rays, options);
            let mut position: f64 = 0.0;
            let mut direction: f64 = 0.0;
            let mut opl: f64 = 0.0;
            let mut weight: f64 = 0.0;
            for ray in &rays {
                let a = primary(&report, ray.id, self.scene.surfaces.len() - 1)
                    .ok_or_else(|| crate::Error::Plan("基準の検出光線が見つかりません".into()))?;
                let b =
                    primary(&other, ray.id, self.scene.surfaces.len() - 1).ok_or_else(|| {
                        crate::Error::Plan("再読込後の検出光線が見つかりません".into())
                    })?;
                position = position.max((a.position_mm - b.position_mm).norm());
                direction = direction.max((a.direction - b.direction).norm());
                opl = opl.max((a.optical_path_mm - b.optical_path_mm).abs());
                weight = weight.max((a.weight - b.weight).abs());
            }
            for (m, e, t) in [
                ("native_zmx_position_mm", position, 1e-7),
                ("native_zmx_direction", direction, 1e-9),
                ("native_zmx_optical_path_mm", opl, 1e-7),
                ("native_zmx_weight", weight, 1e-10),
            ] {
                checks.push(check(
                    m,
                    e,
                    t,
                    "native ZMX/AGF roundtrip; both traces use Rust",
                ));
            }
            let reference = serde_json::json!({"input":path,"rays":rays,"report":report,"wavelengths_um":[0.45,0.55,0.635],"start_surface":1,
                "scope":"Direct unpolarized geometric comparison. Fresnel power and optical path require separate independent validation."});
            fs::write(
                directory.join("reference.json"),
                serde_json::to_vec_pretty(&reference)
                    .map_err(|e| raytrace_io::Error::json(None, e))?,
            )
            .map_err(|e| {
                crate::Error::from(raytrace_io::Error::io(directory.join("reference.json"), e))
            })?;
            zemax_file = path.file_name().map(|p| p.to_string_lossy().into());
        }
        match &self.oracle {
            Oracle::Slab => {
                for (name, expected) in [
                    ("transmitted_collector", 0.96 / 1.04),
                    ("reflected_collector", 0.08 / 1.04),
                ] {
                    let got = report
                        .detectors
                        .iter()
                        .find(|d| d.name == name)
                        .ok_or_else(|| crate::Error::Plan("集光面が見つかりません".into()))?
                        .weights
                        .iter()
                        .sum::<f64>()
                        / report.budget.launched;
                    checks.push(check(
                        name,
                        (got - expected).abs(),
                        1e-12,
                        "analytic infinite internal-reflection series at normal incidence",
                    ));
                }
            }
            Oracle::Rod(frame) => {
                let forward = self
                    .scene
                    .surfaces
                    .iter()
                    .position(|s| s.name == "forward_collector")
                    .ok_or_else(|| crate::Error::Plan("ロッドの集光面が見つかりません".into()))?;
                let mut position: f64 = 0.0;
                let mut direction: f64 = 0.0;
                let mut opl: f64 = 0.0;
                for ray in &rays {
                    let incoming = frame
                        .as_ref()
                        .map_or(ray.direction, |f| f.local_vector(ray.direction));
                    let origin = frame
                        .as_ref()
                        .map_or(ray.position_mm, |f| f.local_point(ray.position_mm));
                    let n = self.scene.materials[1].index(ray.wavelength_um).unwrap();
                    let st = incoming.0[0] / n;
                    let ct = (1.0 - st * st).sqrt();
                    let tangent = incoming.0[0] / incoming.0[2];
                    let unfolded = origin.0[0] + 2.0 * tangent + 0.5 + 30.0 * st / ct;
                    let folded = 1.0 - (unfolded.rem_euclid(2.0) - 1.0).abs() - 0.5;
                    let sign = if (unfolded.floor() as i64) % 2 == 0 {
                        1.0
                    } else {
                        -1.0
                    };
                    let p = V3([folded + 5.0 * tangent * sign, 0.0, 35.0]);
                    let d = V3([incoming.0[0] * sign, 0.0, incoming.0[2]]);
                    let expected_p = frame.as_ref().map_or(p, |f| f.world_point(p));
                    let expected_d = frame.as_ref().map_or(d, |f| f.world_vector(d));
                    let actual = primary(&report, ray.id, forward).ok_or_else(|| {
                        crate::Error::Plan("ロッドの基準光線が見つかりません".into())
                    })?;
                    position = position.max((actual.position_mm - expected_p).norm());
                    direction = direction.max((actual.direction - expected_d).norm());
                    opl = opl.max(
                        (actual.optical_path_mm - (7.0 / incoming.0[2] + 30.0 * n / ct)).abs(),
                    );
                }
                checks.push(check(
                    "unfolded_rod_position_mm",
                    position,
                    1e-7,
                    "analytic unfolded rectangular rod",
                ));
                checks.push(check(
                    "unfolded_rod_direction",
                    direction,
                    1e-9,
                    "analytic total internal reflection",
                ));
                checks.push(check(
                    "unfolded_rod_optical_path_mm",
                    opl,
                    1e-7,
                    "analytic sum of n(lambda) times path length",
                ));
            }
            Oracle::Mirror => {
                let mut position: f64 = 0.0;
                let mut direction: f64 = 0.0;
                let mut opl: f64 = 0.0;
                let mut weight: f64 = 0.0;
                for ray in &rays {
                    let a = primary(&report, ray.id, 1)
                        .ok_or_else(|| crate::Error::Plan("ミラーの光線が見つかりません".into()))?;
                    position = position.max(
                        (a.position_mm - V3([40.0, ray.position_mm.0[1], ray.position_mm.0[0]]))
                            .norm(),
                    );
                    direction = direction.max((a.direction - V3([1.0, 0.0, 0.0])).norm());
                    opl = opl.max((a.optical_path_mm - 60.0).abs());
                    weight = weight.max((a.weight - 0.98).abs());
                }
                for (m, e, t) in [
                    ("mirror_position_mm", position, 1e-7),
                    ("mirror_direction", direction, 1e-9),
                    ("mirror_optical_path_mm", opl, 1e-7),
                    ("mirror_weight", weight, 1e-12),
                ] {
                    checks.push(check(m, e, t, "analytic 45-degree mirror"));
                }
            }
            Oracle::NativeRoundtrip => (),
        }
        Ok(CaseResult { name:self.name.into(),ray_count:rays.len(),checks,zemax_file,
            opticstudio_verification:"not_run; native roundtrip and analytic checks do not establish OpticStudio agreement".into() })
    }
}
