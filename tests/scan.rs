#[path = "support/cli.rs"]
mod cli_support;
use raytrace_rs::{
    assembly,
    model::Scene,
    project::Project,
    record::RunRecord,
    scan::{Parameter, ScanManifest, ScanPlan, linear_values},
    trace::{TraceControl, TraceOptions, trace, trace_controlled},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
#[test]
fn two_axes_and_reversed_dependency_order_preserve_gaps_and_resume() {
    use raytrace_rs::scan::{ParameterLink, ScanAxis};
    let dir = folder();
    let base = lens();
    let x = Parameter::SurfacePosition {
        surface: 0,
        axis: 2,
    };
    let y = Parameter::SurfacePosition {
        surface: 1,
        axis: 2,
    };
    let z = Parameter::SurfacePosition {
        surface: 2,
        axis: 2,
    };
    let plan = ScanPlan {
        parameter: x.clone(),
        values: vec![0., 1.],
        secondary: Some(ScanAxis {
            parameter: Parameter::Wavelength,
            values: vec![0.45, 0.635],
        }),
        links: vec![
            ParameterLink {
                source: y.clone(),
                target: z.clone(),
                scale: 1.,
                offset: 60.,
            },
            ParameterLink {
                source: x.clone(),
                target: y.clone(),
                scale: 1.,
                offset: 5.,
            },
        ],
        rays_per_source: Some(256),
        threads: 2,
    };
    let original = serde_json::to_value(&base).unwrap();
    let mut m = ScanManifest::new(&base, plan).unwrap();
    assert_eq!(m.cases.len(), 4);
    let control = TraceControl::default();
    m.execute(&dir, &control, |m| {
        if m.cases[1].state == "completed" {
            control.cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    let bytes = fs::read(dir.join("case_0000.raytrace-run.json")).unwrap();
    let mut resumed = ScanManifest::open(&dir).unwrap();
    resumed
        .execute(&dir, &TraceControl::default(), |_| {})
        .unwrap();
    assert!(resumed.cases.iter().all(|c| c.state == "completed"));
    assert_eq!(
        bytes,
        fs::read(dir.join("case_0000.raytrace-run.json")).unwrap()
    );
    let mut launch_signatures = Vec::new();
    for (i, c) in resumed.cases.iter().enumerate() {
        let r = RunRecord::open(&dir.join(c.run_file.as_ref().unwrap())).unwrap();
        let p = &r.calculation.project;
        assert_eq!(x.value(p).unwrap(), (i / 2) as f64);
        assert_eq!(y.value(p).unwrap() - x.value(p).unwrap(), 5.);
        assert_eq!(z.value(p).unwrap() - y.value(p).unwrap(), 60.);
        let prepared = r.calculation.scene.clone().prepare().unwrap();
        let rays = prepared.generate_rays().unwrap();
        launch_signatures.push(
            rays.iter()
                .map(|r| (r.position_mm, r.direction))
                .collect::<Vec<_>>(),
        );
    }
    assert!(launch_signatures.windows(2).all(|v| v[0] == v[1]));
    assert_eq!(original, serde_json::to_value(&base).unwrap());
    resumed.cases[0].secondary_value = Some(0.55);
    resumed.save(&dir).unwrap();
    assert!(ScanManifest::open(&dir).is_err());
}
#[test]
fn cyclic_links_duplicate_targets_axis_conflicts_and_case_explosion_are_rejected() {
    use raytrace_rs::scan::{ParameterLink, ScanAxis};
    let base = lens();
    let a = Parameter::SurfacePosition {
        surface: 0,
        axis: 0,
    };
    let b = Parameter::SurfacePosition {
        surface: 1,
        axis: 0,
    };
    let link = ParameterLink {
        source: a.clone(),
        target: b.clone(),
        scale: 1.,
        offset: 2.,
    };
    for links in [
        vec![
            link.clone(),
            ParameterLink {
                source: b.clone(),
                target: a.clone(),
                scale: 1.,
                offset: 0.,
            },
        ],
        vec![link.clone(), link.clone()],
    ] {
        assert!(
            ScanManifest::new(
                &base,
                ScanPlan {
                    links,
                    ..ScanPlan::default()
                }
            )
            .is_err()
        );
    }
    assert!(
        ScanManifest::new(
            &base,
            ScanPlan {
                parameter: b.clone(),
                links: vec![link],
                ..ScanPlan::default()
            }
        )
        .is_err()
    );
    assert!(
        ScanManifest::new(
            &base,
            ScanPlan {
                secondary: Some(ScanAxis {
                    parameter: a,
                    values: (0..100).map(|x| x as f64).collect()
                }),
                ..ScanPlan::default()
            }
        )
        .is_err()
    );
    assert!(
        ScanManifest::new(
            &base,
            ScanPlan {
                secondary: Some(ScanAxis {
                    parameter: Parameter::Wavelength,
                    values: vec![0.55]
                }),
                ..ScanPlan::default()
            }
        )
        .is_err()
    );
}
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/scan_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn lens() -> Project {
    let mut s: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    for source in &mut s.sources {
        source.count = 32;
    }
    Project::new(s)
}
#[test]
fn controlled_trace_preserves_the_complete_report_and_cancellation_keeps_it_unpublished() {
    let p = lens().scene.prepare().unwrap();
    let rays = p.generate_rays().unwrap();
    let options = TraceOptions {
        keep_terminals: true,
        path_launch_count: 8,
    };
    let expected = trace(&p, &rays, options);
    let control = TraceControl::default();
    let actual = trace_controlled(&p, &rays, options, &control).unwrap();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    assert_eq!(control.completed.load(Ordering::Relaxed), rays.len());
    control.cancel.store(true, Ordering::Relaxed);
    assert!(trace_controlled(&p, &rays, options, &control).is_none());
    assert_eq!(control.completed.load(Ordering::Relaxed), 0);
}
#[test]
fn cancelled_scan_resumes_from_embedded_inputs_without_rewriting_completed_cases() {
    let dir = folder();
    let mut base = lens();
    let plan = ScanPlan {
        parameter: Parameter::SurfacePosition {
            surface: 2,
            axis: 2,
        },
        values: vec![67.0, 68.5, 70.0],
        threads: 2,
        rays_per_source: None,
        secondary: None,
        links: Vec::new(),
    };
    let original = serde_json::to_value(&base).unwrap();
    let mut m = ScanManifest::new(&base, plan).unwrap();
    let control = TraceControl::default();
    m.execute(&dir, &control, |m| {
        if m.cases[0].state == "completed" {
            control.cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert_eq!(
        m.cases.iter().map(|c| c.state.as_str()).collect::<Vec<_>>(),
        vec!["completed", "pending", "pending"]
    );
    let first = fs::read(dir.join("case_0000.raytrace-run.json")).unwrap();
    base.scene.surfaces[2].pose.translation_mm.0[2] = 300.0;
    let mut resumed = ScanManifest::open(&dir).unwrap();
    control.cancel.store(false, Ordering::Relaxed);
    resumed.execute(&dir, &control, |_| {}).unwrap();
    assert!(resumed.cases.iter().all(|c| c.state == "completed"));
    assert_eq!(
        first,
        fs::read(dir.join("case_0000.raytrace-run.json")).unwrap()
    );
    assert_eq!(serde_json::to_value(&resumed.base).unwrap(), original);
    let mut launches = None;
    for (i, value) in [67.0, 68.5, 70.0].into_iter().enumerate() {
        let r = RunRecord::open(&dir.join(format!("case_{i:04}.raytrace-run.json"))).unwrap();
        assert_eq!(
            r.calculation.scene.surfaces[2].pose.translation_mm.0[2],
            value
        );
        let rays = serde_json::to_value(r.calculation.rays().unwrap()).unwrap();
        if let Some(expected) = &launches {
            assert_eq!(expected, &rays);
        } else {
            launches = Some(rays);
        }
    }
    fs::copy(
        dir.join("case_0001.raytrace-run.json"),
        dir.join("case_0000.raytrace-run.json"),
    )
    .unwrap();
    assert!(ScanManifest::open(&dir).is_err());
}
#[test]
fn parameter_units_limits_and_failed_cases_are_retained() {
    let mut assembly = assembly::demo_project().unwrap();
    Parameter::RodSize {
        component: 2,
        axis: 2,
    }
    .apply(&mut assembly, 40.0)
    .unwrap();
    Parameter::ComponentRotation {
        component: 2,
        axis: 1,
    }
    .apply(&mut assembly, 3.0)
    .unwrap();
    assert_eq!(
        assembly.assembly.as_ref().unwrap().components[2]
            .rod
            .as_ref()
            .unwrap()
            .length_mm,
        40.0
    );
    assert!(
        Parameter::RodSize {
            component: 2,
            axis: 0
        }
        .apply(&mut assembly, -1.0)
        .is_err()
    );
    assert!(
        Parameter::ComponentPosition {
            component: 2,
            axis: 3
        }
        .apply(&mut assembly, 1.0)
        .is_err()
    );
    assert!(
        Parameter::SurfacePosition {
            surface: 0,
            axis: 2
        }
        .apply(&mut assembly, 1.0)
        .is_err()
    );
    assert_eq!(linear_values(0.4, 0.7, 4).unwrap().last(), Some(&0.7));
    assert!(linear_values(0.4, 0.7, 1).is_err());
    let dir = folder();
    let mut m = ScanManifest::new(
        &lens(),
        ScanPlan {
            values: vec![0.45, 0.8],
            ..ScanPlan::default()
        },
    )
    .unwrap();
    m.execute(&dir, &TraceControl::default(), |_| {}).unwrap();
    assert_eq!(m.cases[0].state, "completed");
    assert_eq!(m.cases[1].state, "failed");
    assert!(m.cases[1].error.is_some());
    assert!(m.cases[1].detected_percent.is_none());
    assert_eq!(ScanManifest::open(&dir).unwrap().cases[1].state, "failed");
}
#[test]
fn cli_scan_can_be_reopened_and_protects_existing_output() {
    let dir = folder();
    lens().save(&dir.join("model.json")).unwrap();
    fs::write(
        dir.join("plan.json"),
        serde_json::to_vec(&ScanPlan {
            values: vec![0.45, 0.635],
            threads: 1,
            ..ScanPlan::default()
        })
        .unwrap(),
    )
    .unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(cli_support::executable())
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap()
    };
    let args = [
        "--scan",
        "model.json",
        "--plan",
        "plan.json",
        "--output",
        "study",
    ];
    let result = run(&args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = fs::read(dir.join("study/scan.json")).unwrap();
    assert!(!run(&args).status.success());
    assert_eq!(bytes, fs::read(dir.join("study/scan.json")).unwrap());
    fs::remove_file(dir.join("model.json")).unwrap();
    assert!(run(&["--resume-scan", "study"]).status.success());
}
