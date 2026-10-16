use raytrace_rs::{
    math::{Frame, Pose},
    model::Diode,
    zemax,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "source_diode_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn native(astigmatism: f64) -> String {
    let mut text = "MODE NSC\nUNIT MM\nPWAV 1\nWAVM 1 .55 1\nSURF 1\nTYPE NONSEQCO\nNSOH NSC_SDIO 0 0\nNSOP 3 4 5 10 20 30\n".to_owned();
    for (id, value) in [
        (2, 8192.),
        (3, 1.),
        (6, astigmatism),
        (7, 10.),
        (8, 2.),
        (9, 12.),
        (10, 0.5),
        (11, 1.),
        (12, 1.),
        (15, 1.),
        (16, 1.),
        (17, 2.),
        (18, 1.),
        (19, 1.),
        (20, 0.5),
    ] {
        text.push_str(&format!("NSOD {id} {value}\n"));
    }
    text.push_str("NSOH NSC_DETC 0 0\nNSOP 0 0 100 0 0 0 ABSORB\nNSOD 1 1000\nNSOD 2 1000\nNSOD 3 16\nNSOD 4 16\n");
    text
}
fn mass(limit: f64, width: f64, factor: f64) -> f64 {
    // Independent Simpson integration of the documented intensity law.
    let n = 4096;
    let h = limit / n as f64;
    let density = |x: f64| (-2. * (x / width).powf(2. * factor)).exp();
    h / 3.
        * ((density(0.) + density(limit))
            + (1..n)
                .map(|i| if i % 2 == 0 { 2. } else { 4. } * density(h * i as f64))
                .sum::<f64>())
}
#[test]
fn native_supergaussian_spatial_and_angular_masses_match_documented_law() {
    let dir = folder();
    let path = dir.join("owned.zmx");
    fs::write(&path, native(0.)).unwrap();
    let p = zemax::read_zmx(&path, &[]).unwrap();
    assert!(!p.issues.iter().any(|i| i.blocking), "{:?}", p.issues);
    let source = &p.scene.sources[0];
    let d = source.diode.as_ref().unwrap();
    let frame = Frame::new(&Pose {
        translation_mm: source.origin_mm,
        rotation_deg: d.rotation_deg,
    });
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    for axis in 0..2 {
        let mut spatial = 0;
        let mut angular = 0;
        for r in &rays {
            let pos = frame.local_point(r.position_mm);
            let dir = frame.local_vector(r.direction);
            let angle = (dir.0[axis] / dir.0[2]).atan().to_degrees();
            assert!(pos.0[axis].abs() <= 1. + 1e-12 && angle.abs() < 90.);
            spatial += usize::from(pos.0[axis].abs() <= 0.5);
            angular += usize::from(angle.abs() <= d.divergence_deg[axis] / 2.);
        }
        let spatial_expected = mass(0.5, 1., d.spatial_supergaussian[axis])
            / mass(1., 1., d.spatial_supergaussian[axis]);
        let angular_expected = mass(
            d.divergence_deg[axis] / 2.,
            d.divergence_deg[axis],
            d.angular_supergaussian[axis],
        ) / mass(90., d.divergence_deg[axis], d.angular_supergaussian[axis]);
        assert!((spatial as f64 / rays.len() as f64 - spatial_expected).abs() < 3. / 8192.);
        assert!((angular as f64 / rays.len() as f64 - angular_expected).abs() < 3. / 8192.);
    }
    assert!((rays.iter().map(|r| r.weight).sum::<f64>() - 1.).abs() < 1e-12);
}
#[test]
fn astigmatic_source_has_two_waist_planes_and_ignores_spatial_parameters() {
    let dir = folder();
    let path = dir.join("astigmatic.zmx");
    fs::write(&path, native(2.)).unwrap();
    let p = zemax::read_zmx(&path, &[]).unwrap();
    let source = &p.scene.sources[0];
    let frame = Frame::new(&Pose {
        translation_mm: source.origin_mm,
        rotation_deg: source.diode.as_ref().unwrap().rotation_deg,
    });
    let rays = p
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap();
    for r in &rays {
        let pos = frame.local_point(r.position_mm);
        let d = frame.local_vector(r.direction);
        assert!(pos.0[1].abs() < 1e-12 && pos.0[2].abs() < 1e-12);
        assert!((pos.0[0] - 2. * d.0[0] / d.0[2]).abs() < 1e-11);
    }
    let mut changed = p.scene.clone();
    let d = changed.sources[0].diode.as_mut().unwrap();
    d.half_width_mm = [-3., -4.];
    d.gaussian_width_mm = [-5., -6.];
    d.spatial_supergaussian = [0., -1.];
    let again = changed.prepare().unwrap().generate_rays().unwrap();
    assert_eq!(
        serde_json::to_value(&rays).unwrap(),
        serde_json::to_value(again).unwrap()
    );
}
#[test]
fn native_distribution_edits_roundtrip_and_old_json_gets_gaussian_defaults() {
    let dir = folder();
    let path = dir.join("edit.zmx");
    fs::write(&path, native(0.)).unwrap();
    let mut p = zemax::read_zmx(&path, &[]).unwrap();
    let d = p.scene.sources[0].diode.as_mut().unwrap();
    d.astigmatism_mm = 1.25;
    d.angular_supergaussian = [3., 4.];
    d.spatial_supergaussian = [5., 6.];
    let out = dir.join("roundtrip.zmx");
    zemax::export(&p, &out).unwrap();
    let q = zemax::read_zmx(&out, &[]).unwrap();
    let mut round = q.scene.sources[0].diode.clone().unwrap();
    let wanted = p.scene.sources[0].diode.as_ref().unwrap();
    assert!((round.rotation_deg - wanted.rotation_deg).norm() < 1e-12);
    round.rotation_deg = wanted.rotation_deg;
    assert_eq!(
        serde_json::to_value(&p.scene.sources[0].diode).unwrap(),
        serde_json::to_value(&round).unwrap()
    );
    let mut legacy = serde_json::to_value(&p.scene.sources[0].diode).unwrap();
    for key in [
        "astigmatism_mm",
        "angular_supergaussian",
        "spatial_supergaussian",
    ] {
        legacy.as_object_mut().unwrap().remove(key);
    }
    let d: Diode = serde_json::from_value(legacy).unwrap();
    assert_eq!(d.astigmatism_mm, 0.);
    assert_eq!(d.angular_supergaussian, [1.; 2]);
    assert_eq!(d.spatial_supergaussian, [1.; 2]);
}
#[test]
fn invalid_new_source_parameters_are_reported_at_import() {
    let dir = folder();
    for (i, (from, to)) in [
        ("NSOD 8 2", "NSOD 8 0.001"),
        ("NSOD 10 0.5", "NSOD 10 51"),
        ("NSOD 6 0", "NSOD 6 -1"),
        ("NSOD 17 2", "NSOD 17 0"),
    ]
    .iter()
    .enumerate()
    {
        let path = dir.join(format!("invalid{i}.zmx"));
        fs::write(&path, native(0.).replace(from, to)).unwrap();
        let p = zemax::read_zmx(&path, &[]).unwrap();
        assert!(p.issues.iter().any(|i| i.blocking && i.code == "source"));
        assert!(p.trace_scene().is_err());
    }
}

#[test]
fn new_distribution_records_reject_legacy_sampling_but_normal_gaussians_replay() {
    let dir = folder();
    let path = dir.join("owned.zmx");
    fs::write(&path, native(0.)).unwrap();
    let mut p = zemax::read_zmx(&path, &[]).unwrap();
    p.scene.sources[0].count = 32;
    let mut record =
        raytrace_rs::record::RunContext::capture(&p, &p.scene, None, 1, Default::default());
    assert!(matches!(
        record.input,
        raytrace_rs::record::LaunchInput::Generated {
            sampling_revision: 3,
            ..
        }
    ));
    record.input = raytrace_rs::record::LaunchInput::Generated {
        sampling_revision: 2,
        count: 32,
    };
    assert!(record.rays().is_err());
    let d = record.scene.sources[0].diode.as_mut().unwrap();
    d.angular_supergaussian = [1.; 2];
    d.spatial_supergaussian = [1.; 2];
    assert!(record.rays().is_ok());
}
