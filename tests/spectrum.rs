use raytrace_rs::{
    assembly,
    model::{Material, Scene},
    project::Project,
    record::RunContext,
    spectrum::Spectrum,
    trace::{trace, TraceOptions},
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
        .join("target/spectrum_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-11, "{a} != {b}");
}
fn scene() -> Scene {
    serde_json::from_value(json!({"name":"spectral","reference_plane":"launch","mode":"sequential",
        "materials":[{"type":"constant","n":1}],
        "sources":[{"count":4096,"origin_mm":[0,0,-1],"direction":[0,0,1],"disk_radius_mm":1,"wavelength_um":0.55,"medium":0,"total_weight":4,"seed":42,
            "spectrum":{"samples":[[0.45,2],[0.55,1],[0.65,1]]}}],
        "surfaces":[{"name":"screen","shape":{"type":"plane"},"aperture":{"type":"rectangle","half_width_mm":2,"half_height_mm":2},"interaction":{"type":"detector","pixels":[8,8]}}]})).unwrap()
}

#[test]
fn spectral_sobol_matches_power_weights_without_position_correlation_and_replays() {
    let spec = scene();
    let prepared = spec.clone().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let mut counts = [0; 3];
    let mut inner = [0; 3];
    for r in &rays {
        let k = if r.wavelength_um == 0.45 {
            0
        } else if r.wavelength_um == 0.55 {
            1
        } else {
            2
        };
        counts[k] += 1;
        if r.position_mm.0[0].powi(2) + r.position_mm.0[1].powi(2) < 0.5 {
            inner[k] += 1;
        }
    }
    assert_eq!(counts, [2048, 1024, 1024]);
    assert_eq!(inner, [1024, 512, 512]);
    near(rays.iter().map(|r| r.weight).sum(), 4.);
    let mut prefix = spec.clone();
    prefix.sources[0].count = 1024;
    for (a, b) in rays
        .iter()
        .zip(prefix.prepare().unwrap().generate_rays().unwrap())
    {
        assert_eq!(a.position_mm, b.position_mm);
        assert_eq!(a.direction, b.direction);
        assert_eq!(a.wavelength_um, b.wavelength_um);
    }
    let p = Project::new(spec.clone());
    let context = RunContext::capture(&p, &spec, None, 1, TraceOptions::default());
    assert_eq!(context.launch_wavelengths_um, vec![0.45, 0.55, 0.65]);
    assert_eq!(
        serde_json::to_value(context.rays().unwrap()).unwrap(),
        serde_json::to_value(&rays).unwrap()
    );
    let result = trace(&prepared, &rays, TraceOptions::default());
    near(result.budget.detected, 4.);
    let mut overridden = p;
    overridden.monochromatic_override_um = Some(0.55);
    assert!(overridden.trace_scene().unwrap().sources[0]
        .spectrum
        .is_none());
}

#[test]
fn spectra_and_catalog_absorption_enforce_the_whole_supported_band() {
    for text in [
        "0.45 0\n0.55 0\n0.65 0",
        "0.65 1\n0.55 1\n0.45 1",
        "0.45 1\n0.55 -1\n0.65 1",
        "0.45 1\n0.55 1",
    ] {
        assert!(Spectrum::parse_spcd(text).is_err());
    }
    let mut spec = scene();
    spec.materials.push(Material::Tabulated {
        samples: vec![[0.5, 1.5], [0.7, 1.4]],
        absorption_per_mm: 0.,
    });
    assert!(spec.prepare().is_err());
    let spectrum =
        Spectrum::parse_spcd("# wave [um] relative power\n0.45 1\n0.55 2\n0.65 1\n").unwrap();
    assert_eq!(spectrum.samples.len(), 3);
    near(spectrum.sample(0.1), 0.45);
    near(spectrum.sample(0.5), 0.55);
    near(spectrum.sample(0.9), 0.65);
}

fn diode(color: &str) -> String {
    format!("MODE NSC\nUNIT MM\nWAVM 1 0.45 1\nWAVM 2 0.55 2\nWAVM 3 0.65 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_SDIO 0 0\n NSOP 0 0 -1 0 0 0\n NSOD 2 256\n NSOD 4 0\n NSCS 7 3 {color}\n NSOH NSC_DETC 0 0\n NSOP 0 0 2 0 0 0 ABSORB\n NSOD 1 10\n NSOD 2 10\n NSOD 3 4\n NSOD 4 4\n")
}

#[test]
fn nsc_spcd_is_embedded_and_inactive_spectrum_files_are_ignored() {
    let dir = folder();
    let path = dir.join("source.zmx");
    fs::write(dir.join("spectrum.SPCD"), "0.45 1\n0.55 2\n0.65 1\n").unwrap();
    fs::write(&path, diode("8 0 0 0 0.45 0.65 3 \"spectrum.SPCD\"")).unwrap();
    let p = Project::open(&path, &[]).unwrap();
    assert!(p.compatibility_report().traceable);
    let json_path = dir.join("saved.json");
    p.save(&json_path).unwrap();
    fs::remove_file(dir.join("spectrum.SPCD")).unwrap();
    let saved = Project::open(&json_path, &[])
        .unwrap()
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap();
    assert_eq!(saved.spec.sources[0].wavelengths(), vec![0.45, 0.55, 0.65]);
    assert_eq!(saved.generate_rays().unwrap().len(), 256);
    assert!(Project::open(&path, &[]).unwrap().trace_scene().is_err());
    fs::write(&path, diode("0 0 0 0 0.45 0.65 3 \"missing.SPCD\"")).unwrap();
    let system = Project::open(&path, &[]).unwrap();
    assert!(system.trace_scene().is_ok());
    assert_eq!(
        system.scene.sources[0].spectrum.as_ref().unwrap().samples,
        vec![[0.45, 1.], [0.55, 2.], [0.65, 1.]]
    );
    fs::write(&path, diode("4 0 0 0 0.45 0.65 3 \"missing.SPCD\"")).unwrap();
    let uniform = Project::open(&path, &[]).unwrap();
    assert!(uniform.trace_scene().is_ok());
    assert!(uniform.scene.sources[0]
        .spectrum
        .as_ref()
        .unwrap()
        .samples
        .iter()
        .all(|s| s[1] == 1.));
    fs::write(&path, diode("7 5000 0 0 0.45 0.65 3 \"missing.SPCD\"")).unwrap();
    let blackbody = Project::open(&path, &[]).unwrap();
    assert!(blackbody.trace_scene().is_ok());
    let s = &blackbody.scene.sources[0]
        .spectrum
        .as_ref()
        .unwrap()
        .samples;
    let expected = (0.65f64 / 0.45).powi(5) * (14387.768775039337f64 / (0.65 * 5000.)).exp_m1()
        / (14387.768775039337f64 / (0.45 * 5000.)).exp_m1();
    near(s[0][1] / s[2][1], expected);
}

#[test]
fn all_agf_formulas_and_internal_transmission_are_native() {
    let dir = folder();
    let path = dir.join("glass.zmx");
    let specs: [(usize, Vec<f64>, f64); 11] = [
        (
            3,
            vec![1.5, 0.01, 0., 0., 0., 0.],
            1.5 + 0.01 / (4. - 0.028),
        ),
        (
            4,
            vec![1.25, 0.2, 0.1, 0.1, 0.2],
            (2.25f64 + 0.8 / 3.99 + 0.1 / 3.96).sqrt(),
        ),
        (5, vec![1.5, 0.1, 0.01], 1.5 + 0.05 + 0.01 / 2f64.powf(3.5)),
        (
            6,
            vec![0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1],
            (1f64 + 3.2 / 3.9).sqrt(),
        ),
        (
            7,
            vec![2.25, 0.1, 0.2, 0.01],
            (2.25f64 + 0.1 / 3.8 - 0.04).sqrt(),
        ),
        (
            8,
            vec![2.25, 0.1, 0.2, 0.01],
            (2.25f64 + 0.4 / 3.8 - 0.04).sqrt(),
        ),
        (
            9,
            vec![2.25, 0.1, 0.2, 0.1, 0.2],
            (2.25f64 + 0.8 / 3.8).sqrt(),
        ),
        (
            10,
            vec![4., 0., 0., 0., 0., 0., 0., 1.],
            (4f64 + 1. / 4096.).sqrt(),
        ),
        (
            11,
            vec![0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1, 0.2, 0.1],
            (1f64 + 4. / 3.9).sqrt(),
        ),
        (12, vec![4., 0., 0., 0., 0., 0., 0., 1.], 68f64.sqrt()),
        (
            13,
            vec![4., 0., 0., 0., 0., 0., 0., 0., 1.],
            (4f64 + 1. / 4096.).sqrt(),
        ),
    ];
    for (formula, c, expected) in specs {
        fs::write(
            dir.join("TEST.AGF"),
            format!(
                "NM TEST {formula} 0 1.5 50 0 0\nCD {}\nLD 0.4 2.1\nIT 0.4 0.9 10\nIT 2.1 0.9 10\n",
                c.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        )
        .unwrap();
        fs::write(&path,"MODE SEQ\nUNIT MM\nGCAT TEST\nWAVM 1 2 1\nSURF 0\n DISZ INFINITY\nSURF 1\n TYPE STANDARD\n GLAS TEST 0 0\n DISZ 5\nSURF 2\n TYPE STANDARD\n DISZ 0\n").unwrap();
        let p = Project::open(&path, &[]).unwrap();
        assert!(p.trace_scene().is_ok(), "formula {formula}: {:?}", p.issues);
        near(p.scene.materials[1].index(2.).unwrap(), expected);
        near(
            (-p.scene.materials[1].absorption_at(2.).unwrap() * 20.).exp(),
            0.81,
        );
        assert!(p.scene.materials[1].index(0.3).is_none());
    }
    fs::write(
        dir.join("TEST.AGF"),
        "NM TEST 5 0 1.5 50 0 0\nCD 1.5 bad 0.1\nLD 0.4 2.1\n",
    )
    .unwrap();
    assert!(Project::open(&path, &[]).unwrap().trace_scene().is_err());
    fs::write(
        dir.join("TEST.AGF"),
        "NM TEST 5 0 1.5 50 0 0\nCD 1.5 0.01 0.1\nLD 0.4 2.1\nIT 0.4 1.2 10\n",
    )
    .unwrap();
    assert!(Project::open(&path, &[]).unwrap().trace_scene().is_err());
}

#[test]
fn generated_rods_retain_face_coatings_after_composition() {
    let mut p = assembly::demo_project().unwrap();
    let c = &mut p.assembly.as_mut().unwrap().components[2];
    c.rod.as_mut().unwrap().coatings.insert(
        "entrance".into(),
        raytrace_rs::coating::parse_zemax("IDEAL AR 0.99 0.005", "AR")
            .unwrap()
            .unwrap(),
    );
    p.refresh_assembly().unwrap();
    assert_eq!(
        p.scene
            .surfaces
            .iter()
            .filter(|s| s.coating.is_some())
            .count(),
        1
    );
    let original_origin = p.scene.sources[0].origin_mm;
    p.assembly.as_mut().unwrap().components[0]
        .project
        .scene
        .sources[0]
        .spectrum = Some(Spectrum {
        samples: vec![[0.45, 1.], [0.55, 2.], [0.65, 1.]],
    });
    p.refresh_assembly().unwrap();
    assert_eq!(
        p.compatibility_report().components[0].effective_wavelengths_um,
        vec![0.45, 0.55, 0.635, 0.65]
    );
    assert!(p.trace_scene().unwrap().sources[0].spectrum.is_some());
    assert_eq!(p.scene.sources[0].origin_mm, original_origin);
}
