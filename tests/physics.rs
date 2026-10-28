use raytrace_rs::geometry::{dielectric, intersect, sag};
use raytrace_rs::math::{Frame, Pose, V3};
use raytrace_rs::model::{Material, Mode, PreparedScene, Ray, Scene};
use raytrace_rs::trace::{TraceOptions, trace};
use serde_json::{Value, json};

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() < tolerance, "{a} != {b}");
}
fn plane(name: &str, z: f64, interaction: Value) -> Value {
    json!({"name":name, "pose":{"translation_mm":[0,0,z]}, "shape":{"type":"plane"},
        "aperture":{"type":"rectangle", "half_width_mm":100, "half_height_mm":100},
        "interaction": interaction})
}
fn detector(name: &str, z: f64) -> Value {
    plane(name, z, json!({"type":"detector", "pixels":[4,4]}))
}
fn glass(minus: usize, plus: usize) -> Value {
    json!({"type":"dielectric", "minus_medium":minus, "plus_medium":plus})
}
fn scene(mode: Mode, surfaces: Vec<Value>) -> PreparedScene {
    serde_json::from_value::<Scene>(
        json!({"name":"test", "reference_plane":"launch", "mode":mode,
        "materials":[{"type":"constant","n":1},{"type":"constant","n":1.5}],
        "surfaces":surfaces, "settings":{"min_relative_branch_weight":1e-14}}),
    )
    .unwrap()
    .prepare()
    .unwrap()
}
fn ray() -> Ray {
    Ray {
        id: 0,
        position_mm: V3([0.0, 0.0, -1.0]),
        direction: V3([0.0, 0.0, 1.0]),
        weight: 1.0,
        wavelength_um: 0.55,
        medium: 0,
        optical_path_mm: 0.0,
    }
}
fn terminals() -> TraceOptions {
    TraceOptions {
        keep_terminals: true,
        path_launch_count: 0,
    }
}

#[test]
fn snell_fresnel_and_critical_angle() {
    let normal = V3([0.0, 0.0, 1.0]);
    let a = 30f64.to_radians();
    let out = dielectric(V3([a.sin(), 0.0, a.cos()]), normal, 1.0, 1.5);
    near(out.transmitted.unwrap().0[0], a.sin() / 1.5, 1e-14);
    near(
        dielectric(normal, normal, 1.0, 1.5).reflectance,
        0.04,
        1e-14,
    );
    let a = 50f64.to_radians();
    let tir = dielectric(V3([a.sin(), 0.0, a.cos()]), normal, 1.5, 1.0);
    assert!(tir.transmitted.is_none());
    near(tir.reflectance, 1.0, 1e-14);
    near(tir.reflected.0[2], -a.cos(), 1e-14);
}

#[test]
fn oblique_plate_matches_analytic_position_and_optical_path() {
    let s = scene(
        Mode::Sequential,
        vec![
            plane("front", 0.0, glass(0, 1)),
            plane("rear", 5.0, glass(1, 0)),
            detector("sensor", 10.0),
        ],
    );
    let a = 25f64.to_radians();
    let mut r = ray();
    r.direction = V3([a.sin(), 0.0, a.cos()]);
    let report = trace(&s, &[r], terminals());
    let hit = report
        .terminals
        .iter()
        .find(|t| t.reason == "detected")
        .unwrap();
    let theta_t = (a.sin() / 1.5).asin();
    near(
        hit.ray.position_mm.0[0],
        6.0 * a.tan() + 5.0 * theta_t.tan(),
        1e-11,
    );
    near(hit.ray.direction.0[0], a.sin(), 1e-14);
    near(
        hit.ray.optical_path_mm,
        6.0 / a.cos() + 7.5 / theta_t.cos(),
        1e-11,
    );
    near(report.budget.accounted(), 1.0, 1e-13);
}

#[test]
fn nonsequential_slab_sums_infinite_internal_reflection_series() {
    let mut spec: Scene =
        serde_json::from_str(include_str!("../examples/nonsequential_slab.json")).unwrap();
    spec.sources.clear();
    let s = spec.prepare().unwrap();
    let report = trace(&s, &[ray()], terminals());
    let forward = report
        .detectors
        .iter()
        .find(|d| d.name == "transmitted_collector")
        .unwrap()
        .weights
        .iter()
        .sum::<f64>();
    let backward = report
        .detectors
        .iter()
        .find(|d| d.name == "reflected_collector")
        .unwrap()
        .weights
        .iter()
        .sum::<f64>();
    near(forward, 0.96 / 1.04, 1e-11);
    near(backward, 0.08 / 1.04, 1e-11);
    near(report.budget.accounted(), 1.0, 1e-13);
    assert_eq!(report.budget.medium_mismatch, 0.0);
    assert!(report.budget.branch_cutoff < 1e-11);
    let mut reversed = s.spec.clone();
    reversed.surfaces.reverse();
    let again = trace(
        &reversed.prepare().unwrap(),
        &[ray()],
        TraceOptions::default(),
    );
    near(again.budget.detected, report.budget.detected, 1e-13);
}

#[test]
fn sequential_primary_plate_and_reflection_budget() {
    let s = scene(
        Mode::Sequential,
        vec![
            plane("front", 0.0, glass(0, 1)),
            plane("rear", 1.0, glass(1, 0)),
            detector("sensor", 2.0),
        ],
    );
    let report = trace(&s, &[ray()], TraceOptions::default());
    near(report.budget.detected, 0.96 * 0.96, 1e-14);
    near(
        report.budget.sequential_reflection_not_followed,
        1.0 - 0.96 * 0.96,
        1e-14,
    );
    near(report.budget.accounted(), 1.0, 1e-14);
}

#[test]
fn absorption_and_trace_limit_preserve_budget() {
    let s = scene(
        Mode::Sequential,
        vec![
            plane("front", 0.0, glass(0, 1)),
            plane("rear", 5.0, glass(1, 0)),
            detector("sensor", 10.0),
        ],
    );
    let mut spec = s.spec;
    spec.materials[1] = Material::Constant {
        n: 1.5,
        absorption_per_mm: 0.1,
    };
    let report = trace(&spec.prepare().unwrap(), &[ray()], TraceOptions::default());
    near(
        report.budget.detected,
        0.96f64.powi(2) * (-0.5f64).exp(),
        1e-14,
    );
    near(report.budget.accounted(), 1.0, 1e-14);
    let mut spec = scene(
        Mode::NonSequential,
        vec![
            plane("left", 0.0, json!({"type":"mirror","reflectance":1})),
            plane("right", 2.0, json!({"type":"mirror","reflectance":1})),
        ],
    )
    .spec;
    spec.settings.max_events_per_launch = 7;
    let mut r = ray();
    r.position_mm.0[2] = 1.0;
    let report = trace(&spec.prepare().unwrap(), &[r], TraceOptions::default());
    near(report.budget.interaction_limit, 1.0, 1e-14);
    assert_eq!(report.surface_intersections, 7);
}

#[test]
fn tilted_mirror_is_identical_in_both_modes() {
    let mut spec: Scene =
        serde_json::from_str(include_str!("../examples/folded_mirror.json")).unwrap();
    let r = ray();
    for mode in [Mode::Sequential, Mode::NonSequential] {
        spec.mode = mode;
        let report = trace(&spec.clone().prepare().unwrap(), &[r], terminals());
        near(report.budget.detected, 0.98, 1e-13);
        let hit = report
            .terminals
            .iter()
            .find(|t| t.reason == "detected")
            .unwrap();
        near(hit.ray.position_mm.0[0], 40.0, 1e-12);
        near(hit.ray.position_mm.0[2], 0.0, 1e-12);
        near(hit.ray.direction.0[0], 1.0, 1e-13);
    }
}

#[test]
fn dispersion_and_no_extrapolation() {
    let silica = Material::Sellmeier {
        b: [0.6961663, 0.4079426, 0.8974794],
        c_um2: [0.004679148, 0.013512063, 97.9340025],
        valid_range_um: [0.4, 0.7],
        absorption_per_mm: 0.0,
    };
    near(silica.index(0.465).unwrap(), 1.464483960997639, 1e-14);
    assert!(silica.index(0.45).unwrap() > silica.index(0.635).unwrap());
    assert!(silica.index(0.8).is_none());
    let table = Material::Tabulated {
        samples: vec![[0.4, 1.6], [0.6, 1.5]],
        absorption_per_mm: 0.0,
    };
    near(table.index(0.5).unwrap(), 1.55, 1e-14);
    near(table.index(0.6).unwrap(), 1.5, 1e-14);
    assert!(table.index(0.3).is_none());
    let schott = Material::Schott {
        coefficients: [2.25, 0.0, 0.01, 0.0, 0.0, 0.0],
        valid_range_um: [0.4, 0.7],
        absorption_per_mm: 0.0,
    };
    near(schott.index(0.5).unwrap(), 2.29f64.sqrt(), 1e-14);
}

#[test]
fn dispersive_lens_changes_ray_position_and_fresnel_weight() {
    let spec: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    for mode in [Mode::Sequential, Mode::NonSequential] {
        let mut spec = spec.clone();
        spec.mode = mode;
        let s = spec.prepare().unwrap();
        let mut r = ray();
        r.position_mm = V3([3.0, 0.0, 0.0]);
        r.wavelength_um = 0.45;
        let blue = trace(&s, &[r], terminals());
        r.wavelength_um = 0.635;
        let red = trace(&s, &[r], terminals());
        let blue_hit = blue
            .terminals
            .iter()
            .find(|t| t.reason == "detected")
            .unwrap();
        let red_hit = red
            .terminals
            .iter()
            .find(|t| t.reason == "detected")
            .unwrap();
        assert!((blue_hit.ray.position_mm.0[0] - red_hit.ray.position_mm.0[0]).abs() > 0.01);
        assert!((blue_hit.ray.weight - red_hit.ray.weight).abs() > 1e-4);
        assert_eq!(
            blue.budget.numerical_failure + blue.budget.medium_mismatch,
            0.0
        );
        near(blue.budget.accounted(), 1.0, 1e-12);
    }
}

#[test]
fn asphere_intersection_obeys_sag_and_backward_rays_miss() {
    let mut front = plane("asphere", 0.0, glass(0, 1));
    front["shape"] =
        json!({"type":"even_asphere","radius_mm":25.0,"conic":-0.5,"coefficients":[2e-6]});
    front["aperture"] = json!({"type":"circle","radius_mm":10.0});
    let s = scene(Mode::Sequential, vec![front]);
    let p = V3([3.0, 2.0, -5.0]);
    let hit = intersect(&s.surfaces[0], p, V3([0.0, 0.0, 1.0])).unwrap();
    let expected = sag(&s.surfaces[0].spec.shape, 3.0, 2.0).unwrap().0;
    near(hit.point.0[2], expected, 1e-11);
    assert!(intersect(&s.surfaces[0], p, V3([0.0, 0.0, -1.0])).is_err());
    let missed = trace(
        &s,
        &[Ray {
            position_mm: V3([50.0, 0.0, -5.0]),
            ..ray()
        }],
        TraceOptions::default(),
    );
    near(missed.budget.geometric_miss, 1.0, 1e-14);
}

#[test]
fn frame_inverse_and_detector_edge() {
    let f = Frame::new(&Pose {
        translation_mm: V3([2.0, 3.0, 4.0]),
        rotation_deg: V3([23.0, -47.0, 81.0]),
    });
    let p = V3([6.0, 7.0, 8.0]);
    assert!((f.local_point(f.world_point(p)) - p).norm() < 1e-13);
    let s = scene(Mode::Sequential, vec![detector("sensor", 1.0)]);
    let report = trace(
        &s,
        &[Ray {
            position_mm: V3([100.0, 100.0, 0.0]),
            ..ray()
        }],
        TraceOptions::default(),
    );
    near(report.detectors[0].weights[15], 1.0, 1e-14);
}

#[test]
fn oblique_paraboloid_matches_nearest_analytic_root_and_does_not_rehit_itself() {
    let mut front = plane("paraboloid", 0., glass(0, 1));
    front["shape"] = json!({"type":"even_asphere","radius_mm":25.,"conic":-1.,"coefficients":[]});
    front["aperture"] = json!({"type":"circle","radius_mm":10.});
    let s = scene(Mode::Sequential, vec![front]);
    for x in [-12., -5., 0., 5., 12.] {
        for z in [0., 0.000001, 0.5, 1.5, 3.] {
            for dz in [-0.2, -0.01, 0., 0.01, 0.2] {
                let p = V3([x, 0., z]);
                let d = V3([1., 0., dz]).unit();
                let a = d.0[0] * d.0[0] / 50.;
                let b = 2. * x * d.0[0] / 50. - d.0[2];
                let c = x * x / 50. - z;
                let mut disc = b * b - 4. * a * c;
                // Coalesce a double root when discriminant cancellation is within
                // floating-point uncertainty; the stationary point is its analytic limit.
                if disc.abs() <= 32. * f64::EPSILON * (b * b + (4. * a * c).abs()) {
                    disc = 0.;
                }
                let expected = if disc < 0. {
                    None
                } else {
                    let q = -0.5 * (b + disc.sqrt().copysign(b));
                    let mut roots = if q == 0. {
                        [-b / (2. * a); 2]
                    } else {
                        [q / a, c / q]
                    };
                    roots.sort_by(f64::total_cmp);
                    roots
                        .into_iter()
                        .find(|t| *t > 1e-8 && (x + d.0[0] * t).abs() <= 10. + 1e-10)
                };
                let hit = intersect(&s.surfaces[0], p, d);
                match expected {
                    Some(t) => {
                        let h =
                            hit.unwrap_or_else(|e| panic!("{p:?} {d:?}: expected {t}, got {e:?}"));
                        near(h.distance_mm, t, 2e-10);
                        let result = dielectric(d, h.normal, 1., 1.5);
                        for outgoing in [Some(result.reflected), result.transmitted]
                            .into_iter()
                            .flatten()
                        {
                            if let Ok(next) = intersect(&s.surfaces[0], h.point, outgoing) {
                                assert!(
                                    next.distance_mm > 1e-6,
                                    "False self-hit: {h:?}, next {next:?}"
                                );
                            }
                        }
                    }
                    None => assert!(
                        matches!(hit, Err(raytrace_rs::geometry::Miss::Geometry)),
                        "{p:?} {d:?}: {hit:?}"
                    ),
                }
            }
        }
    }
}

#[test]
fn monotone_grazing_asphere_finishes_when_the_root_bracket_reaches_precision() {
    let mut front = plane("owned paraboloid", 0., glass(0, 1));
    front["shape"] = json!({"type":"even_asphere","radius_mm":25.,"conic":-1.,"coefficients":[]});
    front["aperture"] = json!({"type":"circle","radius_mm":10.});
    let s = scene(Mode::Sequential, vec![front]);
    for radius in [9.8, 9.99, 9.999] {
        for slope in [0.40001, 0.4001, 0.401] {
            for distance in [7.3, 20.7, 50.3] {
                for angle in [0_f64, 0.3, 1.2] {
                    let target = V3([
                        radius * angle.cos(),
                        radius * angle.sin(),
                        radius * radius / 50.,
                    ]);
                    let direction = V3([angle.cos(), angle.sin(), slope]).unit();
                    let origin = target - direction * distance;
                    let hit = intersect(&s.surfaces[0], origin, direction).unwrap_or_else(|e| {
                        panic!("radius={radius}, slope={slope}, distance={distance}, angle={angle}: {e:?}")
                    });
                    near(hit.distance_mm, distance, 2e-8);
                    near((hit.point - target).norm(), 0., 2e-8);
                    // Once it leaves the aperture, this monotone ray has no second root.
                    assert!(matches!(
                        intersect(&s.surfaces[0], hit.point, direction),
                        Err(raytrace_rs::geometry::Miss::Geometry)
                    ));
                }
            }
        }
    }
}

#[test]
fn wrong_medium_is_reported_and_invalid_configuration_is_rejected() {
    let s = scene(Mode::Sequential, vec![plane("glass", 0.0, glass(0, 1))]);
    let report = trace(&s, &[Ray { medium: 1, ..ray() }], TraceOptions::default());
    near(report.budget.medium_mismatch, 1.0, 1e-14);
    let mut spec = s.spec;
    spec.settings.max_depth = 0;
    assert!(spec.prepare().is_err());
}

#[test]
fn nsc_event_allowance_resolves_strong_reflections_before_weak_descendants() {
    let interface = |minus, plus| json!({"type":"dielectric","minus_medium":minus,"plus_medium":plus,"reflectance":0.5});
    let mut prepared = scene(
        Mode::NonSequential,
        vec![
            plane("front", 0., interface(0, 1)),
            plane("rear", 1., interface(1, 0)),
            detector("forward", 2.),
            detector("backward", -2.),
        ],
    );
    prepared.spec.settings.max_events_per_launch = 8;
    let limited = trace(&prepared, &[ray()], terminals());
    assert!(limited.budget.detected >= 0.875);
    assert!(limited.budget.interaction_limit <= 0.125);
    near(limited.budget.accounted(), 1., 1e-14);
    prepared.spec.settings.max_events_per_launch = 256;
    let complete = trace(&prepared, &[ray()], terminals());
    near(complete.budget.detected, 1., 1e-12);
    near(
        complete.detectors[0].weights.iter().sum::<f64>(),
        1. / 3.,
        1e-12,
    );
    near(
        complete.detectors[1].weights.iter().sum::<f64>(),
        2. / 3.,
        1e-12,
    );
    assert_eq!(complete.budget.interaction_limit, 0.);
}

#[test]
fn interpolated_mesh_normals_preserve_the_geometric_boundary_side() {
    for normal in [V3([0.6, 0., 0.8]), V3([0.25, 0., 1.]).unit()] {
        let mut face = plane("mesh entrance", 0., glass(1, 0));
        face["shape"] = json!({"type":"triangle_mesh","vertices":[[-100.,-100.,0.],[100.,-100.,0.],[100.,100.,0.],[-100.,100.,0.]],"triangles":[[0,1,2],[0,2,3]],"normals":[normal,normal,normal,normal]});
        let mut s = scene(
            Mode::NonSequential,
            vec![
                face,
                plane("rear", -1., glass(0, 1)),
                detector("forward", -2.),
                detector("backward", 2.),
            ],
        );
        s.spec.settings.max_depth = 256;
        let r = Ray {
            position_mm: V3([-1., 0., 1.]),
            direction: V3([1., 0., -0.3]).unit(),
            ..ray()
        };
        let report = trace(&s, &[r], terminals());
        assert_eq!(report.budget.medium_mismatch, 0.);
        assert_eq!(report.budget.numerical_failure, 0.);
        assert!(report.surfaces[0].normal_fallbacks > 0);
        // Finite apertures let some off-axis descendants leave the model.
        near(report.budget.detected + report.budget.escaped, 1., 1e-10);
        near(report.budget.accounted(), 1., 1e-12);
    }
}

#[test]
fn dispersive_rod_matches_unfolded_path_and_wavelength_dependent_optical_length() {
    let spec: Scene =
        serde_json::from_str(include_str!("../examples/integrator_rod.json")).unwrap();
    let s = spec.prepare().unwrap();
    let theta = 10f64.to_radians();
    let mut exit_x = Vec::new();
    for wavelength_um in [0.45, 0.635] {
        let r = Ray {
            position_mm: V3([0.05, 0.0, -2.0]),
            direction: V3([theta.sin(), 0.0, theta.cos()]),
            wavelength_um,
            ..ray()
        };
        let n = s.spec.materials[1].index(wavelength_um).unwrap();
        let sint = theta.sin() / n;
        let cost = (1.0 - sint * sint).sqrt();
        let unfolded = 0.05 + 2.0 * theta.tan() + 0.5 + 30.0 * sint / cost;
        let folded = 1.0 - (unfolded.rem_euclid(2.0) - 1.0).abs() - 0.5;
        let sign = if (unfolded.floor() as i64) % 2 == 0 {
            1.0
        } else {
            -1.0
        };
        let report = trace(&s, &[r], terminals());
        let forward = s
            .surfaces
            .iter()
            .position(|s| s.spec.name == "forward_collector")
            .unwrap();
        let primary = report
            .terminals
            .iter()
            .filter(|t| t.reason == "detected" && t.surface == Some(forward))
            .max_by(|a, b| a.ray.weight.total_cmp(&b.ray.weight))
            .unwrap();
        near(
            primary.ray.position_mm.0[0],
            folded + 5.0 * theta.tan() * sign,
            1e-8,
        );
        near(
            primary.ray.optical_path_mm,
            7.0 / theta.cos() + 30.0 * n / cost,
            1e-8,
        );
        assert!(report.total_internal_reflections > 0);
        assert_eq!(
            report.budget.medium_mismatch + report.budget.numerical_failure,
            0.0
        );
        near(report.budget.accounted(), 1.0, 1e-12);
        exit_x.push(primary.ray.position_mm.0[0]);
    }
    assert!((exit_x[0] - exit_x[1]).abs() > 0.005);
}

#[test]
fn dispersion_can_change_total_internal_reflection_state() {
    // An artificial dispersive material isolates the critical-angle behaviour.
    let material = Material::Tabulated {
        samples: vec![[0.45, 1.5], [0.65, 1.3]],
        absorption_per_mm: 0.0,
    };
    let a = 45f64.to_radians();
    let d = V3([a.sin(), 0.0, a.cos()]);
    assert!(
        dielectric(d, V3([0.0, 0.0, 1.0]), material.index(0.45).unwrap(), 1.0)
            .transmitted
            .is_none()
    );
    assert!(
        dielectric(d, V3([0.0, 0.0, 1.0]), material.index(0.65).unwrap(), 1.0)
            .transmitted
            .is_some()
    );
}

#[test]
fn thread_count_preserves_results_and_sampling() {
    let mut spec: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    for source in &mut spec.sources {
        source.count = 100;
    }
    let scene = spec.prepare().unwrap();
    let rays = scene.generate_rays().unwrap();
    let run = |n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
            .install(|| trace(&scene, &rays, TraceOptions::default()))
    };
    let one = run(1);
    let four = run(4);
    assert_eq!(one.budget.detected, four.budget.detected);
    assert_eq!(one.detectors[0].weights, four.detectors[0].weights);
    let again = scene.generate_rays().unwrap();
    assert_eq!(rays[99].position_mm, again[99].position_mm);
}

#[test]
fn spherical_sag_cap_from_distant_source_and_reversed_direction() {
    let mut surface = plane("sphere", 0.0, glass(0, 1));
    surface["shape"] = json!({"type":"sphere","radius_mm":50});
    surface["aperture"] = json!({"type":"circle","radius_mm":10});
    let s = scene(Mode::Sequential, vec![surface]);
    let expected = 50.0 - (2500.0f64 - 9.0).sqrt();
    let forward = intersect(
        &s.surfaces[0],
        V3([3.0, 0.0, -100000.0]),
        V3([0.0, 0.0, 1.0]),
    )
    .unwrap();
    let backward = intersect(
        &s.surfaces[0],
        V3([3.0, 0.0, 100000.0]),
        V3([0.0, 0.0, -1.0]),
    )
    .unwrap();
    near(forward.point.0[2], expected, 1e-9);
    near(backward.point.0[2], expected, 1e-9);
}

#[test]
fn farther_numerical_asphere_does_not_discard_nearer_detector() {
    let mut front = detector("near", 0.);
    front["pose"] = json!({"translation_mm":[-15.,0.,0.],"rotation_deg":[0.,90.,0.]});
    let mut far = plane("far", 0., glass(0, 1));
    // Cancellation near the polynomial minimum exhausts the oblique search.
    far["shape"] =
        json!({"type":"even_asphere","radius_mm":25.,"conic":-1.,"coefficients":[1e8,-2e8,1e8]});
    far["aperture"] = json!({"type":"circle","radius_mm":10.});
    let s = scene(Mode::NonSequential, vec![front, far]);
    let r = Ray {
        position_mm: V3([-20., 1., 0.]),
        direction: V3([1., 0., 0.]),
        ..ray()
    };
    assert!(matches!(
        intersect(&s.surfaces[1], r.position_mm, r.direction),
        Err(raytrace_rs::geometry::Miss::Numerical)
    ));
    let report = trace(&s, &[r], terminals());
    near(report.detectors[0].weights.iter().sum(), 1., 1e-14);
    assert_eq!(report.budget.numerical_failure, 0.);
    // Without the nearer detector, the unresolved search still terminates the ray.
    let mut spec = s.spec;
    spec.surfaces.remove(0);
    let report = trace(&spec.prepare().unwrap(), &[r], terminals());
    near(report.budget.numerical_failure, 1., 1e-14);
    assert_eq!(report.diagnostics[0].reason, "numerical_failure");
}

#[test]
fn sequential_tir_terminates_inside_glass_and_closes_budget() {
    let mut front = plane("entrance", 0., glass(0, 1));
    front["interaction"]["reflectance"] = json!(0.);
    let mut exit = plane("exit", 1., glass(1, 0));
    exit["pose"]["rotation_deg"] = json!([0., 50., 0.]);
    let s = scene(
        Mode::Sequential,
        vec![front, exit, plane("next", -2., glass(1, 0))],
    );
    let report = trace(&s, &[ray()], terminals());
    near(report.budget.sequential_tir, 1., 1e-14);
    assert_eq!(report.total_internal_reflections, 1);
    assert_eq!(report.budget.medium_mismatch, 0.);
    assert_eq!(report.terminals[0].reason, "sequential_tir");
    assert_eq!(report.diagnostics[0].reason, "sequential_tir");
    near(report.budget.accounted(), report.budget.launched, 1e-14);
    let mut old = serde_json::to_value(&report.budget).unwrap();
    old.as_object_mut().unwrap().remove("sequential_tir");
    assert_eq!(
        serde_json::from_value::<raytrace_rs::trace::Budget>(old)
            .unwrap()
            .sequential_tir,
        0.
    );
}

#[test]
fn path_launch_count_selects_input_order_with_arbitrary_ids() {
    let s = scene(Mode::Sequential, vec![detector("sensor", 1.)]);
    let rays = [1000, 5, 7].map(|id| Ray { id, ..ray() });
    let report = trace(
        &s,
        &rays,
        TraceOptions {
            path_launch_count: 1,
            ..TraceOptions::default()
        },
    );
    assert_eq!(report.paths.len(), 1);
    assert_eq!(report.paths[0].id, 1000);
}

#[test]
fn tiny_closed_mesh_intersects_at_oblique_incidence() {
    let mut face = plane("tetrahedron", 0., json!({"type":"absorb"}));
    face["shape"] = json!({"type":"triangle_mesh","vertices":[[0.,0.,0.],[1e-4,0.,0.],[0.,1e-4,0.],[0.,0.,1e-4]],"triangles":[[0,2,1],[0,1,3],[0,3,2],[1,2,3]],"normals":[[0.,0.,-1.],[0.,0.,-1.],[0.,0.,-1.],[0.,0.,1.]]});
    let s = scene(Mode::NonSequential, vec![face]);
    let d = V3([1., 0., 1e-7]).unit();
    let target = V3([2e-5, 2e-5, 0.]);
    let r = Ray {
        position_mm: target - d * 1e-3,
        direction: d,
        ..ray()
    };
    let h = intersect(&s.surfaces[0], r.position_mm, r.direction).unwrap();
    near(h.distance_mm, 1e-3, 1e-14);
    let report = trace(&s, &[r], TraceOptions::default());
    near(report.budget.surface_absorbed, 1., 1e-14);
}
