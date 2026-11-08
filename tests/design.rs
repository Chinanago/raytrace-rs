//! The design document compiles to the scenes that the previous project model produced,
//! and files of earlier builds (project v1 to v3) still open.
use raytrace_rs::{
    compatibility::ComponentCompatibility,
    design::{self, Component, Design, PartKind},
    engine::{self, ModelInput, Overrides, RecordInput, ReplayRequest, Request, TraceRequest},
    math::V3,
    model::*,
    record::RunRecord,
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

// ---------------------------------------------------------------------------------------
// Files of earlier builds
// ---------------------------------------------------------------------------------------

/// `raytrace-project-v1` written by the previous build: one scene, no assembly.
const LEGACY_V1: &str = r#"{"format":"raytrace-project-v1","scene":{"name":"tiny","reference_plane":"launch","mode":"non_sequential","materials":[{"type":"constant","n":1.0,"absorption_per_mm":0.0}],"surfaces":[{"name":"wall","pose":{"translation_mm":[0.0,0.0,5.0],"rotation_deg":[0.0,0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":3.0},"interaction":{"type":"detector","pixels":[4,4]}}],"settings":{"max_depth":64,"max_events_per_launch":4096,"min_relative_branch_weight":1e-7},"sources":[{"sampling":"sobol","count":16,"origin_mm":[0.0,0.0,0.0],"direction":[0.0,0.0,1.0],"disk_radius_mm":1.0,"cone_half_angle_deg":0.0,"wavelength_um":0.55,"medium":0,"total_weight":1.0,"seed":7}]},"material_names":["AIR"],"issues":[],"zemax":null,"allow_uncoated":false,"monochromatic_override_um":null}"#;
/// `raytrace-project-v2` written by the previous build: a source (monochromatic override
/// 0.6 um on its own project), a rod and a screen, with the stale composed scene on top.
const LEGACY_V2: &str = r#"{"assembly":{"components":[{"name":"tiny","enabled":true,"kind":"source","pose":{"translation_mm":[0.5,0.0,-2.0],"rotation_deg":[0.0,5.0,0.0]},"anchor_mm":[0.0,0.0,0.0],"project":{"format":"raytrace-project-v1","scene":{"name":"tiny","reference_plane":"launch","mode":"non_sequential","materials":[{"type":"constant","n":1.0,"absorption_per_mm":0.0}],"surfaces":[],"settings":{"max_depth":64,"max_events_per_launch":4096,"min_relative_branch_weight":1e-7},"sources":[{"sampling":"sobol","count":16,"origin_mm":[0.0,0.0,0.0],"direction":[0.0,0.0,1.0],"disk_radius_mm":1.0,"cone_half_angle_deg":0.0,"wavelength_um":0.55,"medium":0,"total_weight":1.0,"seed":7}]},"material_names":["AIR"],"issues":[],"zemax":null,"allow_uncoated":false,"monochromatic_override_um":0.6},"rod":null},{"name":"ロッド","enabled":true,"kind":"rod","pose":{"translation_mm":[0.0,0.0,1.0],"rotation_deg":[0.0,0.0,0.0]},"anchor_mm":[0.0,0.0,0.0],"project":{"format":"raytrace-project-v1","scene":{"name":"ロッド","reference_plane":"合成光源の発射面","mode":"non_sequential","materials":[{"type":"constant","n":1.0,"absorption_per_mm":0.0},{"type":"constant","n":1.5,"absorption_per_mm":0.0}],"surfaces":[],"settings":{"max_depth":128,"max_events_per_launch":4096,"min_relative_branch_weight":1e-7},"sources":[]},"material_names":["AIR","TESTGLASS"],"issues":[],"zemax":null,"allow_uncoated":false,"monochromatic_override_um":null},"rod":{"width_mm":1.0,"height_mm":1.0,"length_mm":4.0}},{"name":"スクリーン","enabled":true,"kind":"detector","pose":{"translation_mm":[0.0,0.0,8.0],"rotation_deg":[0.0,0.0,0.0]},"anchor_mm":[0.0,0.0,0.0],"project":{"format":"raytrace-project-v1","scene":{"name":"スクリーン","reference_plane":"合成光源の発射面","mode":"non_sequential","materials":[{"type":"constant","n":1.0,"absorption_per_mm":0.0}],"surfaces":[{"name":"検出面","pose":{"translation_mm":[0.0,0.0,0.0],"rotation_deg":[0.0,0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":10.0,"half_height_mm":10.0},"interaction":{"type":"detector","pixels":[64,64]}}],"settings":{"max_depth":128,"max_events_per_launch":4096,"min_relative_branch_weight":1e-7},"sources":[]},"material_names":["AIR"],"issues":[],"zemax":null,"allow_uncoated":false,"monochromatic_override_um":null},"rod":null}]},"format":"raytrace-project-v2","scene":{"name":"組立光学系","reference_plane":"合成光源の発射面","mode":"non_sequential","materials":[{"type":"constant","n":1.0,"absorption_per_mm":0.0},{"type":"constant","n":1.5,"absorption_per_mm":0.0}],"surfaces":[{"name":"2:ロッド / 入口","pose":{"translation_mm":[0.0,0.0,1.0],"rotation_deg":[0.0,-0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":0.5,"half_height_mm":0.5},"interaction":{"type":"dielectric","minus_medium":0,"plus_medium":1,"reflectance":null}},{"name":"2:ロッド / 出口","pose":{"translation_mm":[0.0,0.0,5.0],"rotation_deg":[0.0,-0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":0.5,"half_height_mm":0.5},"interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0,"reflectance":null}},{"name":"2:ロッド / 側面 +X","pose":{"translation_mm":[0.5,0.0,3.0],"rotation_deg":[0.0,90.0,-0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":2.0,"half_height_mm":0.5},"interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0,"reflectance":null}},{"name":"2:ロッド / 側面 -X","pose":{"translation_mm":[-0.5,0.0,3.0],"rotation_deg":[0.0,-90.0,-0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":2.0,"half_height_mm":0.5},"interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0,"reflectance":null}},{"name":"2:ロッド / 側面 +Y","pose":{"translation_mm":[0.0,0.5,3.0],"rotation_deg":[-90.0,-0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":0.5,"half_height_mm":2.0},"interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0,"reflectance":null}},{"name":"2:ロッド / 側面 -Y","pose":{"translation_mm":[0.0,-0.5,3.0],"rotation_deg":[90.0,-0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":0.5,"half_height_mm":2.0},"interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0,"reflectance":null}},{"name":"3:スクリーン / 検出面","pose":{"translation_mm":[0.0,0.0,8.0],"rotation_deg":[0.0,-0.0,0.0]},"shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":10.0,"half_height_mm":10.0},"interaction":{"type":"detector","pixels":[64,64]}}],"settings":{"max_depth":128,"max_events_per_launch":4096,"min_relative_branch_weight":1e-7},"sources":[{"sampling":"sobol","count":16,"origin_mm":[0.5,0.0,-2.0],"direction":[0.08715574274765817,0.0,0.9961946980917455],"disk_radius_mm":1.0,"cone_half_angle_deg":0.0,"wavelength_um":0.6,"medium":0,"total_weight":1.0,"seed":7}]},"material_names":["AIR","ロッド / TESTGLASS"],"issues":[],"zemax":null,"allow_uncoated":false,"monochromatic_override_um":null}"#;

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
fn legacy_standalone_project_opens_as_one_unplaced_system_with_the_same_scene() {
    let dir = folder();
    let path = dir.join("v1.json");
    fs::write(&path, LEGACY_V1).unwrap();
    let design = design::open(&path, &[]).unwrap();
    assert_eq!(design.format, design::FORMAT);
    assert_eq!(design.parts.len(), 1);
    assert_eq!(design.occurrences.len(), 1);
    assert_eq!(design.parts[0].kind, PartKind::System);
    assert!(design.lone_system().is_some());
    assert_eq!(design.parts[0].id.as_str(), "system-1");
    assert_eq!(design.occurrences[0].id.as_str(), "system-2");
    assert_eq!(design.next_id, 3);
    // Captured from `Project::trace_scene` of the previous build.
    assert_eq!(fingerprint(&design), "aa2af697462d5e23");
    // Saving writes the new format, which opens to the same document.
    let saved = dir.join("v1.design.json");
    design::save(&design, &saved).unwrap();
    let again = design::open(&saved, &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&design).unwrap()
    );
}

#[test]
fn legacy_assembly_project_opens_as_one_part_and_occurrence_per_component() {
    let dir = folder();
    let path = dir.join("v2.json");
    fs::write(&path, LEGACY_V2).unwrap();
    let design = design::open(&path, &[]).unwrap();
    assert!(design.is_assembly());
    assert_eq!(design.mode, Mode::NonSequential);
    let kinds: Vec<_> = design.parts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, [PartKind::Source, PartKind::Rod, PartKind::Detector]);
    let ids: Vec<_> = design
        .occurrences
        .iter()
        .map(|o| (o.id.as_str(), o.part.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            ("source-2", "source-1"),
            ("rod-4", "rod-3"),
            ("detector-6", "detector-5")
        ]
    );
    let names: Vec<_> = design.occurrences.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["tiny", "ロッド", "スクリーン"]);
    assert_eq!(
        design.occurrences[0].pose.translation_mm,
        V3([0.5, 0.0, -2.0])
    );
    assert_eq!(design.occurrences[0].pose.rotation_deg, V3([0.0, 5.0, 0.0]));
    assert_eq!(
        design.occurrences[2].pose.translation_mm,
        V3([0.0, 0.0, 8.0])
    );
    assert_eq!(design.parts[1].rod.as_ref().unwrap().length_mm, 4.0);
    // The component's own override moved to its part; the top level has none.
    assert_eq!(
        design.parts[0].conditions.monochromatic_override_um,
        Some(0.6)
    );
    assert_eq!(design.conditions.monochromatic_override_um, None);
    // Captured from `Project::trace_scene` of the previous build.
    assert_eq!(fingerprint(&design), "040297ff1f2bfd8b");
    let report = design.compatibility_report();
    assert!(report.traceable);
    assert_eq!(
        report.components[0].occurrence.as_ref().unwrap().as_str(),
        "source-2"
    );
    assert_eq!(report.components[0].effective_wavelengths_um, vec![0.6]);
    assert_eq!(report.components[1].source_format, "generated");
}

#[test]
fn project_files_with_an_unknown_format_or_a_run_record_are_refused_with_a_reason() {
    let dir = folder();
    let path = dir.join("unknown.json");
    fs::write(
        &path,
        LEGACY_V1.replace("raytrace-project-v1", "raytrace-project-v9"),
    )
    .unwrap();
    assert!(design::open(&path, &[]).is_err());
    let record = dir.join("record.json");
    fs::write(&record, r#"{"format":"raytrace-run-v1"}"#).unwrap();
    let message = design::open(&record, &[]).unwrap_err().to_string();
    assert!(message.starts_with("計算記録は"), "{message}");
}

/// A record saved by the previous build: `calculation.project` holds the legacy project
/// and the compatibility report refers to components by index.
fn legacy_record(
    design: &Design,
    legacy_project: &str,
    dir: &std::path::Path,
) -> std::path::PathBuf {
    let current = dir.join("current.run.json");
    let request = Request::Trace(TraceRequest {
        model: ModelInput::Design(Box::new(design.clone())),
        catalogs: Vec::new(),
        rays: None,
        overrides: Overrides::default(),
        threads: Some(1),
        repeat: 1,
        warmup: false,
        terminals: false,
        paths: 0,
        preview_paths: 0,
        max_total_rays: None,
        export_zmx: None,
        output: Some(current.clone()),
        include_record: false,
    });
    engine::execute(request).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&current).unwrap()).unwrap();
    let calculation = value["calculation"].as_object_mut().unwrap();
    calculation.remove("design").unwrap();
    calculation.insert(
        "project".into(),
        serde_json::from_str(legacy_project).unwrap(),
    );
    for (index, component) in calculation["compatibility"]["components"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let component = component.as_object_mut().unwrap();
        component.remove("occurrence").unwrap();
        let legacy = if design.is_assembly() {
            index.into()
        } else {
            serde_json::Value::Null
        };
        component.insert("component".into(), legacy);
    }
    let legacy = dir.join("legacy.run.json");
    fs::write(&legacy, serde_json::to_vec(&value).unwrap()).unwrap();
    legacy
}

#[test]
fn records_saved_by_the_previous_build_load_and_replay() {
    for (name, legacy) in [("v1", LEGACY_V1), ("v2", LEGACY_V2)] {
        let dir = folder();
        let source = dir.join(format!("{name}.json"));
        fs::write(&source, legacy).unwrap();
        let design = design::open(&source, &[]).unwrap();
        let path = legacy_record(&design, legacy, &dir);
        let record = RunRecord::open(&path).unwrap();
        assert_eq!(
            serde_json::to_value(&record.calculation.design).unwrap(),
            serde_json::to_value(&design).unwrap(),
            "{name}"
        );
        let ids = |components: &[ComponentCompatibility]| -> Vec<Option<String>> {
            components
                .iter()
                .map(|c| c.occurrence.as_ref().map(|o| o.to_string()))
                .collect()
        };
        assert_eq!(
            ids(&record.calculation.compatibility.components),
            ids(&design.compatibility_report().components),
            "{name}"
        );
        // Replay reproduces the report of the original calculation.
        let replayed = dir.join("replayed.run.json");
        engine::execute(Request::Replay(ReplayRequest {
            record: RecordInput::Path(path),
            threads: Some(1),
            repeat: 1,
            warmup: false,
            terminals: None,
            paths: None,
            output: Some(replayed.clone()),
            include_record: false,
        }))
        .unwrap();
        let before = RunRecord::open(&dir.join("current.run.json")).unwrap();
        let after = RunRecord::open(&replayed).unwrap();
        assert_eq!(
            serde_json::to_value(&before.report).unwrap(),
            serde_json::to_value(&after.report).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn scan_records_of_the_previous_format_are_refused_with_a_reason() {
    let dir = folder();
    fs::write(
        dir.join("scan.json"),
        r#"{"format":"raytrace-scan-v1","software_version":"0.11.0","base":{},"plan":{},"cases":[]}"#,
    )
    .unwrap();
    let error = raytrace_rs::scan::ScanManifest::open(&dir).unwrap_err();
    assert!(error.to_string().contains("移行前の形式"), "{error}");
}
