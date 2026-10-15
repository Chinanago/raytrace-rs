use raytrace_rs::{
    assembly::{self, Component, ComponentKind},
    geometry::intersect,
    math::{Frame, Pose, V3},
    model::*,
    project::{Issue, Project},
    trace::{trace, TraceOptions},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/assembly_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn files() -> (PathBuf, PathBuf, PathBuf) {
    let dir = folder();
    let source = dir.join("source.zmx");
    let lens = dir.join("lens.zmx");
    fs::write(&source, "MODE NSC\nUNIT MM\nWAVM 1 0.55 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_SDIO 0 0 SyntheticSource\n NSOP 5 7 2 0 0 0\n NSOD 2 128\n NSOD 3 1\n NSOD 7 2\n NSOD 9 2\n NSOD 15 0.02\n NSOD 16 0.01\n NSOD 18 0.02\n NSOD 19 0.01\n NSOH NSC_DETC 0 0 OldDetector\n NSOP 0 0 50 0 0 0 ABSORB\n NSOD 1 10\n NSOD 2 10\n NSOD 3 8\n NSOD 4 8\n").unwrap();
    fs::write(&lens, "MODE SEQ\nUNIT MM\nGCAT TEST\nWAVM 1 0.55 1\nSURF 0\n TYPE STANDARD\n DISZ INFINITY\nSURF 1\n TYPE STANDARD\n DISZ 5\n DIAM 2\nSURF 2\n TYPE STANDARD\n CURV 0.02\n GLAS TESTGLASS 0 0\n DISZ 3\n DIAM 2\nSURF 3\n TYPE STANDARD\n CURV -0.02\n DISZ 30\n DIAM 2\nSURF 4\n TYPE STANDARD\n DISZ 0\n DIAM 10\n").unwrap();
    fs::write(
        dir.join("TEST.AGF"),
        "NM TESTGLASS 1 0 1.5 50 0 0\nCD 2.25 0 0 0 0 0\nLD 0.4 0.7\n",
    )
    .unwrap();
    (dir, source, lens)
}
fn assembled() -> (Project, PathBuf, PathBuf, PathBuf) {
    let (dir, source, lens) = files();
    let mut p = Project::new_assembly();
    let mut s =
        Component::imported(Project::open(&source, &[]).unwrap(), ComponentKind::Source).unwrap();
    s.pose.translation_mm.0[2] = -5.0;
    p.add_component(s).unwrap();
    let l = Component::imported(Project::open(&lens, &[]).unwrap(), ComponentKind::Optic).unwrap();
    p.add_component(l).unwrap();
    let mut rod = Component::rod(
        Material::Constant {
            n: 1.7,
            absorption_per_mm: 0.0,
        },
        "TESTGLASS".into(),
    );
    rod.pose.translation_mm.0[2] = 8.0;
    let r = rod.rod.as_mut().unwrap();
    r.width_mm = 3.0;
    r.height_mm = 3.0;
    r.length_mm = 5.0;
    p.add_component(rod).unwrap();
    let mut d = Component::detector();
    d.pose.translation_mm.0[2] = 20.0;
    p.add_component(d).unwrap();
    (p, dir, source, lens)
}
fn run(p: &Project) -> raytrace_rs::trace::Report {
    let s = p.trace_scene().unwrap().prepare().unwrap();
    let rays = s.generate_rays().unwrap();
    trace(
        &s,
        &rays,
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 2,
        },
    )
}

#[test]
fn separate_zmx_parts_drop_test_sources_detectors_and_remap_materials() {
    let (p, _, _, _) = assembled();
    let scene = p.trace_scene().unwrap();
    assert_eq!(scene.mode, Mode::NonSequential);
    assert_eq!(scene.sources.len(), 1);
    assert_eq!(scene.materials.len(), 3);
    assert_eq!(
        scene
            .surfaces
            .iter()
            .filter(|s| matches!(s.interaction, Interaction::Detector { .. }))
            .count(),
        1
    );
    let a = p.assembly.as_ref().unwrap();
    let lens = &a.components[1];
    assert_eq!(lens.project.scene.sources.len(), 0);
    assert_eq!(lens.project.scene.surfaces.len(), 3);
    assert_eq!(lens.anchor_mm, V3([0.0, 0.0, 5.0]));
    assert!(lens.project.zemax.as_ref().unwrap().text.contains("SURF 4"));
    let rod_entry = scene
        .surfaces
        .iter()
        .find(|s| s.name.contains("ロッド / 入口"))
        .unwrap();
    let Interaction::Dielectric { plus_medium, .. } = rod_entry.interaction else {
        panic!()
    };
    assert_eq!(scene.materials[plus_medium].index(0.55), Some(1.7));
    let report = run(&p);
    assert!(report.budget.detected > 0.7, "{:?}", report.budget);
    assert_eq!(
        report.budget.medium_mismatch + report.budget.numerical_failure,
        0.0
    );
    assert!((report.budget.accounted() - report.budget.launched).abs() < 1e-11);
    assert!(report
        .surfaces
        .iter()
        .filter(|s| s.name.contains(" / S")
            || s.name.contains(" / 入口")
            || s.name.contains(" / 出口"))
        .all(|s| s.encounters > 0));
}

#[test]
fn composition_survives_source_file_removal_and_saves_latest_poses() {
    let (mut p, dir, source, lens) = assembled();
    p.assembly.as_mut().unwrap().components[3]
        .pose
        .translation_mm
        .0[2] = 23.0;
    let output = dir.join("system.json");
    p.save(&output).unwrap();
    fs::remove_file(source).unwrap();
    fs::remove_file(lens).unwrap();
    fs::remove_file(dir.join("TEST.AGF")).unwrap();
    let loaded = Project::open(&output, &[]).unwrap();
    assert_eq!(loaded.format, "raytrace-project-v2");
    assert_eq!(loaded.assembly.as_ref().unwrap().components.len(), 4);
    assert_eq!(
        loaded.scene.surfaces.last().unwrap().pose.translation_mm.0[2],
        23.0
    );
    assert!((run(&p).budget.detected - run(&loaded).budget.detected).abs() < 1e-14);
    assert!(raytrace_rs::zemax::export(&loaded, &dir.join("unsupported.zmx")).is_err());
}

#[test]
fn rigid_component_rotations_preserve_ray_positions_directions_power_and_opl() {
    let (mut p, _, _, _) = assembled();
    let before = run(&p);
    let f = Frame::new(&Pose {
        translation_mm: V3([12.0, -7.0, 9.0]),
        rotation_deg: V3([23.0, 31.0, -17.0]),
    });
    for c in &mut p.assembly.as_mut().unwrap().components {
        c.pose = f.transform_pose(&c.pose);
    }
    let after = run(&p);
    assert!((before.budget.detected - after.budget.detected).abs() < 1e-11);
    assert_eq!(
        after.budget.numerical_failure + after.budget.medium_mismatch,
        0.0
    );
    let a: Vec<_> = before
        .terminals
        .iter()
        .filter(|t| t.reason == "detected")
        .collect();
    let b: Vec<_> = after
        .terminals
        .iter()
        .filter(|t| t.reason == "detected")
        .collect();
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(a.ray.id, b.ray.id);
        assert!((f.world_point(a.ray.position_mm) - b.ray.position_mm).norm() < 1e-8);
        assert!((f.world_vector(a.ray.direction) - b.ray.direction).norm() < 1e-10);
        assert!((a.ray.optical_path_mm - b.ray.optical_path_mm).abs() < 1e-8);
    }
}

#[test]
fn pose_composition_handles_gimbal_angles_and_inverse_transforms() {
    for rotation in [[20.0, 35.0, 70.0], [10.0, 90.0, 25.0], [20.0, -90.0, 70.0]] {
        let parent = Frame::new(&Pose {
            translation_mm: V3([4.0, 5.0, 6.0]),
            rotation_deg: V3(rotation),
        });
        let child = Pose {
            translation_mm: V3([1.0, -2.0, 3.0]),
            rotation_deg: V3([13.0, 22.0, -15.0]),
        };
        let composed = Frame::new(&parent.transform_pose(&child));
        let original = Frame::new(&child);
        let recovered = Frame::new(&parent.inverse_pose(&parent.transform_pose(&child)));
        for point in [V3([2.0, 3.0, 4.0]), V3([0.0, 0.0, 1.0])] {
            assert!(
                (composed.world_point(point) - parent.world_point(original.world_point(point)))
                    .norm()
                    < 1e-12
            );
            assert!((recovered.world_point(point) - original.world_point(point)).norm() < 1e-12);
        }
    }
}

#[test]
fn lens_barrel_has_outward_normal_and_nearest_positive_root() {
    let (p, _, _, _) = assembled();
    let mut scene = p.trace_scene().unwrap();
    let i = scene
        .surfaces
        .iter()
        .position(|s| matches!(s.shape, Shape::ConicalSide { .. }))
        .unwrap();
    scene.surfaces[i].pose = Pose::default();
    scene.surfaces[i].shape = Shape::ConicalSide {
        radius_start_mm: 2.0,
        radius_end_mm: 1.0,
        length_mm: 4.0,
    };
    let scene = scene.prepare().unwrap();
    let s = &scene.surfaces[i];
    let hit = intersect(s, V3([0.0, 0.0, 2.0]), V3([1.0, 0.0, 0.0])).unwrap();
    assert!((hit.distance_mm - 1.5).abs() < 1e-12);
    assert!(hit.normal.0[0] > 0.9 && hit.normal.0[2] > 0.0);
    let hit = intersect(s, V3([4.0, 0.0, 2.0]), V3([-1.0, 0.0, 0.0])).unwrap();
    assert!((hit.distance_mm - 2.5).abs() < 1e-12);
}

#[test]
fn oblique_asphere_finds_the_first_of_two_intersections_and_excludes_misses() {
    let (p, _, _, _) = assembled();
    let mut scene = p.trace_scene().unwrap();
    scene.surfaces[0].pose = Pose::default();
    scene.surfaces[0].shape = Shape::EvenAsphere {
        radius_mm: 100.0,
        conic: -1.0,
        coefficients: vec![],
    };
    scene.surfaces[0].aperture = Aperture::Circle { radius_mm: 10.0 };
    let prepared = scene.prepare().unwrap();
    let surface = &prepared.surfaces[0];
    for sign in [-1.0, 1.0] {
        let hit = intersect(surface, V3([-9.0 * sign, 0.0, 0.1]), V3([sign, 0.0, 0.0])).unwrap();
        assert!((hit.distance_mm - (9.0 - 20f64.sqrt())).abs() < 2e-9);
    }
    assert!(matches!(
        intersect(surface, V3([-9.0, 8.0, 0.1]), V3([1.0, 0.0, 0.0])),
        Err(raytrace_rs::geometry::Miss::Geometry)
    ));
}

#[test]
fn disabled_parts_do_not_block_and_import_does_not_override_material_safety() {
    let (mut p, _, _, _) = assembled();
    p.assembly.as_mut().unwrap().components[1]
        .project
        .issues
        .push(Issue {
            code: "coating".into(),
            message: "Synthetic coating".into(),
            blocking: true,
        });
    assert!(p.trace_scene().is_err());
    p.allow_uncoated = true;
    assert!(p.trace_scene().is_ok());
    p.allow_uncoated = false;
    p.assembly.as_mut().unwrap().components[1].enabled = false;
    assert!(p.trace_scene().is_ok());
    let before = p.trace_scene().unwrap().surfaces.len();
    p.assembly.as_mut().unwrap().components.remove(2);
    assert_eq!(p.trace_scene().unwrap().surfaces.len(), before - 6);
}

#[test]
fn source_editor_changes_are_kept_in_component_local_coordinates() {
    let (mut p, _, _, _) = assembled();
    let old = p.scene.sources[0].origin_mm;
    p.scene.sources[0].origin_mm.0[0] += 1.25;
    p.scene.sources[0].count = 256;
    p.assembly.as_mut().unwrap().sync_sources(&p.scene);
    p.refresh_assembly().unwrap();
    assert!((p.scene.sources[0].origin_mm.0[0] - old.0[0] - 1.25).abs() < 1e-12);
    assert_eq!(p.scene.sources[0].count, 256);
    p.assembly.as_mut().unwrap().components[0]
        .pose
        .translation_mm
        .0[0] += 3.0;
    p.refresh_assembly().unwrap();
    assert!((p.scene.sources[0].origin_mm.0[0] - old.0[0] - 4.25).abs() < 1e-12);
}

#[test]
fn demo_traces_source_lens_rod_and_detector_with_tir_and_closed_budget() {
    let p = assembly::demo_project().unwrap();
    let report = run(&p);
    assert!(report.budget.detected > 0.75, "{:?}", report.budget);
    assert!(report.total_internal_reflections > 0);
    assert_eq!(
        report.budget.medium_mismatch + report.budget.numerical_failure,
        0.0
    );
    assert!((report.budget.accounted() - report.budget.launched).abs() < 1e-10);
}
