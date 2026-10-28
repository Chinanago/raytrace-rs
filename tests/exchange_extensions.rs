use raytrace_rs::{
    coating,
    math::V3,
    model::{Material, Ray, Scene},
    project::Project,
    trace::{TraceOptions, trace},
    zemax,
};
use serde_json::json;
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
            "exchange_ext_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn model() -> Project {
    let mut scene:Scene=serde_json::from_value(json!({"name":"synthetic plate","reference_plane":"launch","mode":"sequential",
        "materials":[{"type":"constant","n":1},{"type":"with_absorption","dispersion":{"type":"catalog","formula":5,"coefficients":[1.48,0.01,0.0001],"valid_range_um":[0.4,0.7]},"samples":[[0.4,0.01],[0.7,0.03]]}],
        "sources":[{"count":128,"origin_mm":[0,0,-1],"direction":[0,0,1],"disk_radius_mm":1,"wavelength_um":0.55,"medium":0,"total_weight":1,"seed":42}],
        "surfaces":[{"name":"entry","shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":2},"interaction":{"type":"dielectric","minus_medium":0,"plus_medium":1}},
            {"name":"exit","pose":{"translation_mm":[0,0,5]},"shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":2},"interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0}},
            {"name":"screen","pose":{"translation_mm":[0,0,10]},"shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":2},"interaction":{"type":"detector","pixels":[8,8]}}]})).unwrap();
    scene.surfaces[0].coating=coating::parse_zemax("TABLE TEST\nANGL 0\nWAVE 0.4 0.1 0.2 0.6 0.7 17 29 31 43\nWAVE 0.7 0.1 0.2 0.6 0.7 17 29 31 43\nANGL 60\nWAVE 0.4 0.1 0.2 0.6 0.7 17 29 31 43\nWAVE 0.7 0.1 0.2 0.6 0.7 17 29 31 43\n","TEST").unwrap();
    Project::new(scene)
}
fn detected(p: &Project) -> (f64, f64) {
    let scene = p.trace_scene().unwrap().prepare().unwrap();
    let ray = Ray {
        id: 0,
        position_mm: V3([0., 0., -1.]),
        direction: V3([0., 0., 1.]),
        weight: 1.,
        wavelength_um: 0.55,
        medium: 0,
        optical_path_mm: 0.,
    };
    let r = trace(
        &scene,
        &[ray],
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 0,
        },
    );
    let t = r.terminals.iter().find(|t| t.reason == "detected").unwrap();
    (r.budget.detected, t.ray.optical_path_mm)
}
#[test]
fn coatings_phases_and_catalog_absorption_roundtrip_without_input_files() {
    let dir = folder();
    let p = model();
    let path = dir.join("native.zmx");
    zemax::export(&p, &path).unwrap();
    let native = Project::open(&path, &[]).unwrap();
    let a = detected(&p);
    let b = detected(&native);
    assert!((a.0 - b.0).abs() < 1e-12);
    assert!((a.1 - b.1).abs() < 1e-11);
    let dat = dir.join("native.raytrace-coatings.DAT");
    assert!(fs::read_to_string(&dat).unwrap().contains("17 29 31 43"));
    let saved = dir.join("project.json");
    native.save(&saved).unwrap();
    let original_bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::remove_file(&dat).unwrap();
    fs::remove_file(dir.join("native_RAYTRACE.AGF")).unwrap();
    let p = Project::open(&saved, &[]).unwrap();
    let out = dir.join("restored");
    fs::create_dir_all(&out).unwrap();
    zemax::export(&p, &out.join("native.zmx")).unwrap();
    assert_eq!(fs::read(out.join("native.zmx")).unwrap(), original_bytes);
    let restored = Project::open(&out.join("native.zmx"), &[]).unwrap();
    let c = detected(&restored);
    assert!((a.0 - c.0).abs() < 1e-12);
    assert!((a.1 - c.1).abs() < 1e-11);
    assert!(
        fs::read_to_string(out.join("native.raytrace-coatings.DAT"))
            .unwrap()
            .contains("17 29 31 43")
    );
}
#[test]
fn nsc_wave_edit_does_not_change_the_other_sources_system_spectrum() {
    let dir = folder();
    let path = dir.join("source.zmx");
    fs::write(&path,"MODE NSC\nUNIT MM\nWAVM 1 0.45 1\nWAVM 2 0.55 2\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_SDIO 0 0\n NSOP 0 0 -1 0 0 0\n NSOD 2 128\n NSOD 4 0\n NSOD 7 0\n NSOD 9 0\n NSOH NSC_SDIO 0 0\n NSOP 1 0 -1 0 0 0\n NSOD 2 128\n NSOD 4 2\n NSOD 7 0\n NSOD 9 0\n NSOH NSC_DETC 0 0\n NSOP 0 0 5 0 0 0 ABSORB\n NSOD 1 10\n NSOD 2 10\n NSOD 3 4\n NSOD 4 4\n").unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    p.scene.sources[1].wavelength_um = 0.65;
    let expected = p
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap();
    let output = dir.join("edited.zmx");
    zemax::export(&p, &output).unwrap();
    let reopened = Project::open(&output, &[]).unwrap();
    assert!(reopened.trace_scene().is_ok(), "{:?}", reopened.issues);
    assert_eq!(reopened.scene.sources[0].wavelengths(), vec![0.45, 0.55]);
    assert_eq!(reopened.scene.sources[1].wavelength_um, 0.65);
    let actual = reopened
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap();
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert_eq!(a.wavelength_um, b.wavelength_um);
        assert_eq!(a.weight, b.weight);
        assert!((a.position_mm - b.position_mm).norm() < 1e-12);
    }
    let saved = dir.join("edited.json");
    reopened.save(&saved).unwrap();
    fs::remove_file(dir.join("edited.raytrace-source-1.SPCD")).unwrap();
    let restore = dir.join("restored");
    fs::create_dir_all(&restore).unwrap();
    zemax::export(
        &Project::open(&saved, &[]).unwrap(),
        &restore.join("restored.zmx"),
    )
    .unwrap();
    assert!(
        Project::open(&restore.join("restored.zmx"), &[])
            .unwrap()
            .trace_scene()
            .is_ok()
    );
}
#[test]
fn unsupported_exports_and_conflicting_sidecars_preserve_existing_files() {
    let dir = folder();
    let path = dir.join("native.zmx");
    let agf = dir.join("native_RAYTRACE.AGF");
    fs::write(&agf, "keep user data").unwrap();
    assert!(zemax::export(&model(), &path).is_err());
    assert_eq!(fs::read_to_string(&agf).unwrap(), "keep user data");
    assert!(!path.exists());
    assert!(!dir.join("native.raytrace-coatings.DAT").exists());
    let mut p = model();
    p.scene.materials[1] = Material::Tabulated {
        samples: vec![[0.4, 1.5], [0.7, 1.4]],
        absorption_per_mm: 0.,
    };
    assert!(zemax::export(&p, &dir.join("unsupported.zmx")).is_err());
    assert!(!dir.join("unsupported.zmx").exists());
    let mut p = model();
    let c = p.scene.surfaces[0].coating.as_mut().unwrap();
    c.back = Some(coating::CoatingModel::Ideal {
        reflectance: 0.1,
        transmittance: 0.8,
        tir_reflectance: 1.,
    });
    assert!(zemax::export(&p, &dir.join("two_sides.zmx")).is_err());
    assert!(!dir.join("two_sides.zmx").exists());
}

#[test]
fn sequential_spectral_weights_and_monochromatic_edits_roundtrip() {
    let dir = folder();
    let mut p = model();
    p.scene.sources[0].spectrum = Some(raytrace_rs::spectrum::Spectrum {
        samples: vec![[0.45, 1.], [0.55, 2.], [0.65, 1.]],
    });
    let path = dir.join("spectral.zmx");
    zemax::export(&p, &path).unwrap();
    let mut reopened = Project::open(&path, &[]).unwrap();
    assert_eq!(
        reopened.scene.sources[0].wavelengths(),
        vec![0.45, 0.55, 0.65]
    );
    let weights = &reopened.scene.sources[0].spectrum.as_ref().unwrap().samples;
    assert!((weights[1][1] / weights[0][1] - 2.).abs() < 1e-12);
    reopened.scene.sources[0].spectrum = None;
    reopened.scene.sources[0].wavelength_um = 0.6;
    zemax::export(&reopened, &dir.join("mono.zmx")).unwrap();
    let single = Project::open(&dir.join("mono.zmx"), &[]).unwrap();
    assert_eq!(single.scene.sources[0].wavelengths(), vec![0.6]);
}
