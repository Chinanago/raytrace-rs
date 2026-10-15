use raytrace_rs::{
    math::V3,
    model::*,
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
        .join("target/exchange_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn write_utf16(path: &std::path::Path, text: &str) {
    let mut bytes = vec![255, 254];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(path, bytes).unwrap();
}
fn lens() -> String {
    "VERS 221221 0 0\nMODE SEQ\nNAME Test lens\nUNIT MM X W X CM MR CPMM\nGCAT TEST\nENPD 4\nWAVM 1 0.55 1\nPWAV 1\nNOTE 0 Preserve this note\nSURF 0\n TYPE STANDARD\n DISZ INFINITY\nSURF 1\n TYPE EVENASPH\n CURV 0.02\n CONI -0.2\n PARM 1 0\n PARM 2 1e-7\n DISZ 5\n GLAS TESTGLASS 0 0\n COAT ORIGINAL_COATING\n DIAM 10\nSURF 2\n TYPE STANDARD\n CURV 0\n DISZ 50\n COAT ORIGINAL_COATING\n DIAM 10\nSURF 3\n TYPE STANDARD\n DISZ 0\n DIAM 5\nMNUM 1 1\nOPER TEST_RETAIN\n".into()
}
fn glass() -> &'static str {
    "CC Test\nNM TESTGLASS 2 0 1.5 50 0 0\nCD 0.6961663 0.004679148 0.4079426 0.013512063 0.8974794 97.9340025\nLD 0.4 0.7\n"
}

#[test]
fn native_agf_dispersion_utf16_and_exact_unchanged_roundtrip() {
    let dir = folder();
    let input = dir.join("lens.zmx");
    write_utf16(&input, &lens());
    fs::write(dir.join("TEST.AGF"), glass()).unwrap();
    let p = Project::open(&input, &[]).unwrap();
    assert_eq!(p.scene.surfaces.len(), 3);
    assert_eq!(p.material_names[1], "TESTGLASS");
    assert!((p.scene.materials[1].index(0.465).unwrap() - 1.464483960997639).abs() < 1e-14);
    assert!(p.trace_scene().is_err());
    let mut allowed = p.clone();
    allowed.allow_uncoated = true;
    assert!(allowed.trace_scene().is_ok());
    let output = dir.join("copy.zmx");
    zemax::export(&p, &output).unwrap();
    assert_eq!(fs::read(input).unwrap(), fs::read(output).unwrap());
    let json = dir.join("project.json");
    p.save(&json).unwrap();
    let reopened = Project::open(&json, &[]).unwrap();
    assert_eq!(reopened.zemax.unwrap().text, lens());
}

#[test]
fn sequential_edits_preserve_coating_notes_and_merit_records() {
    let dir = folder();
    let input = dir.join("lens.zmx");
    write_utf16(&input, &lens());
    fs::write(dir.join("TEST.AGF"), glass()).unwrap();
    let mut p = Project::open(&input, &[]).unwrap();
    p.scene.surfaces[1].pose.translation_mm.0[2] = 6.0;
    let output = dir.join("edited.zmx");
    zemax::export(&p, &output).unwrap();
    let bytes = fs::read(&output).unwrap();
    let (text, _) = zemax::decode(&bytes);
    assert!(text.contains("COAT ORIGINAL_COATING"));
    assert!(text.contains("NOTE 0 Preserve this note"));
    assert!(text.contains("OPER TEST_RETAIN"));
    let again = Project::open(&output, &[]).unwrap();
    assert_eq!(again.scene.surfaces[1].pose.translation_mm.0[2], 6.0);
    let Shape::EvenAsphere { coefficients, .. } = &again.scene.surfaces[0].shape else {
        panic!()
    };
    assert_eq!(coefficients[0], 1e-7);
}

fn diode() -> String {
    let mut text=String::from("MODE NSC\nUNIT MM\nWAVM 1 0.55 1\nPWAV 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_SDIO 0 0 Source\n NSOP 0 0 0 0 0 0\n");
    for (k, v) in [
        (3, 1.0),
        (6, 0.0),
        (7, 10.0),
        (8, 1.0),
        (9, 20.0),
        (10, 1.0),
        (11, 1.0),
        (12, 1.0),
        (13, 0.0),
        (14, 0.0),
        (15, 0.01),
        (16, 0.005),
        (17, 1.0),
        (18, 0.01),
        (19, 0.005),
        (20, 1.0),
    ] {
        text.push_str(&format!(" NSOD {k} {v}\n"));
    }
    text.push_str(" NSOH NSC_DETC 0 0 Detector\n NSOP 0 0 10 0 0 0 ABSORB\n NSOD 1 100\n NSOD 2 100\n NSOD 3 16\n NSOD 4 16\n");
    text
}

#[test]
fn native_source_diode_angular_distribution_and_detector() {
    let dir = folder();
    let path = dir.join("source.zmx");
    fs::write(&path, diode()).unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    assert_eq!(p.scene.sources.len(), 1);
    assert_eq!(p.scene.surfaces.len(), 1);
    assert!(p.trace_scene().is_ok());
    p.scene.sources[0].count = 20000;
    let scene = p.scene.prepare().unwrap();
    let rays = scene.generate_rays().unwrap();
    let x: Vec<_> = rays
        .iter()
        .map(|r| (r.direction.0[0] / r.direction.0[2]).atan().to_degrees())
        .collect();
    let y: Vec<_> = rays
        .iter()
        .map(|r| (r.direction.0[1] / r.direction.0[2]).atan().to_degrees())
        .collect();
    let rms = |v: &Vec<f64>| (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt();
    assert!((rms(&x) - 5.0).abs() < 0.12);
    assert!((rms(&y) - 10.0).abs() < 0.25);
    assert!(rays
        .iter()
        .all(|r| r.position_mm.0[0].abs() <= 0.01 && r.position_mm.0[1].abs() <= 0.01));
    let report = trace(&scene, &rays, TraceOptions::default());
    assert!((report.budget.detected - 1.0).abs() < 1e-8);
}

#[test]
fn missing_spectrum_and_unknown_object_cannot_silently_trace() {
    let dir = folder();
    let path = dir.join("spectrum.zmx");
    let text = diode().replace(
        " NSOP 0 0 0 0 0 0",
        " NSCS 7 3 8 0 0 0 0.4 0.7 3 \"missing.SPCD\"\n NSOP 0 0 0 0 0 0",
    );
    fs::write(&path, text).unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_err());
    p.monochromatic_override_um = Some(0.465);
    assert!(p.trace_scene().is_ok());
    let text = diode().replace("NSC_SDIO", "NSC_UNKNOWN");
    fs::write(&path, &text).unwrap();
    let p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_err());
    let copy = dir.join("copy.zmx");
    zemax::export(&p, &copy).unwrap();
    assert_eq!(fs::read_to_string(copy).unwrap(), text);
}

#[test]
fn nsc_source_edits_and_dependency_copy_preserve_the_original() {
    let dir = folder();
    let path = dir.join("source.zmx");
    let text = format!(
        "{}\n NSOH NSC_IMPT 0 0 body.STEP\n NSOP 0 0 0 0 0 0\n",
        diode()
    );
    fs::write(&path, &text).unwrap();
    fs::write(dir.join("body.STEP"), "reference geometry bytes").unwrap();
    let mut project = Project::open(&path, &[]).unwrap();
    project.scene.sources[0].origin_mm.0[0] = 1.25;
    project.scene.sources[0].count = 1500;
    let destination = dir.join("out");
    fs::create_dir(&destination).unwrap();
    let output = destination.join("edited.zmx");
    zemax::export(&project, &output).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(
        fs::read_to_string(destination.join("body.STEP")).unwrap(),
        "reference geometry bytes"
    );
    let again = Project::open(&output, &[]).unwrap();
    assert_eq!(again.scene.sources[0].origin_mm.0[0], 1.25);
    assert_eq!(again.scene.sources[0].count, 1500);
    fs::write(destination.join("body.STEP"), "different existing file").unwrap();
    assert!(zemax::export(&project, &output).is_err());
}

#[test]
fn declared_catalog_selection_and_unsupported_records_are_explicit() {
    let dir = folder();
    let path = dir.join("lens.zmx");
    fs::write(&path, lens()).unwrap();
    fs::write(dir.join("TEST.AGF"), glass()).unwrap();
    fs::write(
        dir.join("WRONG.AGF"),
        glass().replace("0.6961663", "1.6961663"),
    )
    .unwrap();
    let project = Project::open(&path, &[]).unwrap();
    assert!((project.scene.materials[1].index(0.465).unwrap() - 1.464483960997639).abs() < 1e-14);
    fs::write(
        &path,
        lens().replace(
            " TYPE EVENASPH",
            " MYSTERY_OPTICAL_EFFECT 10\n TYPE EVENASPH",
        ),
    )
    .unwrap();
    let mut project = Project::open(&path, &[]).unwrap();
    project.allow_uncoated = true;
    assert!(project.trace_scene().is_err());
}

#[test]
fn newly_exported_lens_keeps_positions_apertures_and_dispersion() {
    let dir = folder();
    let scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    let project = Project::new(scene.clone());
    let output = dir.join("new_lens.zmx");
    zemax::export(&project, &output).unwrap();
    let again = Project::open(&output, &[]).unwrap();
    assert_eq!(again.scene.surfaces.len(), scene.surfaces.len() + 1);
    for (s, t) in scene
        .surfaces
        .iter()
        .zip(again.scene.surfaces.iter().skip(1))
    {
        assert_eq!(s.pose.translation_mm, t.pose.translation_mm);
        assert_eq!(
            serde_json::to_value(&s.aperture).unwrap(),
            serde_json::to_value(&t.aperture).unwrap()
        );
    }
    assert!(
        (again.scene.materials[1].index(0.45).unwrap() - scene.materials[1].index(0.45).unwrap())
            .abs()
            < 1e-14
    );
}

#[test]
fn mesh_bvh_and_absorbing_medium_match_planar_solution() {
    // Two large square mesh faces define the entry and exit of a slab.
    let vertices = vec![
        V3([-10.0, -10.0, 0.0]),
        V3([10.0, -10.0, 0.0]),
        V3([10.0, 10.0, 0.0]),
        V3([-10.0, 10.0, 0.0]),
        V3([-10.0, -10.0, 1.0]),
        V3([10.0, -10.0, 1.0]),
        V3([10.0, 10.0, 1.0]),
        V3([-10.0, 10.0, 1.0]),
    ];
    let shape = Shape::TriangleMesh {
        vertices,
        normals: vec![V3([0.0, 0.0, -1.0]); 4]
            .into_iter()
            .chain(vec![V3([0.0, 0.0, 1.0]); 4])
            .collect(),
        triangles: vec![[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7]],
    };
    let scene = Scene {
        name: "mesh".into(),
        reference_plane: "launch".into(),
        mode: Mode::NonSequential,
        materials: vec![
            Material::Constant {
                n: 1.0,
                absorption_per_mm: 0.0,
            },
            Material::WithAbsorption {
                dispersion: Box::new(Material::Constant {
                    n: 1.5,
                    absorption_per_mm: 0.0,
                }),
                samples: vec![[0.4, 0.1], [0.7, 0.1]],
            },
        ],
        settings: Settings::default(),
        sources: Vec::new(),
        surfaces: vec![
            Surface {
                coating: None,
                name: "slab".into(),
                pose: Default::default(),
                shape,
                aperture: Aperture::Rectangle {
                    half_width_mm: 10.0,
                    half_height_mm: 10.0,
                },
                interaction: Interaction::Dielectric {
                    minus_medium: 1,
                    plus_medium: 0,
                    reflectance: None,
                },
            },
            Surface {
                coating: None,
                name: "screen".into(),
                pose: raytrace_rs::math::Pose {
                    translation_mm: V3([0.0, 0.0, 2.0]),
                    ..Default::default()
                },
                shape: Shape::Plane,
                aperture: Aperture::Circle { radius_mm: 5.0 },
                interaction: Interaction::Detector { pixels: [4, 4] },
            },
        ],
    }
    .prepare()
    .unwrap();
    let ray = Ray {
        id: 0,
        position_mm: V3([0.1, 0.2, -1.0]),
        direction: V3([0.0, 0.0, 1.0]),
        weight: 1.0,
        wavelength_um: 0.55,
        medium: 0,
        optical_path_mm: 0.0,
    };
    let report = trace(
        &scene,
        &[ray],
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 0,
        },
    );
    let primary = 0.96f64.powi(2) * (-0.1f64).exp();
    let expected = primary / (1.0 - 0.04f64.powi(2) * (-0.2f64).exp());
    assert!((report.budget.detected - expected).abs() < 1e-7);
    assert_eq!(report.budget.medium_mismatch, 0.0);
    assert!((report.budget.accounted() - 1.0).abs() < 1e-12);
}
