use raytrace_rs::{
    assembly::{Assembly, Component, ComponentKind},
    geometry::{dielectric, intersect_ordered},
    illumination::{pupil_geometry, RayAiming},
    math::{Frame, Pose, V3},
    model::{Interaction, PreparedScene, Ray},
    project::Project,
    trace::{trace, TraceOptions},
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
            "pupil_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn native(mode: usize, finite: bool) -> String {
    format!("MODE SEQ\nUNIT MM\nGCAT own\nENPD 6\nRAIM {mode} 0 0\nFTYP {} 0 2 2\nXFLN 0 3\nYFLN 0 0\nFWGN 1 2\nWAVM 1 .45 1\nWAVM 2 .65 1\nPWAV 1\nSURF 0\nTYPE STANDARD\nDISZ {}\nSURF 1\nTYPE STANDARD\nCURV .05\nDIAM 5.5\nGLAS OWN 0 0\nDISZ 2\nSURF 2\nTYPE STANDARD\nCURV -.05\nDIAM 5.5\nDISZ 8\nSURF 3\nTYPE STANDARD\nSTOP\nDIAM 6\nDISZ 10\nSURF 4\nTYPE STANDARD\nDIAM 20\nDISZ 0\n",if finite {1}else{0},if finite {"50"}else{"INFINITY"})
}
fn load(mode: usize, finite: bool, dispersive: bool) -> (PathBuf, Project) {
    let dir = folder();
    let path = dir.join("owned.zmx");
    fs::write(
        dir.join("own.AGF"),
        if dispersive {
            "NM OWN 2 0 1.46 50 0 0 0\nCD .8 .005 .3 .01 .1 10\nLD .4 .8\n"
        } else {
            "NM OWN 5 0 1.5 50 0 0 0\nCD 1.5 0 0\nLD .4 .8\n"
        },
    )
    .unwrap();
    fs::write(&path, native(mode, finite)).unwrap();
    let mut p = zemax::read_zmx(&path, &[]).unwrap();
    assert!(!p.issues.iter().any(|i| i.blocking), "{:?}", p.issues);
    for s in &mut p.scene.sources {
        s.count = 128;
    }
    (dir, p)
}
fn stop_hit(scene: &PreparedScene, ray: &Ray) -> V3 {
    let mut r = *ray;
    for (i, s) in scene.surfaces.iter().enumerate() {
        let h = intersect_ordered(s, r.position_mm, r.direction).unwrap();
        if i == 2 {
            return h.local_point;
        }
        let Interaction::Dielectric {
            minus_medium,
            plus_medium,
            ..
        } = s.spec.interaction
        else {
            panic!("refracting prefix expected")
        };
        assert_eq!(r.medium, minus_medium);
        r.direction = dielectric(
            r.direction,
            h.normal,
            scene.spec.materials[minus_medium]
                .index(r.wavelength_um)
                .unwrap(),
            scene.spec.materials[plus_medium]
                .index(r.wavelength_um)
                .unwrap(),
        )
        .transmitted
        .unwrap();
        r.position_mm = h.point;
        r.medium = plus_medium;
    }
    panic!("stop expected")
}
#[test]
fn entrance_pupil_matches_independent_thick_lens_matrix_and_updates_with_stop() {
    let (_, mut p) = load(0, false, false);
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let g = pupil_geometry(&prepared, 0).unwrap().unwrap();
    // R1=20, R2=-20, n=1.5, t=2, stop 8 behind rear: A=43/75, B=136/15.
    assert!((g.entrance_pose.translation_mm.0[2] - 680. / 43.).abs() < 1e-11);
    assert!((g.paraxial_stop_radius_mm - 3. * 43. / 75.).abs() < 1e-12);
    assert_eq!(g.aiming, RayAiming::Off);
    p.scene.surfaces[2].pose.translation_mm.0[2] += 2.;
    let moved = pupil_geometry(&p.trace_scene().unwrap().prepare().unwrap(), 0)
        .unwrap()
        .unwrap();
    assert!((moved.entrance_pose.translation_mm.0[2] - 440. / 19.).abs() < 1e-10);
}
#[test]
fn paraxial_and_real_ray_aiming_fill_stop_at_each_field_and_wavelength() {
    for finite in [false, true] {
        for mode in [1, 2] {
            let (_, p) = load(mode, finite, true);
            let prepared = p.trace_scene().unwrap().prepare().unwrap();
            let mut off = p.scene.clone();
            for s in &mut off.sources {
                s.pupil.as_mut().unwrap().optical.as_mut().unwrap().aiming = RayAiming::Off;
            }
            let off = off.prepare().unwrap();
            let seeds = off.generate_rays().unwrap();
            let aimed = prepared
                .generate_rays()
                .unwrap_or_else(|e| panic!("finite={finite}, mode={mode}: {e}"));
            let mut max_off_error: f64 = 0.;
            for (i, (reference, ray)) in seeds.iter().zip(&aimed).enumerate() {
                let g = pupil_geometry(&prepared, i / 128).unwrap().unwrap();
                let t = (g.entrance_pose.translation_mm.0[2] - reference.position_mm.0[2])
                    / reference.direction.0[2];
                let ep = reference.position_mm + reference.direction * t;
                let target = [
                    ep.0[0] / g.entrance_radius_mm * g.target_stop_radius_mm,
                    ep.0[1] / g.entrance_radius_mm * g.target_stop_radius_mm,
                ];
                let hit = stop_hit(&prepared, ray);
                assert!(
                    (hit.0[0] - target[0]).hypot(hit.0[1] - target[1]) < 3e-8,
                    "finite={finite}, mode={mode}, ray={i}"
                );
                let h = stop_hit(&off, reference);
                max_off_error = max_off_error.max((h.0[0] - target[0]).hypot(h.0[1] - target[1]));
            }
            assert!(max_off_error > 1e-3);
            assert!(
                aimed.iter().any(|r| r.wavelength_um == 0.45)
                    && aimed.iter().any(|r| r.wavelength_um == 0.65)
            );
            let report = trace(&prepared, &aimed, TraceOptions::default());
            assert_eq!(report.budget.medium_mismatch, 0.);
            assert_eq!(report.budget.numerical_failure, 0.);
        }
    }
}
#[test]
fn real_aiming_stop_radius_comes_from_the_primary_marginal_ray() {
    let (_, p) = load(2, false, false);
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let g = pupil_geometry(&prepared, 0).unwrap().unwrap();
    let r = Ray {
        id: 0,
        position_mm: V3([3., 0., -1.]),
        direction: V3([0., 0., 1.]),
        wavelength_um: 0.45,
        weight: 1.,
        medium: 0,
        optical_path_mm: 0.,
    };
    let h = stop_hit(&prepared, &r);
    assert!((g.target_stop_radius_mm - h.0[0].hypot(h.0[1])).abs() < 1e-11);
    assert!((g.target_stop_radius_mm - g.paraxial_stop_radius_mm).abs() > 1e-3);
}
#[test]
fn vignetting_factors_shift_compress_then_rotate_without_hiding_aperture_losses() {
    let (dir, p) = load(1, false, false);
    let path = dir.join("vignette.zmx");
    fs::write(
        &path,
        native(1, false).replace(
            "RAIM 1",
            "VDXN .2 -.1\nVDYN .1 .2\nVCXN .2 .3\nVCYN .4 .1\nVANN 30 -20\nRAIM 1",
        ),
    )
    .unwrap();
    let mut q = zemax::read_zmx(&path, &[]).unwrap();
    for s in &mut q.scene.sources {
        s.count = 128;
    }
    let base = p.scene.clone().prepare().unwrap();
    let before = base.generate_rays().unwrap();
    let transformed = q.scene.clone().prepare().unwrap();
    let after = transformed.generate_rays().unwrap();
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        let pupil = q.scene.sources[i / 128].pupil.as_ref().unwrap();
        let v = &pupil.vignetting;
        let g = pupil_geometry(&base, i / 128).unwrap().unwrap();
        let aa = stop_hit(&base, a);
        let bb = stop_hit(&transformed, b);
        let x = v.decenter[0] + aa.0[0] / g.target_stop_radius_mm * (1. - v.compression[0]);
        let y = v.decenter[1] + aa.0[1] / g.target_stop_radius_mm * (1. - v.compression[1]);
        let (sn, cs) = v.tangential_angle_deg.to_radians().sin_cos();
        assert!((bb.0[0] / g.target_stop_radius_mm - (x * cs - y * sn)).abs() < 1e-8);
        assert!((bb.0[1] / g.target_stop_radius_mm - (x * sn + y * cs)).abs() < 1e-8);
    }
    // A narrow physical front aperture remains a loss in the actual trace.
    q.scene.surfaces[0].aperture = raytrace_rs::model::Aperture::Circle { radius_mm: 0.5 };
    let clipped = q.scene.prepare().unwrap();
    let rays = clipped.generate_rays().unwrap();
    let report = trace(&clipped, &rays, TraceOptions::default());
    assert!(report.budget.geometric_miss > 0.5);
    assert!((report.budget.accounted() - report.budget.launched).abs() < 1e-12);
}
#[test]
fn finite_angle_fields_follow_the_computed_pupil_after_layout_changes() {
    let (dir, _) = load(1, true, false);
    let path = dir.join("angle.zmx");
    fs::write(&path, native(1, true).replace("FTYP 1", "FTYP 0")).unwrap();
    let mut p = zemax::read_zmx(&path, &[]).unwrap();
    assert!(!p.issues.iter().any(|i| i.blocking), "{:?}", p.issues);
    for s in &mut p.scene.sources {
        s.count = 128;
    }
    p.scene.surfaces[2].pose.translation_mm.0[2] += 1.;
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let g = pupil_geometry(&prepared, 1).unwrap().unwrap();
    let x = -3_f64.to_radians().tan() * (g.entrance_pose.translation_mm.0[2] + 50.);
    assert!((g.object_origin_mm.0[0] - x).abs() < 1e-12);
    let before = prepared.generate_rays().unwrap();
    assert!(before
        .iter()
        .skip(128)
        .all(|r| (r.position_mm.0[0] - x).abs() < 1e-12));
    let out = dir.join("edited_angle.zmx");
    zemax::export(&p, &out).unwrap();
    let mut q = zemax::read_zmx(&out, &[]).unwrap();
    for s in &mut q.scene.sources {
        s.count = 128;
    }
    let after = q
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap();
    for (a, b) in before.iter().zip(after) {
        assert!((a.position_mm - b.position_mm).norm() < 1e-11);
        assert!((a.direction - b.direction).norm() < 1e-11);
    }
}

#[test]
fn saved_conditions_capture_pupil_geometry_and_enforce_sampling_revision() {
    let (_, p) = load(1, false, true);
    let scene = p.trace_scene().unwrap();
    let record =
        raytrace_rs::record::RunContext::capture(&p, &scene, None, 2, TraceOptions::default());
    assert_eq!(record.pupils.len(), 2);
    assert!(record
        .pupils
        .iter()
        .all(|p| p.geometry.is_some() && p.error.is_none()));
    assert_eq!(
        record.pupils[0]
            .geometry
            .as_ref()
            .unwrap()
            .primary_wavelength_um,
        0.45
    );
    let expected = scene.clone().prepare().unwrap().generate_rays().unwrap();
    assert_eq!(
        serde_json::to_value(record.rays().unwrap()).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let explicit = raytrace_rs::record::RunContext::capture(
        &p,
        &scene,
        Some(&expected),
        2,
        TraceOptions::default(),
    );
    assert!(explicit.pupils.is_empty());
    let mut old = record.clone();
    old.input = raytrace_rs::record::LaunchInput::Generated {
        sampling_revision: 2,
        count: 256,
    };
    assert!(old.rays().is_err());
    for source in &mut old.scene.sources {
        source.pupil.as_mut().unwrap().optical = None;
    }
    assert!(old.rays().is_ok());
    let mut mono = p;
    mono.monochromatic_override_um = Some(0.65);
    let prepared = mono.trace_scene().unwrap().prepare().unwrap();
    assert_eq!(
        pupil_geometry(&prepared, 0)
            .unwrap()
            .unwrap()
            .primary_wavelength_um,
        0.65
    );
}

#[test]
fn failed_aiming_reports_the_source_ray_and_wavelength_instead_of_substitution() {
    let (dir, _) = load(1, false, true);
    fs::write(
        dir.join("own.AGF"),
        "NM OWN 2 0 1.6 50 0 0 0\nCD 1 .01 .5 .02 .1 10\nLD .4 .8\n",
    )
    .unwrap();
    let mut p = zemax::read_zmx(&dir.join("owned.zmx"), &[]).unwrap();
    for s in &mut p.scene.sources {
        s.count = 128;
    }
    let error = p
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap_err();
    assert!(
        error.contains("光源 2")
            && error.contains("光線")
            && error.contains("0.45 um")
            && error.contains("絞り上の誤差"),
        "{error}"
    );
}

#[test]
fn aspheric_prefix_aims_at_the_same_stop_targets() {
    let (_, mut p) = load(1, false, false);
    p.scene.surfaces[0].shape = raytrace_rs::model::Shape::EvenAsphere {
        radius_mm: 20.,
        conic: -1.,
        coefficients: vec![1e-5],
    };
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let mut off = p.scene.clone();
    for s in &mut off.sources {
        s.pupil.as_mut().unwrap().optical.as_mut().unwrap().aiming = RayAiming::Off;
    }
    let seeds = off.prepare().unwrap().generate_rays().unwrap();
    for (i, (a, b)) in seeds.iter().zip(&rays).enumerate() {
        let g = pupil_geometry(&prepared, i / 128).unwrap().unwrap();
        let ep = a.position_mm
            + a.direction
                * ((g.entrance_pose.translation_mm.0[2] - a.position_mm.0[2]) / a.direction.0[2]);
        let h = stop_hit(&prepared, b);
        assert!(
            (h.0[0] - ep.0[0] / 3. * g.target_stop_radius_mm)
                .hypot(h.0[1] - ep.0[1] / 3. * g.target_stop_radius_mm)
                < 3e-8
        );
    }
}

#[test]
fn optical_pupil_survives_composition_native_roundtrip_and_source_file_removal() {
    let (dir, mut p) = load(1, true, false);
    p.scene.surfaces[3].pose.translation_mm.0[2] += 3.;
    let output = dir.join("roundtrip.zmx");
    zemax::export(&p, &output).unwrap();
    let mut q = zemax::read_zmx(&output, &[]).unwrap();
    for s in &mut q.scene.sources {
        s.count = 128;
    }
    let original = p
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap();
    let round = q
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&original).unwrap(),
        serde_json::to_value(&round).unwrap()
    );
    let saved = dir.join("saved.json");
    p.save(&saved).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&saved).unwrap()).unwrap()["format"],
        "raytrace-project-v3"
    );
    fs::remove_file(dir.join("owned.zmx")).unwrap();
    fs::remove_file(dir.join("own.AGF")).unwrap();
    let reopened = Project::open(&saved, &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&original).unwrap(),
        serde_json::to_value(
            reopened
                .trace_scene()
                .unwrap()
                .prepare()
                .unwrap()
                .generate_rays()
                .unwrap()
        )
        .unwrap()
    );
    let mut component = Component::imported(reopened, ComponentKind::System).unwrap();
    component.pose = Pose {
        translation_mm: V3([3., 4., 5.]),
        rotation_deg: V3([12., 18., 25.]),
    };
    let frame = Frame::new(&component.pose);
    let anchor = component.anchor_mm;
    let mut assembly = Assembly {
        components: vec![Component::detector(), component],
    };
    let (flat, _, _) = assembly.compose(&p.scene).unwrap();
    assert_eq!(
        flat.sources[0]
            .pupil
            .as_ref()
            .unwrap()
            .optical
            .as_ref()
            .unwrap()
            .first_surface,
        1
    );
    let rays = flat.clone().prepare().unwrap().generate_rays().unwrap();
    for (a, b) in original.iter().zip(rays) {
        assert!((frame.world_point(a.position_mm - anchor) - b.position_mm).norm() < 1e-8);
        assert!((frame.world_vector(a.direction) - b.direction).norm() < 1e-10);
    }
    assembly.sync_sources(&flat);
    let (synced, _, _) = assembly.compose(&p.scene).unwrap();
    assert_eq!(
        synced.sources[0]
            .pupil
            .as_ref()
            .unwrap()
            .optical
            .as_ref()
            .unwrap()
            .first_surface,
        1
    );
}
