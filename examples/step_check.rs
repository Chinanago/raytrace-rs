//! Measure native STEP import and compare ray intersections with a reference mesh.
use raytrace_rs::{
    cad,
    math::{Pose, V3},
    mesh::MeshAccel,
    model::{Aperture, Interaction, Material, Mode, Ray, Scene, Settings, Shape, Surface},
    trace::{trace, Report, TraceOptions},
};
use std::{env, fs, path::Path, time::Instant};
#[path = "../tests/support/step.rs"]
mod owned;

fn bounds(shape: &Shape) -> [V3; 2] {
    let Shape::TriangleMesh { vertices, .. } = shape else {
        unreachable!()
    };
    [
        V3(std::array::from_fn(|a| {
            vertices
                .iter()
                .map(|p| p.0[a])
                .fold(f64::INFINITY, f64::min)
        })),
        V3(std::array::from_fn(|a| {
            vertices
                .iter()
                .map(|p| p.0[a])
                .fold(f64::NEG_INFINITY, f64::max)
        })),
    ]
}
fn optical_scene(mesh: Shape, reference_bounds: [V3; 2]) -> Scene {
    let [lo, hi] = reference_bounds;
    let center = (lo + hi) * 0.5;
    let extent = (hi - lo).norm() * 3. + 10.;
    let mut surfaces = vec![Surface {
        name: "STEP solid".into(),
        coating: None,
        pose: Pose::default(),
        shape: mesh,
        aperture: Aperture::Rectangle {
            half_width_mm: 1e8,
            half_height_mm: 1e8,
        },
        interaction: Interaction::Dielectric {
            minus_medium: 1,
            plus_medium: 0,
            reflectance: None,
        },
    }];
    for axis in 0..3 {
        for sign in [-1., 1.] {
            let mut position = center;
            position.0[axis] += extent * sign;
            let rotation = match axis {
                0 => V3([0., 90., 0.]),
                1 => V3([-90., 0., 0.]),
                _ => V3::default(),
            };
            surfaces.push(Surface {
                name: format!("collector_{axis}_{sign}"),
                coating: None,
                pose: Pose {
                    translation_mm: position,
                    rotation_deg: rotation,
                },
                shape: Shape::Plane,
                aperture: Aperture::Rectangle {
                    half_width_mm: extent * 1.01,
                    half_height_mm: extent * 1.01,
                },
                interaction: Interaction::Detector { pixels: [32, 32] },
            });
        }
    }
    Scene {
        name: "STEP optical comparison".into(),
        reference_plane: "fixed launch rays = 100%".into(),
        mode: Mode::NonSequential,
        materials: vec![
            Material::Constant {
                n: 1.,
                absorption_per_mm: 0.,
            },
            Material::Tabulated {
                samples: vec![[0.45, 1.52], [0.65, 1.49]],
                absorption_per_mm: 0.001,
            },
        ],
        surfaces,
        settings: Settings {
            max_depth: 128,
            max_events_per_launch: 16384,
            min_relative_branch_weight: 1e-8,
        },
        sources: vec![],
    }
}
fn optical_report(scene: Scene, rays: &[Ray]) -> Result<Report, String> {
    let scene = scene.prepare().map_err(|e| e.to_string())?;
    for mut ray in rays.iter().copied() {
        ray.validate(&scene).map_err(|e| e.to_string())?;
    }
    Ok(trace(
        &scene,
        rays,
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 0,
        },
    ))
}
fn statistics(v: &mut [f64]) -> serde_json::Value {
    v.sort_by(f64::total_cmp);
    serde_json::json!({"max":v.last(),"p95":v.get(v.len()*95/100),"p50":v.get(v.len()/2)})
}
fn compare_optics(
    old: &Shape,
    new: &Shape,
    rays: &[Ray],
    output: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let reference_bounds = bounds(old);
    let a = optical_report(optical_scene(old.clone(), reference_bounds), rays)?;
    let b = optical_report(optical_scene(new.clone(), reference_bounds), rays)?;
    let n = a.budget.launched;
    let l1 = a
        .detectors
        .iter()
        .zip(&b.detectors)
        .flat_map(|(a, b)| a.weights.iter().zip(&b.weights))
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
        / n;
    fn dominant(report: &Report) -> std::collections::HashMap<u64, &raytrace_rs::trace::Terminal> {
        let mut result = std::collections::HashMap::new();
        for t in report.terminals.iter().filter(|t| t.reason == "detected") {
            let slot = result.entry(t.ray.id).or_insert(t);
            if t.ray.weight > slot.ray.weight {
                *slot = t;
            }
        }
        result
    }
    let da = dominant(&a);
    let db = dominant(&b);
    let (mut positions, mut angles, mut opl) = (Vec::new(), Vec::new(), Vec::new());
    let mut changed_collector = 0;
    for (id, t) in &da {
        if let Some(u) = db.get(id) {
            if t.surface != u.surface {
                changed_collector += 1;
            }
            positions.push((t.ray.position_mm - u.ray.position_mm).norm());
            angles.push(
                t.ray
                    .direction
                    .dot(u.ray.direction)
                    .clamp(-1., 1.)
                    .acos()
                    .to_degrees(),
            );
            opl.push((t.ray.optical_path_mm - u.ray.optical_path_mm).abs());
        }
    }
    let mean_opl = |r: &Report| {
        r.terminals
            .iter()
            .filter(|t| t.reason == "detected")
            .map(|t| t.ray.weight * t.ray.optical_path_mm)
            .sum::<f64>()
            / r.budget.detected
    };
    fs::write(
        output.with_extension("reference_report.json"),
        serde_json::to_vec(&a)?,
    )?;
    fs::write(
        output.with_extension("native_report.json"),
        serde_json::to_vec(&b)?,
    )?;
    Ok(
        serde_json::json!({"definition":"All differences in power are divided by the fixed launch weight. Synthetic dispersive material n(450nm)=1.52, n(650nm)=1.49, absorption=0.001/mm; six surrounding 32x32 detectors.",
        "rays":rays.len(),"reference_budget":a.budget,"native_budget":b.budget,"detector_l1_per_launch":l1,
        "detected_difference_per_launch":(b.budget.detected-a.budget.detected)/n,"bulk_absorption_difference_per_launch":(b.budget.bulk_absorbed-a.budget.bulk_absorbed)/n,
        "reference_budget_closure":(a.budget.accounted()-n)/n,"native_budget_closure":(b.budget.accounted()-n)/n,
        "reference_mean_detected_opl_mm":mean_opl(&a),"native_mean_detected_opl_mm":mean_opl(&b),
        "dominant_branches":{"compared":positions.len(),"changed_collector":changed_collector,"position_error_mm":statistics(&mut positions),"direction_error_deg":statistics(&mut angles),"optical_path_error_mm":statistics(&mut opl)}}),
    )
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = env::args().skip(1).collect();
    let optical = args.iter().any(|s| s == "--optical");
    args.retain(|s| s != "--optical");
    if args.first().is_some_and(|s| s == "--generate-owned") {
        let directory = Path::new(args.get(1).ok_or("Output directory is required")?);
        fs::create_dir_all(directory)?;
        for (name, text) in owned::owned_cases()
            .into_iter()
            .chain(std::iter::once(("mapped_box", owned::mapped_box_step())))
        {
            fs::write(directory.join(format!("{name}.step")), text)?;
        }
        return Ok(());
    }
    if args.len() < 2 {
        return Err(
            "Usage: step_check INPUT.step OUTPUT.json [REFERENCE_MESH.json] [DEFLECTION_MM]".into(),
        );
    }
    let tolerance = args
        .get(3)
        .map(|s| s.parse::<f64>())
        .transpose()?
        .unwrap_or(0.02);
    let start = Instant::now();
    let reference_json = Path::new(&args[0])
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    let mesh = if reference_json {
        serde_json::from_slice(&fs::read(&args[0])?)?
    } else {
        cad::load_step_native_with_tolerance(Path::new(&args[0]), tolerance)?
    };
    let elapsed = start.elapsed().as_secs_f64();
    let Shape::TriangleMesh {
        vertices,
        triangles,
        ..
    } = &mesh
    else {
        unreachable!()
    };
    let output = Path::new(&args[1]);
    fs::write(output, serde_json::to_vec(&mesh)?)?;
    let mut result = serde_json::json!({"load_seconds":elapsed,"input_kind":if reference_json {"reference_json"}else{"native_step"},"vertices":vertices.len(),"triangles":triangles.len(),"deflection_mm":tolerance,"native_bounds":bounds(&mesh)});
    if let Some(reference) = args.get(2) {
        let old: Shape = serde_json::from_slice(&fs::read(reference)?)?;
        let a = MeshAccel::from_shape(&old).ok_or("Reference must be a triangle mesh")?;
        let b = MeshAccel::from_shape(&mesh).unwrap();
        let [lo, hi] = bounds(&old);
        let span = hi - lo;
        let margin = span.norm() + 1.;
        let (mut same_hit, mut only_old, mut only_new, mut neither) = (0, 0, 0, 0);
        let (mut positions, mut angles) = (Vec::new(), Vec::new());
        let mut rays = Vec::new();
        let mut worst = Vec::new();
        for axis in 0..3 {
            for sign in [-1., 1.] {
                for i in 0..17 {
                    for j in 0..17 {
                        let mut p = (lo + hi) * 0.5;
                        let x = (axis + 1) % 3;
                        let y = (axis + 2) % 3;
                        p.0[x] = lo.0[x] + span.0[x] * (i as f64 + 0.371) / 17.;
                        p.0[y] = lo.0[y] + span.0[y] * (j as f64 + 0.619) / 17.;
                        p.0[axis] = if sign > 0. {
                            lo.0[axis] - margin
                        } else {
                            hi.0[axis] + margin
                        };
                        let mut d = V3::default();
                        d.0[axis] = sign;
                        rays.push(Ray {
                            id: rays.len() as u64,
                            position_mm: p,
                            direction: d,
                            weight: 1. / 1734.,
                            wavelength_um: if (i + j + axis) % 2 == 0 { 0.45 } else { 0.65 },
                            medium: 0,
                            optical_path_mm: 0.,
                        });
                        match (a.hit_with_normals(p, d), b.hit_with_normals(p, d)) {
                            (Some(u), Some(v)) => {
                                same_hit += 1;
                                let error = (u.distance_mm - v.distance_mm).abs();
                                let angle =
                                    u.normal.dot(v.normal).clamp(-1., 1.).acos().to_degrees();
                                positions.push(error);
                                angles.push(angle);
                                worst.push(serde_json::json!({"ray":rays.last(),"position_error_mm":error,"normal_error_deg":angle,"reference_hit":p+d*u.distance_mm,"native_hit":p+d*v.distance_mm,"reference_normal":u.normal,"native_normal":v.normal}));
                            }
                            (Some(_), None) => only_old += 1,
                            (None, Some(_)) => only_new += 1,
                            (None, None) => neither += 1,
                        }
                    }
                }
            }
        }
        positions.sort_by(f64::total_cmp);
        angles.sort_by(f64::total_cmp);
        let stats = |v: &[f64]| serde_json::json!({"max":v.last(),"p95":v.get(v.len()*95/100),"p50":v.get(v.len()/2)});
        result["reference_comparison"] = serde_json::json!({"reference_bounds":[lo,hi],"rays":1734,"both_hit":same_hit,"only_reference_hit":only_old,"only_native_hit":only_new,"both_miss":neither,"hit_position_error_mm":stats(&positions),"normal_angle_error_deg":stats(&angles)});
        worst.sort_by(|a, b| {
            b["normal_error_deg"]
                .as_f64()
                .unwrap()
                .total_cmp(&a["normal_error_deg"].as_f64().unwrap())
        });
        worst.truncate(16);
        fs::write(
            output.with_extension("worst_hits.json"),
            serde_json::to_vec_pretty(&worst)?,
        )?;
        if optical {
            result["optical_comparison"] = compare_optics(&old, &mesh, &rays, output)?;
        }
    }
    fs::write(
        output.with_extension("summary.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
