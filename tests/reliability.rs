#[path = "support/cli.rs"]
mod cli_support;
use raytrace_rs::{
    assembly,
    compatibility::CompatibilityStatus as Status,
    math::{Pose, V3},
    model::*,
    project::Project,
    record::{LaunchInput, RunContext, RunRecord},
    trace::{TraceOptions, trace},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/reliability_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cli_replays_generated_and_csv_runs_without_the_original_inputs() {
    let dir = folder();
    let cli = |args: &[&str]| {
        std::process::Command::new(cli_support::executable())
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap()
    };
    for explicit in [false, true] {
        let mut scene: Scene =
            serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
        for source in &mut scene.sources {
            source.count = 16;
        }
        fs::write(dir.join("input.json"), serde_json::to_vec(&scene).unwrap()).unwrap();
        let mut args = vec![
            "input.json",
            "--threads",
            "2",
            "--terminals",
            "--paths",
            "8",
            "--output",
            "run.json",
        ];
        if explicit {
            fs::write(
                dir.join("rays.csv"),
                "id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium\n7,0.1,-0.2,0,0.00123,-0.00321,1,0.3,0.45,0\n9,-0.1,0.2,0,-0.001,0.002,1,0.7,0.635,0\n",
            ).unwrap();
            args.extend(["--rays-csv", "rays.csv"]);
        }
        let first = cli(&args);
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let original = fs::read(dir.join("run.json")).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(saved["format"], "raytrace-run-v1");
        assert_eq!(saved["schema_version"], 2);
        assert_eq!(
            saved["calculation"]["input"]["type"],
            if explicit { "explicit" } else { "generated" }
        );
        fs::remove_file(dir.join("input.json")).unwrap();
        if explicit {
            fs::remove_file(dir.join("rays.csv")).unwrap();
        }
        let replay = cli(&[
            "--replay",
            "run.json",
            "--threads",
            "1",
            "--output",
            "replay.json",
        ]);
        assert!(
            replay.status.success(),
            "{}",
            String::from_utf8_lossy(&replay.stderr)
        );
        let replayed: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("replay.json")).unwrap()).unwrap();
        assert_eq!(saved["report"], replayed["report"]);
        assert_eq!(
            saved["calculation"]["scene"],
            replayed["calculation"]["scene"]
        );
        assert!(
            !cli(&["--replay", "run.json", "--wavelength-um", "0.55"])
                .status
                .success()
        );
        assert!(
            !cli(&["--replay", "run.json", "--output", "run.json"])
                .status
                .success()
        );
        assert_eq!(original, fs::read(dir.join("run.json")).unwrap());
    }
}

fn ray(id: u64, x: f64) -> Ray {
    Ray {
        id,
        position_mm: V3([x, 0.0, -1.0]),
        direction: V3([0.0, 0.0, 1.0]),
        weight: 1.0,
        wavelength_um: 0.55,
        medium: 1,
        optical_path_mm: 0.0,
    }
}

#[test]
fn compatibility_keeps_component_conditions_even_when_flat_issues_are_resolved() {
    let mut p = assembly::demo_project().unwrap();
    let a = p.assembly.as_mut().unwrap();
    a.components[0]
        .project
        .issue("spectrum", "test spectrum", true);
    a.components[1]
        .project
        .issue("coating", "test coating", true);
    a.components[1].project.allow_uncoated = true;
    a.components[2].project.issue("object", "unsupported", true);
    a.components[2].enabled = false;
    p.monochromatic_override_um = Some(0.55);
    p.refresh_assembly().unwrap();
    assert!(p.trace_scene().is_ok());
    let report = p.compatibility_report();
    assert!(report.traceable);
    assert_eq!(report.components.len(), 4);
    assert!(
        report.components[0]
            .entries
            .iter()
            .any(|e| e.code == "spectrum" && e.status == Status::OverrideApplied)
    );
    assert!(
        report.components[1]
            .entries
            .iter()
            .any(|e| e.code == "coating" && e.status == Status::OverrideApplied)
    );
    assert!(
        report.components[2]
            .entries
            .iter()
            .any(|e| e.status == Status::Unsupported)
    );
    assert!(!report.components[2].enabled);
    assert!(
        report
            .components
            .iter()
            .flat_map(|c| &c.materials)
            .flat_map(|m| &m.samples)
            .all(|s| s.wavelength_um == 0.55 && s.refractive_index.is_some())
    );
    p.monochromatic_override_um = None;
    assert!(!p.compatibility_report().traceable);
    assert!(p.trace_scene().is_err());
    p.assembly.as_mut().unwrap().components[0]
        .project
        .monochromatic_override_um = Some(0.465);
    assert!(p.compatibility_report().traceable);
}

#[test]
fn unknown_records_and_invalid_or_missing_wavelengths_do_not_silently_trace() {
    let dir = folder();
    let path = dir.join("test.zmx");
    let text = "MODE SEQ\nUNIT MM\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n TYPE STANDARD\n DISZ INFINITY\nSURF 1\n TYPE STANDARD\n DISZ 2\n DIAM 3\nSURF 2\n TYPE STANDARD\n DISZ 0\n DIAM 3\nMNUM 1 1\nOPER RETAIN\n";
    fs::write(&path, text).unwrap();
    assert!(Project::open(&path, &[]).unwrap().trace_scene().is_ok());
    fs::write(&path, text.replace(" DISZ 2", "MYSTERY_EFFECT 2\n DISZ 2")).unwrap();
    assert!(Project::open(&path, &[]).unwrap().trace_scene().is_err());
    for bad in ["NaN", "-0.55", "not_a_number"] {
        fs::write(&path, text.replace("WAVM 1 0.55", &format!("WAVM 1 {bad}"))).unwrap();
        assert!(Project::open(&path, &[]).is_err());
    }
    fs::write(&path, text.replace("PWAV 1", "PWAV 2")).unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_err());
    p.monochromatic_override_um = Some(0.465);
    assert_eq!(p.trace_scene().unwrap().sources[0].wavelength_um, 0.465);
}

#[test]
fn unknown_nsc_attributes_and_coated_air_cad_are_explicit_conditions() {
    let dir = folder();
    let path = dir.join("attributes.zmx");
    let base = "MODE NSC\nUNIT MM\nWAVM 1 0.55 1\nSURF 1\n TYPE NONSEQCO\n NSOH NSC_IMPT 0 0 unused.step\n NSOP 0 0 0 0 0 0\n";
    fs::write(&path, format!("{base} NSCS 1 0 \"EXAMPLE_COATING\"\n")).unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_err());
    p.allow_uncoated = true;
    let report = p.compatibility_report();
    assert!(report.traceable);
    assert!(
        report.components[0]
            .entries
            .iter()
            .any(|e| e.status == Status::PreservedOnly)
    );
    fs::write(&path, format!("{base} NSCS 999 0 0\n")).unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    p.allow_uncoated = true;
    assert!(p.trace_scene().is_err());
    fs::write(&path, format!("{base} NSOD 1 invalid\n")).unwrap();
    assert!(Project::open(&path, &[]).is_err());
    let source = base.replace("NSC_IMPT 0 0 unused.step", "NSC_SDIO 0 0");
    fs::write(&path, format!("{source} NSCS 7 3 0\n")).unwrap();
    let mut p = Project::open(&path, &[]).unwrap();
    assert!(p.trace_scene().is_err());
    p.monochromatic_override_um = Some(0.465);
    assert_eq!(p.trace_scene().unwrap().sources[0].wavelength_um, 0.465);
}

#[test]
fn diagnostics_identify_medium_and_surface_and_remain_bounded() {
    let mut scene = assembly::empty_scene("diagnostic groups");
    scene.materials.push(Material::Constant {
        n: 1.5,
        absorption_per_mm: 0.0,
    });
    for i in 0..100 {
        scene.surfaces.push(Surface {
            coating: None,
            name: format!("plane_{i}"),
            pose: Pose {
                translation_mm: V3([i as f64 * 2.0, 0.0, 0.0]),
                ..Pose::default()
            },
            shape: Shape::Plane,
            aperture: Aperture::Rectangle {
                half_width_mm: 0.25,
                half_height_mm: 0.25,
            },
            interaction: Interaction::Dielectric {
                minus_medium: 0,
                plus_medium: 1,
                reflectance: None,
            },
        });
    }
    let prepared = scene.prepare().unwrap();
    let rays: Vec<_> = (0..100).map(|i| ray(i, i as f64 * 2.0)).collect();
    let report = trace(&prepared, &rays, TraceOptions::default());
    assert_eq!(report.budget.medium_mismatch, 100.0);
    assert!(report.terminals.is_empty());
    assert_eq!(report.diagnostics.len(), 64);
    assert_eq!(report.diagnostic_overflow.branch_count, 36);
    assert_eq!(
        report.diagnostics.iter().map(|d| d.weight).sum::<f64>()
            + report.diagnostic_overflow.weight,
        100.0
    );
    for d in &report.diagnostics {
        assert_eq!(d.actual_medium, 1);
        assert_eq!(d.expected_medium, Some(0));
        assert_eq!(d.surface, Some(d.example.ray.id as usize));
        assert_eq!(d.example.ray.position_mm.0[2], 0.0);
        let hit = d.intersection.as_ref().unwrap();
        assert_eq!(hit.origin_mm.0[2], -1.);
        assert_eq!(hit.distance_mm, 1.);
    }
}

#[test]
fn depth_limit_points_to_the_last_interacting_surface() {
    let mut spec: Scene =
        serde_json::from_str(include_str!("../examples/folded_mirror.json")).unwrap();
    spec.settings.max_depth = 1;
    let report = trace(
        &spec.prepare().unwrap(),
        &[Ray {
            medium: 0,
            ..ray(0, 0.0)
        }],
        TraceOptions::default(),
    );
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.reason == "interaction_limit")
        .unwrap();
    assert_eq!(d.surface, Some(0));
    assert!((d.weight - 0.98).abs() < 1e-14);
}

#[test]
fn generated_record_is_immutable_and_replays_every_float_after_reload() {
    let mut p = assembly::demo_project().unwrap();
    let mut scene = p.trace_scene().unwrap();
    for source in &mut scene.sources {
        source.count = 128;
        source.cone_half_angle_deg = 1.0;
    }
    let prepared = scene.clone().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let options = TraceOptions {
        keep_terminals: true,
        path_launch_count: 2,
    };
    let report = trace(&prepared, &rays, options);
    let context = RunContext::capture(&p, &scene, None, 2, options);
    p.assembly.as_mut().unwrap().components[2]
        .pose
        .translation_mm
        .0[2] += 7.0;
    p.refresh_assembly().unwrap();
    let path = folder().join("run.json");
    context.save(&report, 0.25, &path).unwrap();
    let saved = RunRecord::open(&path).unwrap();
    let again = saved.calculation.rays().unwrap();
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&rays).unwrap()
    );
    assert_eq!(
        saved
            .calculation
            .project
            .assembly
            .as_ref()
            .unwrap()
            .components[2]
            .pose
            .translation_mm
            .0[2],
        68.5
    );
    let replay = trace(
        &saved.calculation.scene.prepare().unwrap(),
        &again,
        saved.calculation.trace_options,
    );
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&report).unwrap()
    );
}

#[test]
fn explicit_record_keeps_launches_and_uses_their_actual_wavelengths() {
    let scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    let mut p = Project::new(scene.clone());
    p.issue("spectrum", "external spectrum", true);
    p.monochromatic_override_um = Some(0.465);
    let prepared = scene.clone().prepare().unwrap();
    let mut rays = vec![Ray {
        medium: 0,
        ..ray(7, 0.125)
    }];
    rays[0].direction = V3([0.00123, -0.00321, 1.0]);
    rays[0].validate(&prepared).unwrap();
    let report = trace(&prepared, &rays, TraceOptions::default());
    let mut context = RunContext::capture(&p, &scene, Some(&rays), 1, TraceOptions::default());
    assert_eq!(context.launch_wavelengths_um, vec![0.55]);
    assert!(
        context.compatibility.components[0]
            .entries
            .iter()
            .any(|e| e.code == "spectrum" && e.status == Status::PreservedOnly)
    );
    assert!(
        context.compatibility.components[0]
            .materials
            .iter()
            .flat_map(|m| &m.samples)
            .all(|s| s.wavelength_um == 0.55)
    );
    let path = folder().join("explicit.json");
    context.save(&report, 0.1, &path).unwrap();
    let saved = RunRecord::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(saved.calculation.rays().unwrap()).unwrap(),
        serde_json::to_value(&rays).unwrap()
    );
    context.input = LaunchInput::Explicit {
        rays: vec![rays[0], rays[0]],
    };
    assert!(context.rays().is_err());
}

#[test]
fn coincident_sequential_stop_and_interface_do_not_discard_the_ray() {
    let mut scene = assembly::empty_scene("coincident stop");
    scene.mode = Mode::Sequential;
    scene.materials.push(Material::Constant {
        n: 1.5,
        absorption_per_mm: 0.0,
    });
    for (name, z, interaction) in [
        (
            "stop",
            0.0,
            Interaction::Dielectric {
                minus_medium: 0,
                plus_medium: 0,
                reflectance: None,
            },
        ),
        (
            "front",
            0.0,
            Interaction::Dielectric {
                minus_medium: 0,
                plus_medium: 1,
                reflectance: None,
            },
        ),
        (
            "rear",
            2.0,
            Interaction::Dielectric {
                minus_medium: 1,
                plus_medium: 0,
                reflectance: None,
            },
        ),
        ("screen", 2.0, Interaction::Detector { pixels: [4, 4] }),
    ] {
        scene.surfaces.push(Surface {
            coating: None,
            name: name.into(),
            pose: Pose {
                translation_mm: V3([0.0, 0.0, z]),
                ..Pose::default()
            },
            shape: Shape::Plane,
            aperture: Aperture::Rectangle {
                half_width_mm: 2.0,
                half_height_mm: 2.0,
            },
            interaction,
        });
    }
    let report = trace(
        &scene.clone().prepare().unwrap(),
        &[Ray {
            medium: 0,
            ..ray(0, 0.1)
        }],
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 1,
        },
    );
    assert!((report.budget.detected - 0.96f64.powi(2)).abs() < 1e-14);
    assert_eq!(report.budget.geometric_miss, 0.0);
    assert!(
        (report
            .terminals
            .iter()
            .find(|t| t.reason == "detected")
            .unwrap()
            .ray
            .optical_path_mm
            - 4.0)
            .abs()
            < 1e-12
    );
    // The same configuration must survive the native ZMX roundtrip, including its start plane.
    let dir = folder();
    let path = dir.join("coincident.zmx");
    raytrace_rs::zemax::export(&Project::new(scene), &path).unwrap();
    let imported = Project::open(&path, &[])
        .unwrap()
        .trace_scene()
        .unwrap()
        .prepare()
        .unwrap();
    let again = trace(
        &imported,
        &[Ray {
            medium: 0,
            ..ray(0, 0.1)
        }],
        TraceOptions::default(),
    );
    assert_eq!(again.budget.geometric_miss, 0.0);
    assert!((again.budget.detected - report.budget.detected).abs() < 1e-14);
}
