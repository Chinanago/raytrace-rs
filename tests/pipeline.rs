//! The engine, scans and convergence studies share the stages of `raytrace_rs::pipeline`.
use raytrace_rs::{
    command::call_once,
    commands,
    convergence::{self, ConvergencePlan},
    design::{self, Design},
    engine::{ModelInput, Overrides, TraceRequest},
    model::{Interaction, Scene},
    pipeline::{self, LaunchRequest, LimitViolation, Limits, SceneOverrides, Schedule},
    record::RunRecord,
    scan::{Parameter, ScanManifest, ScanPlan},
    trace::TraceControl,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/pipeline_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn lens(count: usize) -> Scene {
    let mut scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    for source in &mut scene.sources {
        source.count = count;
    }
    scene
}
/// The report the engine `trace` request produces for `design`, as JSON.
fn engine_report(design: Design, rays_per_source: usize) -> serde_json::Value {
    let request = TraceRequest {
        model: Some(ModelInput::Design(Box::new(design))),
        catalogs: Vec::new(),
        rays: None,
        overrides: Overrides {
            rays_per_source: Some(rays_per_source),
            ..Overrides::default()
        },
        threads: Some(2),
        repeat: 1,
        warmup: false,
        terminals: false,
        paths: 0,
        preview_paths: 0,
        max_total_rays: None,
        export_zmx: None,
        output: None,
        include_record: true,
    };
    let result = call_once(commands::ANALYSIS_TRACE, request).unwrap();
    serde_json::to_value(&result.record.unwrap().report).unwrap()
}
fn saved_report(path: &std::path::Path) -> (Design, serde_json::Value) {
    let record = RunRecord::open(path).unwrap();
    (
        record.calculation.design.clone(),
        serde_json::to_value(&record.report).unwrap(),
    )
}

#[test]
fn scan_case_and_engine_trace_save_identical_reports() {
    let dir = folder();
    let plan = ScanPlan {
        parameter: Parameter::Wavelength,
        values: vec![0.55],
        secondary: None,
        links: Vec::new(),
        rays_per_source: Some(32),
        threads: 2,
    };
    let mut manifest = ScanManifest::new(&Design::from_scene(lens(8)), plan).unwrap();
    manifest
        .execute(&dir, &TraceControl::default(), |_| {})
        .unwrap();
    let (design, scanned) = saved_report(&dir.join("case_0000.raytrace-run.json"));
    assert_eq!(scanned, engine_report(design, 32));
}

#[test]
fn convergence_case_and_engine_trace_save_identical_reports() {
    let dir = folder();
    let model = dir.join("input.json");
    design::save(&Design::from_scene(lens(8)), &model).unwrap();
    let plan = ConvergencePlan {
        ray_counts: vec![24],
        seeds: Vec::new(),
        pixels: Vec::new(),
        threads: 2,
        ..ConvergencePlan::default()
    };
    let report = convergence::run(&model, &plan, &dir.join("runs")).unwrap();
    let file = report.cases[0].run_file.as_ref().unwrap();
    let (design, converged) = saved_report(&dir.join("runs").join(file));
    assert_eq!(converged, engine_report(design, 24));
}

#[test]
fn stages_configure_scenes_and_report_limits() {
    let mut scene = lens(10);
    pipeline::configure_scene(
        &mut scene,
        &SceneOverrides {
            rays_per_source: Some(50),
            max_rays_per_source: Some(20),
            seed: Some(7),
            detector_pixels: Some([3, 5]),
            ..SceneOverrides::default()
        },
    );
    for (i, source) in scene.sources.iter().enumerate() {
        assert_eq!(source.count, 20);
        assert_eq!(source.seed, 7 + i as u64);
    }
    for surface in &scene.surfaces {
        if let Interaction::Detector { pixels } = surface.interaction {
            assert_eq!(pixels, [3, 5]);
        }
    }
    let total = pipeline::total_rays(&scene);
    let limits = |max_rays, max_screen_pixels| Limits {
        max_rays,
        max_screen_pixels,
    };
    assert_eq!(
        pipeline::check_limits(&scene, limits(total as usize, Some(15))),
        Ok(())
    );
    assert_eq!(
        pipeline::check_limits(&scene, limits(total as usize - 1, None)),
        Err(LimitViolation::Rays {
            total,
            max: total as usize - 1
        })
    );
    assert_eq!(
        pipeline::check_limits(&scene, limits(total as usize, Some(14))),
        Err(LimitViolation::ScreenPixels { max: 14 })
    );
    assert!(Schedule::new(false, 0).is_err());
}

#[test]
fn cancelled_trace_is_a_distinct_outcome() {
    let scene = lens(16).prepare().unwrap();
    let launch = pipeline::resolve_launch(&scene, LaunchRequest::Generated).unwrap();
    assert_eq!(launch.count(), 32);
    let control = TraceControl::default();
    control.cancel.store(true, Ordering::Relaxed);
    let outcome = pipeline::trace_once(&scene, &launch, Default::default(), &control).unwrap();
    assert!(matches!(outcome, pipeline::Completion::Cancelled));
    assert_eq!(outcome.into_result().unwrap_err().code(), "cancelled");
}
