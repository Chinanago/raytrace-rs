// Tests and examples may panic on failure; the no-panic rule (DESIGN.md P8) covers product code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use raytrace_rs::{
    design::{self, Component, Design, Part, PartKind},
    math::{Frame, Pose, V3},
    model::Interaction,
    trace::{TraceOptions, trace},
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
            "illumination_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
/// The single unplaced system of a design.
fn part(design: &Design) -> &Part {
    design.lone_system().unwrap()
}
fn part_mut(design: &mut Design) -> &mut Part {
    design.lone_system_mut().unwrap()
}
fn zmx_text(design: &Design) -> String {
    zemax::document(part(design)).unwrap().unwrap().text
}
fn finite(dir: &std::path::Path) -> Design {
    let path = dir.join("finite.zmx");
    fs::write(&path,"MODE SEQ\nENPD 4\nFTYP 1 0 2 2\nXFLN -1 2\nYFLN 0 0\nFWGN 1 3\nWAVM 1 .55 1\nSURF 0\nTYPE STANDARD\nDISZ 20\nSURF 1\nTYPE STANDARD\nSTOP\nDIAM 2\nDISZ 10\nSURF 2\nTYPE STANDARD\nDIAM 10\nDISZ 0\n").unwrap();
    zemax::read_zmx(&path, &[]).unwrap()
}
#[test]
fn finite_objects_fill_the_bound_pupil_and_preserve_field_weights_after_composition() {
    let dir = folder();
    let mut p = finite(&dir);
    assert!(
        !part(&p).issues.iter().any(|i| i.blocking),
        "{:?}",
        part(&p).issues
    );
    assert_eq!(part(&p).scene.sources.len(), 2);
    assert_eq!(part(&p).scene.sources[0].total_weight, 0.25);
    assert_eq!(part(&p).scene.sources[1].total_weight, 0.75);
    for s in &mut part_mut(&mut p).scene.sources {
        s.count = 1024;
    }
    part_mut(&mut p).scene.surfaces[0].pose.translation_mm.0[2] = 3.;
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    for (i, r) in rays.iter().enumerate() {
        let s = &prepared.spec.sources[i / 1024];
        assert_eq!(r.position_mm, s.origin_mm);
        let hit = r.position_mm + r.direction * ((3. - r.position_mm.0[2]) / r.direction.0[2]);
        assert!(hit.0[0].hypot(hit.0[1]) < 2.);
        assert!(r.direction.0[2] > 0.);
    }
    let report = trace(&prepared, &rays, TraceOptions::default()).unwrap();
    assert!((report.budget.detected - 1.).abs() < 1e-12);
    let mut c = Component::imported(p.clone(), PartKind::System).unwrap();
    c.occurrence.pose = Pose {
        translation_mm: V3([3., 4., 5.]),
        rotation_deg: V3([12., 18., 25.]),
    };
    let frame = Frame::new(&c.occurrence.pose);
    let mut a = Design::empty();
    a.add_component(Component::detector()).unwrap();
    a.add_component(c).unwrap();
    let flat = a.compile().unwrap().scene;
    assert_eq!(flat.sources[0].pupil.as_ref().unwrap().surface, Some(1));
    let transformed = flat.prepare().unwrap().generate_rays().unwrap();
    for (r, t) in rays.iter().zip(&transformed) {
        assert!((frame.world_point(r.position_mm) - t.position_mm).norm() < 1e-12);
        assert!((frame.world_vector(r.direction) - t.direction).norm() < 1e-12);
    }
    let flat = a.compile().unwrap().scene;
    a.sync_sources(&flat);
    let round = a.compile().unwrap().scene;
    for (a, b) in round.sources.iter().zip(&flat.sources) {
        assert!((a.origin_mm - b.origin_mm).norm() < 1e-12);
        assert!((a.direction - b.direction).norm() < 1e-12);
        assert_eq!(
            a.pupil.as_ref().unwrap().surface,
            b.pupil.as_ref().unwrap().surface
        );
    }
}
#[test]
fn infinite_angle_fields_fill_stop_and_native_edits_preserve_object_definition() {
    let dir = folder();
    let mut p = finite(&dir);
    let path = dir.join("angle.zmx");
    let text = zmx_text(&p)
        .replace("FTYP 1 0 2 2", "FTYP 0 0 2 2")
        .replace("DISZ 20", "DISZ INFINITY");
    fs::write(&path, text).unwrap();
    p = zemax::read_zmx(&path, &[]).unwrap();
    assert!(
        !part(&p).issues.iter().any(|i| i.blocking),
        "{:?}",
        part(&p).issues
    );
    part_mut(&mut p).scene.sources[0].count = 256;
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    for r in rays.iter().take(256) {
        assert!((r.direction.0[0] / r.direction.0[2] - (-1f64).to_radians().tan()).abs() < 1e-14);
        assert_eq!(r.position_mm.0[2], -1.);
        let hit = r.position_mm + r.direction * (-r.position_mm.0[2] / r.direction.0[2]);
        assert!(hit.0[0].hypot(hit.0[1]) < 2.);
    }
    let out = dir.join("edited.zmx");
    zemax::export(&p, &out).unwrap();
    let q = zemax::read_zmx(&out, &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&part(&p).scene.sources[0].pupil).unwrap(),
        serde_json::to_value(&part(&q).scene.sources[0].pupil).unwrap()
    );
    assert_eq!(
        part(&p).scene.sources[0].direction,
        part(&q).scene.sources[0].direction
    );
    part_mut(&mut p).scene.sources[0].wavelength_um = 0.635;
    assert!(zemax::export(&p, &dir.join("incompatible_fields.zmx")).is_err());
}
#[test]
fn unsupported_fields_block_system_but_keep_optic_import_available() {
    let dir = folder();
    let p = finite(&dir);
    let base = &zmx_text(&p);
    for (i, text) in [
        base.replace("FTYP 1", "FTYP 3"),
        base.replace("FWGN 1 3", "FWGN -1 3"),
        base.replace("WAVM 1", "RAIM 3 0 0\nWAVM 1"),
        base.replace("STOP\n", ""),
        base.replace("WAVM 1", "VCXN 1.1 0\nWAVM 1"),
    ]
    .iter()
    .enumerate()
    {
        let path = dir.join(format!("bad{i}.zmx"));
        fs::write(&path, text).unwrap();
        let p = zemax::read_zmx(&path, &[]).unwrap();
        assert!(
            part(&p)
                .issues
                .iter()
                .any(|i| i.blocking && i.code == "illumination_definition"),
            "{:?}",
            part(&p).issues
        );
        assert!(p.trace_scene().is_err());
        if i > 0 {
            let mut p = p;
            part_mut(&mut p).scene.surfaces[0].interaction =
                Interaction::Mirror { reflectance: 1. };
            let optic = design::imported_component(p, PartKind::Optic).unwrap();
            assert!(
                !optic
                    .part
                    .issues
                    .iter()
                    .any(|i| i.code == "illumination_definition")
            );
        }
    }
}
#[test]
fn folded_native_mirror_then_glass_preserves_negative_gaps_and_optical_path() {
    let dir = folder();
    let path = dir.join("folded.zmx");
    fs::write(
        dir.join("own.AGF"),
        "NM OWN 5 0 1.5 50 0 0 0\nCD 1.5 0 0\nLD .4 .8\n",
    )
    .unwrap();
    fs::write(&path,"MODE SEQ\nGCAT own\nENPD .2\nWAVM 1 .55 1\nSURF 0\nTYPE STANDARD\nDISZ INFINITY\nSURF 1\nTYPE STANDARD\nDIAM 2\nDISZ 10\nSURF 2\nTYPE STANDARD\nGLAS MIRROR\nDIAM 2\nDISZ -5\nSURF 3\nTYPE STANDARD\nGLAS OWN 0 0\nDIAM 2\nDISZ -1\nSURF 4\nTYPE STANDARD\nDIAM 2\nDISZ -1\nSURF 5\nTYPE STANDARD\nDIAM 2\nDISZ 0\n").unwrap();
    let mut p = zemax::read_zmx(&path, &[]).unwrap();
    assert!(
        !part(&p).issues.iter().any(|i| i.blocking),
        "{:?}",
        part(&p).issues
    );
    assert!(matches!(
        part(&p).scene.surfaces[1].interaction,
        Interaction::Mirror { .. }
    ));
    part_mut(&mut p).scene.sources[0].count = 1;
    let prepared = p.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let options = TraceOptions {
        keep_terminals: true,
        ..TraceOptions::default()
    };
    let r = trace(&prepared, &rays, options).unwrap();
    assert!((r.budget.detected - 0.9216).abs() < 1e-12);
    assert!(
        (r.terminals
            .iter()
            .find(|t| t.reason == "detected")
            .unwrap()
            .ray
            .optical_path_mm
            - 18.5)
            .abs()
            < 1e-10
    );
    let out = dir.join("edited.zmx");
    zemax::export(&p, &out).unwrap();
    let q = zemax::read_zmx(&out, &[]).unwrap();
    let prepared = q.trace_scene().unwrap().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let after = trace(&prepared, &rays, options).unwrap();
    assert!((after.budget.detected - r.budget.detected).abs() < 1e-12);
    assert!(after.budget.medium_mismatch < 1e-12);
}
