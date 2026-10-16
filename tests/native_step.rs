use raytrace_rs::{cad, math::V3, mesh::MeshAccel, model::Shape};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
#[path = "support/step.rs"]
mod owned;
fn file(name: &str, text: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "raytrace_native_step_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}
fn volume(shape: &Shape) -> f64 {
    let Shape::TriangleMesh {
        vertices,
        triangles,
        ..
    } = shape
    else {
        panic!("expected mesh")
    };
    triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            a.dot(b.cross(c)) / 6.
        })
        .sum()
}
#[test]
fn owned_step_solids_have_analytic_volume_units_and_repeatable_meshes() {
    for (name, text) in owned::owned_cases() {
        let path = file(&format!("{name}.step"), &text);
        let a = cad::load_step_native_with_tolerance(&path, 0.02)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let b = cad::load_step_native_with_tolerance(&path, 0.02).unwrap();
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap(),
            "{name}"
        );
        let expected = match name {
            "box_mm" => 24.,
            "box_inch" => 24. * 25.4f64.powi(3),
            "tube" => 21. * std::f64::consts::PI,
            "sphere_cap" => 68. / 3. * std::f64::consts::PI,
            "torus" => 6. * std::f64::consts::PI.powi(2),
            "cone" => 4. / 3. * std::f64::consts::PI * (6.25 + 3.75 + 2.25),
            _ => 18. * std::f64::consts::PI,
        };
        assert!(
            (volume(&a) / expected - 1.).abs() < 0.01,
            "{name}: {} vs {expected}",
            volume(&a)
        );
    }
}
#[test]
fn native_step_rational_periodic_surface_matches_sphere_intersections_and_normals() {
    let path = file("dome.step", &owned::nurbs_dome_step());
    let shape = cad::load_step_native_with_tolerance(&path, 0.01).unwrap();
    let mesh = MeshAccel::from_shape(&shape).unwrap();
    for [x, y] in [[0., 0.], [0.7, 0.3], [-1.1, 0.8], [1.5, -0.6]] {
        let origin = V3([x, y, 5.]);
        let direction = V3([0., 0., -1.]);
        let hit = mesh.hit_with_normals(origin, direction).unwrap();
        let expected = V3([x, y, (9. - x * x - y * y).sqrt()]);
        assert!((hit.distance_mm - (5. - expected.0[2])).abs() < 0.011);
        assert!(
            hit.normal.dot(expected.unit()) > 1. - 2e-5,
            "normal {:?} expected {:?}",
            hit.normal,
            expected.unit()
        );
    }
}
#[test]
fn native_step_mapped_placement_transforms_vertices_and_normals() {
    let path = file("mapped.step", &owned::mapped_box_step());
    let shape = cad::load_step_native_with_tolerance(&path, 0.02).unwrap();
    let Shape::TriangleMesh { vertices, .. } = &shape else {
        panic!()
    };
    let lo = V3(std::array::from_fn(|a| {
        vertices
            .iter()
            .map(|p| p.0[a])
            .fold(f64::INFINITY, f64::min)
    }));
    let hi = V3(std::array::from_fn(|a| {
        vertices
            .iter()
            .map(|p| p.0[a])
            .fold(f64::NEG_INFINITY, f64::max)
    }));
    assert_eq!(lo, V3([6., 11., 11.]));
    assert_eq!(hi, V3([8., 14., 15.]));
    assert!((volume(&shape) - 24.).abs() < 1e-10);
    let mesh = MeshAccel::from_shape(&shape).unwrap();
    let hit = mesh
        .hit_with_normals(V3([7., 20., 13.]), V3([0., -1., 0.]))
        .unwrap();
    assert!((hit.distance_mm - 6.).abs() < 1e-10);
    assert_eq!(hit.normal, V3([0., 1., 0.]));
}
#[test]
fn native_step_tube_curvature_converges_for_oblique_rays() {
    let path = file("tube.step", &owned::tube_step());
    let mut previous = f64::INFINITY;
    for tolerance in [0.08, 0.02, 0.005] {
        let shape = cad::load_step_native_with_tolerance(&path, tolerance).unwrap();
        let mesh = MeshAccel::from_shape(&shape).unwrap();
        let mut error: f64 = 0.;
        for y in [0.3, 1.4, 2., 2.45] {
            let origin = V3([5., y, 1.234]);
            let direction = V3([-1., 0., 0.]);
            let hit = mesh.hit_with_normals(origin, direction).unwrap();
            let x = (6.25 - y * y).sqrt();
            let point = origin + direction * hit.distance_mm;
            error = error.max((point.0[0] - x).abs());
            assert!(hit.normal.dot(V3([x, y, 0.]).unit()) > 0.9999);
        }
        assert!(error <= previous + 1e-10);
        assert!(error < tolerance * 0.8);
        previous = error;
    }
}
#[test]
fn native_step_rejects_unsupported_geometry_and_invalid_topology() {
    let good = owned::box_step(false);
    for text in [
        good.replace("PLANE(", "OFFSET_SURFACE("),
        good.replace("CLOSED_SHELL(", "OPEN_SHELL("),
        good.replace("SI_UNIT(.MILLI.,.METRE.)", "SI_UNIT(.MILLI.,.SECOND.)"),
    ] {
        let path = file("invalid.step", &text);
        assert!(cad::load_step_native_with_tolerance(&path, 0.02).is_err());
    }
    let path = file("good.step", &good);
    for tolerance in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(cad::load_step_native_with_tolerance(&path, tolerance).is_err());
    }
}

#[test]
fn native_step_matches_900_frozen_legacy_first_hits() {
    #[derive(serde::Deserialize)]
    struct Hit {
        distance_mm: f64,
        normal: V3,
    }
    #[derive(serde::Deserialize)]
    struct Probe {
        origin_mm: V3,
        direction: V3,
        hit: Option<Hit>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        rays: Vec<Probe>,
    }
    #[derive(serde::Deserialize)]
    struct Reference {
        deflection_mm: f64,
        cases: Vec<Case>,
    }
    // Captured from the previous FreeCAD bridge using only our own solids.
    // Independent Moller-Trumbore evaluation froze the old mesh's first hits.
    let reference: Reference =
        serde_json::from_str(include_str!("fixtures/step_legacy_hits.json")).unwrap();
    let originals = owned::owned_cases();
    let mut compared = 0;
    for case in reference.cases {
        let text = &originals
            .iter()
            .find(|(name, _)| *name == case.name)
            .unwrap()
            .1;
        let path = file(&format!("{}.step", case.name), text);
        let shape = cad::load_step_native_with_tolerance(&path, reference.deflection_mm).unwrap();
        let mesh = MeshAccel::from_shape(&shape).unwrap();
        let minimum_radius = match case.name.as_str() {
            "tube" | "torus" => 1.,
            "cone" => 1.5,
            "sphere_cap" => 3.,
            _ => f64::INFINITY,
        };
        for (i, probe) in case.rays.into_iter().enumerate() {
            let actual = mesh.hit_with_normals(probe.origin_mm, probe.direction);
            assert_eq!(
                actual.is_some(),
                probe.hit.is_some(),
                "{} ray {i}",
                case.name
            );
            if let (Some(a), Some(b)) = (actual, probe.hit) {
                let cosine = b.normal.dot(probe.direction).abs().max(0.05);
                // Each curved tessellation has the specified chord tolerance;
                // incidence converts this to a first-hit position bound.
                let position_limit = if minimum_radius.is_infinite() {
                    1e-10
                } else {
                    2. * reference.deflection_mm / cosine
                };
                assert!(
                    (a.distance_mm - b.distance_mm).abs() <= position_limit,
                    "{} ray {i}",
                    case.name
                );
                let angle = a.normal.dot(b.normal).clamp(-1., 1.).acos();
                let normal_limit = if minimum_radius.is_infinite() {
                    1e-7
                } else {
                    0.005 + position_limit / minimum_radius
                };
                assert!(
                    angle <= normal_limit,
                    "{} ray {i}: angle {angle}",
                    case.name
                );
            }
            compared += 1;
        }
    }
    assert_eq!(compared, 900);
}
