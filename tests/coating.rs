use raytrace_rs::{
    coating::parse_zemax,
    math::V3,
    model::{Mode, Ray, Scene},
    project::Project,
    trace::{trace, TraceOptions},
};
use serde_json::{json, Value};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-12, "{a} != {b}");
}
fn coating() -> Value {
    json!({"name":"two_sides", "front":{"type":"ideal", "reflectance":0.1,"transmittance":0.7},
    "back":{"type":"ideal","reflectance":0.3,"transmittance":0.5,"tir_reflectance":0.65}})
}
fn scene(mode: Mode, c: Value) -> Scene {
    serde_json::from_value(json!({"name":"coating", "reference_plane":"launch","mode":mode,
        "materials":[{"type":"constant","n":1},{"type":"constant","n":1.5}],
        "surfaces":[{"name":"interface","coating":c,"shape":{"type":"plane"},
            "aperture":{"type":"circle","radius_mm":100},"interaction":{"type":"dielectric","minus_medium":0,"plus_medium":1}},
            {"name":"forward","pose":{"translation_mm":[0,0,2]},"shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":100},"interaction":{"type":"detector","pixels":[4,4]}},
            {"name":"backward","pose":{"translation_mm":[0,0,-2]},"shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":100},"interaction":{"type":"detector","pixels":[4,4]}}]})).unwrap()
}
fn ray() -> Ray {
    Ray {
        id: 0,
        position_mm: V3([0., 0., -1.]),
        direction: V3([0., 0., 1.]),
        weight: 1.,
        wavelength_um: 0.55,
        medium: 0,
        optical_path_mm: 0.,
    }
}

#[test]
fn coatings_conserve_power_for_both_directions_and_sequential_loss() {
    let prepared = scene(Mode::NonSequential, coating()).prepare().unwrap();
    let forward = trace(&prepared, &[ray()], TraceOptions::default());
    near(forward.detectors[0].weights.iter().sum(), 0.7);
    near(forward.detectors[1].weights.iter().sum(), 0.1);
    near(forward.budget.surface_absorbed, 0.2);
    near(forward.budget.accounted(), 1.);
    let mut r = ray();
    r.position_mm.0[2] = 1.;
    r.direction.0[2] = -1.;
    r.medium = 1;
    let backward = trace(&prepared, &[r], TraceOptions::default());
    near(backward.detectors[0].weights.iter().sum(), 0.3);
    near(backward.detectors[1].weights.iter().sum(), 0.5);
    near(backward.budget.accounted(), 1.);
    let sequential = trace(
        &scene(Mode::Sequential, coating()).prepare().unwrap(),
        &[ray()],
        TraceOptions::default(),
    );
    near(sequential.budget.detected, 0.7);
    near(sequential.budget.sequential_reflection_not_followed, 0.1);
    near(sequential.budget.surface_absorbed, 0.2);
    near(sequential.budget.accounted(), 1.);
}

#[test]
fn ideal_tir_uses_its_own_reflectance_and_absorption() {
    let prepared = scene(Mode::NonSequential, coating()).prepare().unwrap();
    let mut r = ray();
    r.medium = 1;
    r.position_mm.0[2] = 1.;
    let a = 50f64.to_radians();
    r.direction = V3([a.sin(), 0., -a.cos()]);
    let result = trace(&prepared, &[r], TraceOptions::default());
    assert_eq!(result.total_internal_reflections, 1);
    near(result.budget.detected, 0.65);
    near(result.budget.surface_absorbed, 0.35);
    near(result.budget.accounted(), 1.);
}

#[test]
fn table_interpolates_wavelength_and_cosine_in_lower_index_medium() {
    let text="TABLE TEST\nANGL 0\nWAVE 0.4 0.1 0.1 0.9 0.9\nWAVE 0.8 0.3 0.3 0.7 0.7\nANGL 60\nWAVE 0.4 0.3 0.3 0.7 0.7\nWAVE 0.8 0.5 0.5 0.5 0.5\n";
    let c = parse_zemax(text, "TEST").unwrap().unwrap();
    near(c.front.evaluate(0.6, 0.75, 1., 1.5, false)[0], 0.3);
    let cos_high = (1. - (1. - 0.75f64.powi(2)) / 1.5f64.powi(2)).sqrt();
    near(c.front.evaluate(0.6, cos_high, 1.5, 1., false)[0], 0.3);
    near(c.front.evaluate(0.6, 0.01, 1., 1.5, false)[0], 0.4);
    assert!(!c.covers(0.3));
    assert!(parse_zemax(&text.replace("0.1 0.1 0.9 0.9", "0.9 0.1 0.9 0.9"), "TEST").is_err());
    assert!(parse_zemax(&text.replace("ANGL 60", "ANGL 0"), "TEST").is_err());
}

#[test]
fn invalid_coating_energy_and_wavelength_are_rejected() {
    let mut c = coating();
    c["front"]["transmittance"] = json!(1.);
    assert!(scene(Mode::NonSequential, c).prepare().is_err());
    let c = parse_zemax(
        "TABLE TEST\nANGL 0\nWAVE 0.4 0 0 1 1\nWAVE 0.5 0 0 1 1\n",
        "TEST",
    )
    .unwrap()
    .unwrap();
    let prepared = scene(Mode::NonSequential, serde_json::to_value(c).unwrap())
        .prepare()
        .unwrap();
    assert!(ray().validate(&prepared).is_err());
    let c = parse_zemax("IDEAL SCALE 0.8 0.8 0.6", "SCALE")
        .unwrap()
        .unwrap();
    near(c.front.evaluate(0.55, 1., 1., 1.5, false)[0], 0.5);
    near(c.front.evaluate(0.55, 1., 1., 1.5, false)[1], 0.5);
}

#[test]
fn native_zmx_coating_is_saved_with_results_and_uncoated_override_is_explicit() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/coating_test")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lens.zmx");
    std::fs::write(&path,"MODE SEQ\nUNIT MM\nCOFN TEST.DAT\nWAVM 1 0.55 1\nSURF 0\n DISZ INFINITY\nSURF 1\n TYPE STANDARD\n DISZ 5\n COAT MYCOAT\nSURF 2\n TYPE STANDARD\n DISZ 0\n").unwrap();
    std::fs::write(dir.join("TEST.DAT"), "IDEAL MYCOAT 0.7 0.1\n").unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_ok());
    let report = p.compatibility_report();
    assert!(report.components[0]
        .entries
        .iter()
        .any(|e| e.code == "coating_model"));
    let json_path = dir.join("project.json");
    p.save(&json_path).unwrap();
    std::fs::remove_file(dir.join("TEST.DAT")).unwrap();
    let saved = Project::open(&json_path, &[]).unwrap();
    let trace_scene = saved.trace_scene().unwrap().prepare().unwrap();
    let traced = trace(&trace_scene, &[ray()], TraceOptions::default());
    near(traced.budget.detected, 0.7);
    near(traced.budget.surface_absorbed, 0.2);
    p.allow_uncoated = true;
    assert!(p.trace_scene().unwrap().surfaces[0].coating.is_none());
    assert!(Project::open(&path, &[]).unwrap().trace_scene().is_err());
}
