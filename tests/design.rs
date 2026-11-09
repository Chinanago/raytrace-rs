//! The design document compiles to the scenes that the previous project model produced.
//! Files of the pre-beta project formats are not read.
// Tests and examples may panic on failure; the no-panic rule (DESIGN.md P8) covers product code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use raytrace_rs::{
    design::{self, Component, Design, PartKind},
    math::V3,
    model::*,
};
use std::fs;

/// Drop the sign of zero array entries. A placement without transform now keeps `0.0`
/// where the previous model recomputed `-0.0`; the traced reports are identical.
fn without_negative_zero(text: &str) -> String {
    let (bytes, mut out) = (text.as_bytes(), Vec::new());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"-0.0")
            && i > 0
            && b"[,:".contains(&bytes[i - 1])
            && bytes.get(i + 4).is_some_and(|c| b",]}".contains(c))
        {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap()
}
/// FNV-1a over the JSON of the traced scene, material names and issues.
fn fingerprint(design: &Design) -> String {
    let scene = design.trace_scene().unwrap();
    let compiled = design.compile().unwrap();
    let text =
        serde_json::to_string(&(&scene, &compiled.material_names, &compiled.issues)).unwrap();
    let text = without_negative_zero(&text);
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
fn example(name: &str) -> Scene {
    serde_json::from_str(&fs::read_to_string(format!("examples/{name}.json")).unwrap()).unwrap()
}

#[test]
fn examples_and_assemblies_compile_to_the_scenes_of_the_previous_project_model() {
    // Fingerprints captured from `Project::trace_scene` before the project model was removed.
    let mut actual = Vec::new();
    for name in [
        "folded_mirror",
        "integrator_rod",
        "nonsequential_slab",
        "sequential_lens",
    ] {
        actual.push((name, fingerprint(&Design::from_scene(example(name)))));
    }
    let demo = design::demo_design().unwrap();
    actual.push(("demo", fingerprint(&demo)));

    let mut d = demo.clone();
    d.occurrences[0].pose.translation_mm = V3([2.0, -3.0, 8.0]);
    d.occurrences[0].pose.rotation_deg = V3([7.0, 19.0, 31.0]);
    d.occurrences[1].pose.rotation_deg = V3([1.0, 2.0, 3.0]);
    d.occurrences[2].pose.rotation_deg = V3([0.0, 0.0, 10.0]);
    d.occurrence_part_mut(2)
        .unwrap()
        .rod
        .as_mut()
        .unwrap()
        .width_mm = 2.5;
    d.occurrences[3].enabled = false;
    actual.push(("demo_transformed", fingerprint(&d)));

    let mut d = demo.clone();
    for o in &mut d.occurrences {
        o.pose = Default::default();
    }
    actual.push(("demo_identity", fingerprint(&d)));

    let mut d = demo.clone();
    d.conditions.allow_uncoated = true;
    d.conditions.monochromatic_override_um = Some(0.5);
    actual.push(("demo_conditions", fingerprint(&d)));

    let mut d = demo.clone();
    d.occurrence_part_mut(0)
        .unwrap()
        .conditions
        .monochromatic_override_um = Some(0.6);
    d.occurrence_part_mut(1).unwrap().conditions.allow_uncoated = true;
    actual.push(("demo_part_conditions", fingerprint(&d)));

    let lens = example("sequential_lens");
    let mut d = Design::from_scene(lens.clone());
    let mut detector = Component::detector();
    detector.occurrence.pose.translation_mm.0[2] = 120.0;
    d.add_component(detector).unwrap();
    actual.push(("lens_plus_detector", fingerprint(&d)));

    let mut d = Design::from_scene(lens.clone());
    d.conditions.allow_uncoated = true;
    d.add_component(Component::rod(lens.materials[1].clone(), "Silica".into()))
        .unwrap();
    actual.push(("lens_plus_rod_uncoated", fingerprint(&d)));

    let slab = Design::from_scene(example("nonsequential_slab"));
    let mut c = Component::imported(slab, PartKind::System).unwrap();
    c.occurrence.pose.translation_mm = V3([1.0, 2.0, 3.0]);
    let mut d = Design::empty();
    d.add_component(c).unwrap();
    actual.push(("slab_as_system", fingerprint(&d)));

    let expected = [
        ("folded_mirror", "0795b977634282de"),
        ("integrator_rod", "4eb2b222e838cadb"),
        ("nonsequential_slab", "5f644ed02b1a01d9"),
        ("sequential_lens", "aade188817b419c9"),
        ("demo", "a9ad4da0b1d6e625"),
        ("demo_transformed", "f30878423e39e3d5"),
        ("demo_identity", "73223aeb3ed65497"),
        ("demo_conditions", "3e6ffacc0a08c92a"),
        ("demo_part_conditions", "57d7a08665b856d8"),
        ("lens_plus_detector", "74352c4e3b290ece"),
        ("lens_plus_rod_uncoated", "b5bbde4018d0e1c2"),
        ("slab_as_system", "7ac8d99bab15b504"),
    ];
    let mut mismatches = Vec::new();
    for ((name, hash), (expected_name, expected_hash)) in actual.iter().zip(expected) {
        assert_eq!(*name, expected_name);
        if hash != expected_hash {
            mismatches.push(format!("{name}: {hash} != {expected_hash}"));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

fn folder() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/design_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn project_files_of_earlier_builds_and_run_records_are_refused_with_a_reason() {
    let dir = folder();
    for format in [
        "raytrace-project-v1",
        "raytrace-project-v2",
        "raytrace-project-v3",
    ] {
        let path = dir.join("project.json");
        fs::write(&path, format!(r#"{{"format":"{format}","scene":{{}}}}"#)).unwrap();
        let message = design::open(&path, &[]).unwrap_err().to_string();
        assert!(
            message.contains("未対応のファイル形式")
                && message.contains(format)
                && message.contains("raytrace-design-v1"),
            "{message}"
        );
    }
    let record = dir.join("record.json");
    fs::write(&record, r#"{"format":"raytrace-run-v1"}"#).unwrap();
    let message = design::open(&record, &[]).unwrap_err().to_string();
    assert!(message.starts_with("計算記録は"), "{message}");
}
