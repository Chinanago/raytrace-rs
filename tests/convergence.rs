use raytrace_rs::{
    convergence::{self, ConvergencePlan},
    model::Scene,
    project::Project,
    record::RunRecord,
    trace::DetectorMap,
    validation,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/convergence_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn area_rebinning_conserves_power_on_noninteger_grid_ratios() {
    let map = DetectorMap {
        name: "uniform".into(),
        pixels: [5, 3],
        half_extent_mm: [3.0, 2.0],
        weights: vec![24.0 / 15.0; 15],
    };
    let coarse = convergence::rebin(&map, [2, 2]).unwrap();
    assert!(coarse.weights.iter().all(|w| (w - 6.0).abs() < 1e-12));
    assert!((coarse.weights.iter().sum::<f64>() - 24.0).abs() < 1e-12);
    let nonuniform = DetectorMap {
        weights: (1..=15).map(|x| x as f64).collect(),
        ..map.clone()
    };
    let coarse = convergence::rebin(&nonuniform, [2, 1]).unwrap();
    assert!((coarse.weights.iter().sum::<f64>() - 120.0).abs() < 1e-12);
    assert!(convergence::rebin(&map, [6, 2]).is_err());
}

#[test]
fn convergence_varies_one_axis_at_a_time_and_saves_replayable_conditions() {
    let dir = folder();
    let model = dir.join("input.json");
    let mut scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    for s in &mut scene.sources {
        s.count = 16;
    }
    Project::new(scene).save(&model).unwrap();
    let plan = ConvergencePlan {
        ray_counts: vec![16, 32],
        seeds: vec![91, 93],
        pixels: vec![[8, 8], [16, 16]],
        threads: 2,
        ..ConvergencePlan::default()
    };
    let report = convergence::run(&model, &plan, &dir.join("runs")).unwrap();
    assert_eq!(report.cases.len(), 8);
    assert!(report.cases.iter().all(|c| c.error.is_none()));
    assert_eq!(report.comparisons.len(), 12);
    for comparison in &report.comparisons {
        let a = &report.cases[comparison.case];
        let b = &report.cases[comparison.reference_case];
        let changed = usize::from(a.settings.rays_per_source != b.settings.rays_per_source)
            + usize::from(a.settings.seed != b.settings.seed)
            + usize::from(a.settings.pixels != b.settings.pixels)
            + usize::from(a.settings.cad_deflection_mm != b.settings.cad_deflection_mm);
        assert_eq!(changed, 1);
        assert_eq!(comparison.detectors[0].comparison_pixels, [8, 8]);
        assert!(comparison.detectors[0]
            .distribution_l1_percent_of_reference
            .is_finite());
    }
    let first = &report.cases[0];
    let run = RunRecord::open(&dir.join("runs").join(first.run_file.as_ref().unwrap())).unwrap();
    assert_eq!(run.calculation.scene.sources[0].count, 16);
    assert_eq!(run.calculation.scene.sources[0].seed, 91);
    assert_eq!(run.calculation.scene.sources[1].seed, 92);
    assert_eq!(run.calculation.rays().unwrap().len(), 32);
    let loaded: convergence::ConvergenceReport =
        serde_json::from_slice(&fs::read(dir.join("runs/convergence.json")).unwrap()).unwrap();
    assert_eq!(loaded.cases.len(), 8);
}

#[test]
fn invalid_axes_fail_before_tracing_and_case_errors_are_retained() {
    let dir = folder();
    let model = dir.join("model.json");
    let scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    Project::new(scene).save(&model).unwrap();
    let mut plan = ConvergencePlan {
        ray_counts: vec![0],
        save_runs: false,
        ..ConvergencePlan::default()
    };
    assert!(convergence::run(&model, &plan, &dir.join("invalid")).is_err());
    plan.ray_counts = vec![16];
    plan.cad_deflections_mm = vec![0.02, 0.01];
    assert!(convergence::run(&model, &plan, &dir.join("invalid")).is_err());
    plan.cad_deflections_mm = vec![0.02];
    plan.wavelength_um = Some(0.8);
    let report = convergence::run(&model, &plan, &dir.join("failed")).unwrap();
    assert!(report.cases.iter().all(|c| c.error.is_some()));
    assert!(report.comparisons.is_empty());
    assert!(dir.join("failed/convergence.json").is_file());
}

#[test]
fn owned_validation_fixtures_pass_local_checks_and_keep_external_status_pending() {
    let dir = folder();
    let cases = validation::cases();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let result = case.run(&dir.join(case.name)).unwrap();
        assert!(
            result.checks.iter().all(|c| c.passed),
            "{}: {:?}",
            case.name,
            result.checks
        );
        assert!(result.opticstudio_verification.starts_with("not_run"));
        assert!(dir.join(case.name).join("run.json").is_file());
    }
}
