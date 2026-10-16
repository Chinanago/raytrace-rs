use raytrace_rs::{math::V3, model::*};
use serde_json::json;
use std::f64::consts::TAU;

fn specification(count: usize) -> Scene {
    serde_json::from_value(json!({
        "name":"Sampling check", "reference_plane":"Launch", "mode":"non_sequential",
        "materials":[{"type":"constant","n":1}],
        "surfaces":[{"name":"screen", "pose":{"translation_mm":[0,0,100]},
            "shape":{"type":"plane"}, "aperture":{"type":"circle","radius_mm":100},
            "interaction":{"type":"detector","pixels":[8,8]}}],
        "sources":[{"count":count,"origin_mm":[0,0,0],"direction":[0,0,1],
            "disk_radius_mm":1,"cone_half_angle_deg":20,"wavelength_um":0.55,
            "medium":0,"total_weight":1,"seed":42}]
    }))
    .unwrap()
}

fn coordinates(ray: &Ray) -> [f64; 4] {
    // The +Z source frame has local u=+Y, v=-X. Radius^2 and cos(theta) are uniform.
    [
        ray.position_mm.0[0].powi(2) + ray.position_mm.0[1].powi(2),
        (-ray.position_mm.0[0])
            .atan2(ray.position_mm.0[1])
            .rem_euclid(TAU)
            / TAU,
        (1.0 - ray.direction.0[2]) / (1.0 - 20f64.to_radians().cos()),
        (-ray.direction.0[0])
            .atan2(ray.direction.0[1])
            .rem_euclid(TAU)
            / TAU,
    ]
}

#[test]
fn uniform_disk_cone_and_joint_phase_space_have_balanced_equal_measure_bins() {
    for seed in [0, 42, 123456] {
        let mut spec = specification(4096);
        spec.sources[0].seed = seed;
        assert_eq!(spec.sources[0].sampling, Sampling::Sobol);
        let rays = spec.prepare().unwrap().generate_rays().unwrap();
        let mut area = [0; 64];
        let mut solid_angle = [0; 64];
        let mut phase_space = [0; 256];
        for ray in &rays {
            let u = coordinates(ray);
            assert!(u.iter().all(|x| (0.0..1.0).contains(x)));
            area[(u[0] * 8.0) as usize * 8 + (u[1] * 8.0) as usize] += 1;
            solid_angle[(u[2] * 8.0) as usize * 8 + (u[3] * 8.0) as usize] += 1;
            let cell = u
                .into_iter()
                .fold(0, |cell, x| cell * 4 + (x * 4.0) as usize);
            phase_space[cell] += 1;
        }
        assert!(area.iter().all(|&n| n == 64), "seed={seed}: {area:?}");
        assert!(solid_angle.iter().all(|&n| n == 64));
        assert!(phase_space.iter().all(|&n| n == 16), "{phase_space:?}");
        assert!((rays.iter().map(|r| r.weight).sum::<f64>() - 1.0).abs() < 1e-12);
    }
}

#[test]
fn source_seed_prefix_and_serialization_are_reproducible_for_both_methods() {
    for method in [Sampling::Sobol, Sampling::Pseudorandom] {
        let mut spec = specification(257);
        spec.sources[0].sampling = method;
        let encoded = serde_json::to_string(&spec).unwrap();
        let restored: Scene = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.sources[0].sampling, method);
        let full = restored.prepare().unwrap().generate_rays().unwrap();
        assert_eq!(full.len(), 257);
        let mut shorter = spec.clone();
        shorter.sources[0].count = 64;
        let prefix = shorter.prepare().unwrap().generate_rays().unwrap();
        for (a, b) in full.iter().zip(&prefix) {
            assert_eq!(a.position_mm, b.position_mm);
            assert_eq!(a.direction, b.direction);
        }
        spec.sources[0].seed += 1;
        let changed = spec.prepare().unwrap().generate_rays().unwrap();
        assert_ne!(full[0].position_mm, changed[0].position_mm);
    }
}

#[test]
fn diode_keeps_gaussian_mass_and_balances_array_elements() {
    let mut spec = specification(4096);
    spec.sources[0].diode = Some(Diode {
        astigmatism_mm: 0.,
        angular_supergaussian: [1.; 2],
        spatial_supergaussian: [1.; 2],
        rotation_deg: V3([0.0, 0.0, 0.0]),
        divergence_deg: [10.0, 20.0],
        array: [4, 2],
        spacing_mm: [10.0, 10.0],
        half_width_mm: [0.3, 0.3],
        gaussian_width_mm: [0.2, 0.2],
    });
    let rays = spec.prepare().unwrap().generate_rays().unwrap();
    let mut counts = [0; 8];
    let mut central = 0;
    let mut variance = [0.0; 2];
    for ray in &rays {
        let [x, y, _] = ray.position_mm.0;
        let ix = ((x + 15.0) / 10.0).round() as usize;
        let iy = ((y + 5.0) / 10.0).round() as usize;
        counts[ix * 2 + iy] += 1;
        let local_x = x - (ix as f64 - 1.5) * 10.0;
        assert!(local_x.abs() <= 0.3);
        if local_x.abs() <= 0.1 {
            central += 1;
        }
        for (axis, v) in variance.iter_mut().enumerate() {
            let a = (ray.direction.0[axis] / ray.direction.0[2])
                .atan()
                .to_degrees();
            *v += a * a / rays.len() as f64;
        }
    }
    assert!(counts.iter().all(|&n| n == 512), "{counts:?}");
    let expected =
        libm::erf(1.0 / std::f64::consts::SQRT_2) / libm::erf(3.0 / std::f64::consts::SQRT_2);
    assert!((central as f64 / rays.len() as f64 - expected).abs() < 2.0 / 4096.0);
    assert!((variance[0] - 25.0).abs() < 0.12);
    assert!((variance[1] - 100.0).abs() < 0.5);
}
