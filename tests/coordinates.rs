use raytrace_rs::{
    assembly::{Component, ComponentKind},
    math::{Frame, Pose, V3},
    model::Ray,
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
            "coordinate_tests_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn near(a: V3, b: V3) {
    assert!((a - b).norm() < 1e-10, "{a:?} != {b:?}");
}
// Apply Rx Ry Rz to a vector independently of the implementation's matrix representation.
fn intrinsic(mut p: V3, angles: V3) -> V3 {
    for axis in [2, 1, 0] {
        let (s, c) = angles.0[axis].to_radians().sin_cos();
        let [x, y, z] = p.0;
        p = V3(match axis {
            0 => [x, c * y - s * z, s * y + c * z],
            1 => [c * x + s * z, y, -s * x + c * z],
            _ => [c * x - s * y, s * x + c * y, z],
        });
    }
    p
}
#[test]
fn intrinsic_rotations_convert_to_editor_angles_and_back_including_gimbal_lock() {
    for a in [
        [12., 23., 34.],
        [-70., -25., 161.],
        [10., 90., 30.],
        [10., -90., 30.],
    ] {
        let p = Pose {
            translation_mm: V3([2., 3., 4.]),
            rotation_deg: V3(a),
        };
        let f = Frame::from_intrinsic_xyz(&p);
        let canonical = Frame::new(&f.pose());
        let restored = Frame::from_intrinsic_xyz(&canonical.intrinsic_xyz_pose());
        for v in [V3([1., 0., 0.]), V3([0., 1., 0.]), V3([0., 0., 1.])] {
            near(f.world_vector(v), intrinsic(v, p.rotation_deg));
            near(f.world_vector(v), canonical.world_vector(v));
            near(f.world_vector(v), restored.world_vector(v));
        }
    }
}
#[test]
fn coordinate_break_compound_inverse_restores_position_and_orientation() {
    let dir = folder();
    let path = dir.join("chain.zmx");
    fs::write(&path,"MODE SEQ\nUNIT MM\nWAVM 1 0.55 1\nSURF 0\n DISZ INFINITY\nSURF 1\n TYPE COORDBRK\n PARM 1 3\n PARM 2 5\n PARM 3 12\n PARM 4 23\n PARM 5 34\n PARM 6 0\n DISZ 0\nSURF 2\n TYPE STANDARD\n DISZ 0\nSURF 3\n TYPE COORDBRK\n PARM 1 -3\n PARM 2 -5\n PARM 3 -12\n PARM 4 -23\n PARM 5 -34\n PARM 6 1\n DISZ 0\nSURF 4\n TYPE STANDARD\n DISZ 0\n").unwrap();
    let p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_ok(), "{:?}", p.issues);
    assert_eq!(p.scene.surfaces.len(), 2);
    near(p.scene.surfaces[0].pose.translation_mm, V3([3., 5., 0.]));
    near(p.scene.surfaces[1].pose.translation_mm, V3::default());
    near(
        Frame::new(&p.scene.surfaces[1].pose).world_vector(V3([0., 0., 1.])),
        V3([0., 0., 1.]),
    );
    assert_eq!(p.zemax.as_ref().unwrap().surface_numbers, vec![2, 4]);
    zemax::export(&p, &dir.join("same.zmx")).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        fs::read(dir.join("same.zmx")).unwrap()
    );
}
#[test]
fn tilted_lens_thickness_optical_path_and_assembly_are_preserved() {
    let dir = folder();
    let path = dir.join("tilted.zmx");
    fs::write(
        dir.join("TEST.AGF"),
        "NM TEST 1 0 1.5 50 0 0\nCD 2.25 0 0 0 0 0\nLD 0.4 0.7\n",
    )
    .unwrap();
    fs::write(&path,"MODE SEQ\nUNIT MM\nGCAT TEST\nWAVM 1 0.55 1\nSURF 0\n DISZ INFINITY\nSURF 1\n TYPE COORDBRK\n PARM 1 3\n PARM 2 5\n PARM 3 12\n PARM 4 23\n PARM 5 34\n DISZ 2\nSURF 2\n TYPE STANDARD\n GLAS TEST 0 0\n DIAM 2\n DISZ 5\nSURF 3\n TYPE STANDARD\n DIAM 2\n DISZ 3\nSURF 4\n TYPE STANDARD\n DIAM 2\n DISZ 0\n").unwrap();
    let p = Project::open(&path, &[]).unwrap();
    let norm = intrinsic(V3([0., 0., 1.]), V3([12., 23., 34.]));
    near(
        p.scene.surfaces[0].pose.translation_mm,
        V3([3., 5., 0.]) + norm * 2.,
    );
    let ray = Ray {
        id: 0,
        position_mm: p.scene.surfaces[0].pose.translation_mm - norm * 2.,
        direction: norm,
        weight: 1.,
        wavelength_um: 0.55,
        medium: 0,
        optical_path_mm: 0.,
    };
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let r = trace(
        &prepared,
        &[ray],
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 1,
        },
    );
    assert!((r.budget.detected - 0.9216).abs() < 1e-12);
    let terminal = r.terminals.iter().find(|t| t.reason == "detected").unwrap();
    assert!((terminal.ray.optical_path_mm - 12.5).abs() < 1e-10);
    let lens = Component::imported(p, ComponentKind::Optic).unwrap();
    assert_eq!(lens.project.scene.surfaces.len(), 3); // two optical faces and one barrel
    let barrel = &lens.project.scene.surfaces[2];
    near(
        Frame::new(&barrel.pose).world_vector(V3([0., 0., 1.])),
        norm,
    );
}
#[test]
fn nsc_reference_hierarchy_and_compound_tilts_hit_the_referenced_detector() {
    let dir = folder();
    let path = dir.join("references.zmx");
    fs::write(&path,"MODE NSC\nUNIT MM\nWAVM 1 0.55 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_NULL 0 0\n NSOP 2 3 4 12 23 34\n NSOH NSC_NULL 1 0\n NSOP 1 0 5 -5 7 11\n NSOH NSC_SDIO -1 0\n NSOP 0 0 -1 0 0 0\n NSOD 2 256\n NSOD 3 1\n NSOD 7 0\n NSOD 9 0\n NSOH NSC_DETC 2 0\n NSOP 0 0 10 0 0 0 ABSORB\n NSOD 1 10\n NSOD 2 10\n NSOD 3 4\n NSOD 4 4\n").unwrap();
    let p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_ok(), "{:?}", p.issues);
    let a = V3([12., 23., 34.]);
    let b = V3([-5., 7., 11.]);
    let origin = V3([2., 3., 4.]) + intrinsic(V3([1., 0., 5.]), a);
    let axis = intrinsic(intrinsic(V3([0., 0., 1.]), b), a);
    near(p.scene.sources[0].origin_mm, origin - axis);
    near(p.scene.sources[0].direction, axis);
    near(p.scene.surfaces[0].pose.translation_mm, origin + axis * 10.);
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let r = trace(&prepared, &rays, TraceOptions::default());
    assert!((r.budget.detected - 1.).abs() < 1e-12);
    let mut moved = p.clone();
    moved.scene.sources[0].origin_mm.0[0] += 1.;
    zemax::export(&moved, &dir.join("edited.zmx")).unwrap();
    let reopened = Project::open(&dir.join("edited.zmx"), &[]).unwrap();
    near(
        reopened.scene.sources[0].origin_mm,
        moved.scene.sources[0].origin_mm,
    );
    near(
        reopened.scene.sources[0].direction,
        moved.scene.sources[0].direction,
    );
    near(
        reopened.scene.surfaces[0].pose.translation_mm,
        moved.scene.surfaces[0].pose.translation_mm,
    );
}
#[test]
fn invalid_references_and_inside_of_missing_volumes_block_tracing() {
    let dir = folder();
    let path = dir.join("invalid.zmx");
    let base="MODE NSC\nUNIT MM\nWAVM 1 0.55 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_NULL 0 0\n NSOP 0 0 0 0 0 0\n NSOH NSC_SDIO REF INSIDE\n NSOP 0 0 0 0 0 0\n";
    for (reference, inside) in [("2", "0"), ("3", "0"), ("-3", "0"), ("0", "1")] {
        fs::write(
            &path,
            base.replace("REF", reference).replace("INSIDE", inside),
        )
        .unwrap();
        assert!(Project::open(&path, &[]).unwrap().trace_scene().is_err());
    }
}
