use raytrace_rs::{
    math::{Frame, Pose, V3},
    model::{Material, Mode, Ray, Scene},
    trace::{trace, TraceOptions},
};
use serde_json::json;

fn pad(scene: &Scene) -> Scene {
    let mut padded = scene.clone();
    for i in 0..64 {
        padded.surfaces.push(serde_json::from_value(json!({
            "name":format!("unused_{i}"), "pose":{"translation_mm":[10000.0+i as f64*10.0,0,0]},
            "shape":{"type":"plane"}, "aperture":{"type":"rectangle","half_width_mm":1,"half_height_mm":1},
            "interaction":{"type":"absorb"}
        })).unwrap());
    }
    padded
}

#[test]
fn indexed_systems_keep_every_terminal_path_pixel_and_branch_order() {
    for fixture in [
        include_str!("../examples/sequential_lens.json"),
        include_str!("../examples/nonsequential_slab.json"),
        include_str!("../examples/integrator_rod.json"),
        include_str!("../examples/folded_mirror.json"),
    ] {
        let mut scene: Scene = serde_json::from_str(fixture).unwrap();
        scene.mode = Mode::NonSequential;
        for source in &mut scene.sources {
            source.count = 257;
        }
        for rotated in [false, true] {
            let mut placed = scene.clone();
            if rotated {
                let frame = Frame::new(&Pose {
                    translation_mm: V3([8.0, -4.0, 3.0]),
                    rotation_deg: V3([17.0, -23.0, 8.0]),
                });
                for surface in &mut placed.surfaces {
                    surface.pose = frame.transform_pose(&surface.pose);
                }
                for source in &mut placed.sources {
                    source.origin_mm = frame.world_point(source.origin_mm);
                    source.direction = frame.world_vector(source.direction);
                }
            }
            let base = placed.clone().prepare().unwrap();
            let indexed = pad(&placed).prepare().unwrap();
            let rays = base.generate_rays().unwrap();
            let mut previous = None;
            for workers in [1, 4] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .build()
                    .unwrap();
                let options = TraceOptions {
                    keep_terminals: true,
                    path_launch_count: 32,
                };
                let expected = pool.install(|| trace(&base, &rays, options));
                let mut actual = pool.install(|| trace(&indexed, &rays, options));
                actual.surfaces.truncate(base.surfaces.len());
                let expected = serde_json::to_vec(&expected).unwrap();
                assert_eq!(
                    serde_json::to_vec(&actual).unwrap(),
                    expected,
                    "rotation={rotated}, workers={workers}"
                );
                if let Some(previous) = &previous {
                    assert_eq!(&expected, previous);
                }
                previous = Some(expected);
            }
        }
    }
}

#[test]
fn wavelengths_beyond_cache_capacity_keep_dispersion_and_absorption() {
    let mut scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    scene.sources.clear();
    scene.materials[1] = Material::WithAbsorption {
        dispersion: Box::new(scene.materials[1].clone()),
        samples: vec![[0.4, 0.0001], [0.7, 0.0003]],
    };
    for (index, z) in [10.0, 15.0, 20.0].into_iter().enumerate() {
        scene.surfaces[index].shape = raytrace_rs::model::Shape::Plane;
        scene.surfaces[index].pose.translation_mm = V3([0.0, 0.0, z]);
        scene.surfaces[index].aperture = raytrace_rs::model::Aperture::Circle { radius_mm: 10.0 };
        if let raytrace_rs::model::Interaction::Dielectric { reflectance, .. } =
            &mut scene.surfaces[index].interaction
        {
            *reflectance = Some(0.005);
        }
    }
    let scene = scene.prepare().unwrap();
    // Each of 31 chunks contains 300 distinct wavelengths, exceeding its 256-entry budget.
    let rays: Vec<_> = (0..9300)
        .map(|i| Ray {
            id: i,
            position_mm: V3([0.0, 0.0, 0.0]),
            direction: V3([0.0, 0.0, 1.0]),
            weight: 1.0 / 9300.0,
            wavelength_um: 0.4 + (i % 400) as f64 * 0.0007,
            medium: 0,
            optical_path_mm: 0.0,
        })
        .collect();
    let mut previous = None;
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let report = pool.install(|| {
            trace(
                &scene,
                &rays,
                TraceOptions {
                    keep_terminals: true,
                    path_launch_count: 0,
                },
            )
        });
        assert_eq!(report.terminals.len(), rays.len());
        for terminal in &report.terminals {
            assert_eq!(terminal.reason, "detected");
            let ray = terminal.ray;
            let n = scene.spec.materials[1].index(ray.wavelength_um).unwrap();
            assert!((ray.optical_path_mm - (15.0 + 5.0 * n)).abs() < 1e-13);
            let alpha = 0.0001 + (ray.wavelength_um - 0.4) / 0.3 * 0.0002;
            let weight = (1.0 / 9300.0) * 0.995f64.powi(2) * (-alpha * 5.0).exp();
            assert!((ray.weight - weight).abs() < weight * 1e-12);
        }
        let serialized = serde_json::to_vec(&report).unwrap();
        if let Some(previous) = &previous {
            assert_eq!(&serialized, previous);
        }
        previous = Some(serialized);
    }
}

#[test]
fn indexed_search_keeps_numerical_failure_visibility_and_nearer_hits() {
    for nearer in [false, true] {
        let mut surfaces = vec![];
        if nearer {
            surfaces.push(json!({"name":"near", "pose":{"translation_mm":[-15,0,0],"rotation_deg":[0,90,0]},
                "shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":100,"half_height_mm":100},
                "interaction":{"type":"detector","pixels":[4,4]}}));
        }
        surfaces.push(json!({"name":"far", "shape":{"type":"even_asphere","radius_mm":25,"conic":-1,"coefficients":[1e8,-2e8,1e8]},
            "aperture":{"type":"circle","radius_mm":10},"interaction":{"type":"absorb"}}));
        let spec: Scene = serde_json::from_value(
            json!({"name":"numerical","reference_plane":"launch","mode":"non_sequential",
            "materials":[{"type":"constant","n":1}],"surfaces":surfaces}),
        )
        .unwrap();
        let plain = spec.clone().prepare().unwrap();
        let indexed = pad(&spec).prepare().unwrap();
        let ray = Ray {
            id: 0,
            position_mm: V3([-20.0, 1.0, 0.0]),
            direction: V3([1.0, 0.0, 0.0]),
            weight: 1.0,
            wavelength_um: 0.55,
            medium: 0,
            optical_path_mm: 0.0,
        };
        let options = TraceOptions {
            keep_terminals: true,
            path_launch_count: 1,
        };
        let expected = trace(&plain, &[ray], options);
        let mut actual = trace(&indexed, &[ray], options);
        actual.surfaces.truncate(plain.surfaces.len());
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(
            actual.budget.numerical_failure,
            if nearer { 0.0 } else { 1.0 }
        );
    }
}

#[test]
fn indexed_mesh_and_conical_sides_keep_edge_and_grazing_hits() {
    for shape in [
        json!({"type":"conical_side","radius_start_mm":1.0,"radius_end_mm":0.7,"length_mm":4.0}),
        json!({"type":"triangle_mesh","vertices":[[-1,-1,0],[1,-1,0],[-1,1,0],[-1,-1,2]],
            "triangles":[[0,2,1],[0,1,3],[0,3,2],[1,2,3]],"normals":[[0,0,-1],[0,0,-1],[0,0,-1],[0,0,1]]}),
        json!({"type":"flat_even_asphere","coefficients":[0.02,-0.001]}),
    ] {
        for rotation in [[0.0, 0.0, 0.0], [17.0, -23.0, 8.0], [73.0, -8.0, 26.0]] {
            let placement = Frame::new(&Pose {
                translation_mm: V3([10.0, -2.0, 3.0]),
                rotation_deg: V3(rotation),
            });
            let mut spec: Scene = serde_json::from_value(json!({"name":"edge regression","reference_plane":"launch","mode":"non_sequential",
                "materials":[{"type":"constant","n":1}],"surfaces":[
                    {"name":"test", "shape":shape,"aperture":{"type":"circle","radius_mm":2},"interaction":{"type":"absorb"}},
                    {"name":"backstop","pose":{"translation_mm":[0,0,10]},"shape":{"type":"plane"},
                        "aperture":{"type":"circle","radius_mm":100},"interaction":{"type":"detector","pixels":[8,8]}}
                ]})).unwrap();
            for surface in &mut spec.surfaces {
                surface.pose = placement.transform_pose(&surface.pose);
            }
            let plain = spec.clone().prepare().unwrap();
            let indexed = pad(&spec).prepare().unwrap();
            let mut rays = Vec::new();
            for i in 0..2000 {
                let local_d = if i % 2 == 0 {
                    V3([0.0, 0.0, 1.0])
                } else {
                    V3([1.0, 0.0, 1e-7]).unit()
                };
                let target = V3([
                    (i % 51) as f64 * 0.08 - 2.0,
                    (i % 47) as f64 * 0.08 - 2.0,
                    0.0,
                ]);
                rays.push(Ray {
                    id: i,
                    position_mm: placement.world_point(target - local_d * 5.0),
                    direction: placement.world_vector(local_d),
                    weight: 1.0 / 2000.0,
                    wavelength_um: 0.55,
                    medium: 0,
                    optical_path_mm: 0.0,
                });
            }
            let options = TraceOptions {
                keep_terminals: true,
                path_launch_count: 32,
            };
            let expected = trace(&plain, &rays, options);
            let mut actual = trace(&indexed, &rays, options);
            actual.surfaces.truncate(plain.surfaces.len());
            assert_eq!(
                serde_json::to_vec(&actual).unwrap(),
                serde_json::to_vec(&expected).unwrap()
            );
        }
    }
}
