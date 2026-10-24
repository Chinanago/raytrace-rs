#[path = "support/streaming.rs"]
mod fixtures;

use raytrace_rs::{
    model::{Material, Sampling},
    record::{RunContext, RunRecord, RUN_FORMAT},
    trace::{trace, trace_generated, trace_generated_controlled, TraceControl, TraceOptions},
};
use std::sync::atomic::Ordering;

// Hashes were recorded on Windows before streaming; platform libm may differ
// in the last ulp, so the portable checks are the exact report comparisons below.
#[cfg(target_os = "windows")]
#[test]
fn launch_json_matches_pre_streaming_hashes() {
    let expected = [
        0xcc018e73405ecad7,
        0xf696a1a1fa6c821f,
        0xabc8659ae8af6dd6,
        0x605df69957920987,
        0x605df69957920987,
        0xdb4835112089fd61,
        0x94394dd1c0049942,
        0x7ef44f242bb7b4b6,
        0x0f8f500227ddeab4,
    ];
    for ((name, mut scene), expected) in fixtures::scenes().into_iter().zip(expected) {
        for source in &mut scene.sources {
            source.count = 997;
        }
        let rays = scene.prepare().unwrap().generate_rays().unwrap();
        let json = serde_json::to_string(&rays).unwrap();
        let hash = json.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        assert_eq!(hash, expected, "{name}");
    }
}

#[test]
fn generated_reports_match_materialized_reports_exactly() {
    let options = TraceOptions {
        keep_terminals: true,
        path_launch_count: 3,
    };
    for (name, scene) in fixtures::scenes() {
        for count in [1, 30, 31, 32, 997, 4099] {
            let mut scene = scene.clone();
            for source in &mut scene.sources {
                source.count = count;
            }
            let scene = scene.prepare().unwrap();
            let rays = scene.generate_rays().unwrap();
            assert_eq!(
                serde_json::to_value(trace(&scene, &rays, options)).unwrap(),
                serde_json::to_value(trace_generated(&scene, options).unwrap()).unwrap(),
                "{name}, count={count}"
            );
        }
    }
}

#[test]
fn slices_match_across_sources_and_sampling_methods() {
    let fixtures = fixtures::scenes();
    for sampling in [Some(Sampling::Sobol), Some(Sampling::Pseudorandom), None] {
        let mut scene = fixtures[3].1.clone();
        scene.sources = [
            fixtures[5].1.sources[0].clone(),
            fixtures[7].1.sources[0].clone(),
            fixtures[6].1.sources[0].clone(),
        ]
        .into();
        for (i, source) in scene.sources.iter_mut().enumerate() {
            source.sampling = sampling.unwrap_or(if i == 1 {
                Sampling::Pseudorandom
            } else {
                Sampling::Sobol
            });
            source.count = [30, 32, 997][i];
            source.seed = 42 + i as u64;
        }
        let scene = scene.prepare().unwrap();
        let generator = scene.ray_generator().unwrap();
        assert!(!generator.is_empty());
        let n = generator.len();
        assert_eq!(n, 1059);
        let all = generator.rays(0..n).collect::<Result<Vec<_>, _>>().unwrap();
        for k in [0, 1, 29, 30, 31, 32, 61, 62, 63, 997, n] {
            let sliced = generator
                .rays(0..k)
                .chain(generator.rays(k..n))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                serde_json::to_value(&all).unwrap(),
                serde_json::to_value(sliced).unwrap(),
                "{sampling:?}, k={k}"
            );
        }
    }
}

#[test]
fn generation_errors_keep_the_first_launch_error() {
    let mut scene = fixtures::scenes().pop().unwrap().1;
    for source in &mut scene.sources {
        source.count = 128;
    }
    scene.materials[1] = Material::Sellmeier {
        b: [1., 0.5, 0.1],
        c_um2: [0.01, 0.02, 10.],
        valid_range_um: [0.4, 0.8],
        absorption_per_mm: 0.,
    };
    let scene = scene.prepare().unwrap();
    let error = scene.generate_rays().unwrap_err();
    // The on-axis source succeeds before the second source fails aiming.
    assert!(error.contains("光源 2"), "{error}");
    let generator = scene.ray_generator().unwrap();
    let first = generator
        .rays(0..generator.len())
        .position(|r| r.is_err())
        .unwrap();
    assert!(first >= 128);
    assert_eq!(
        trace_generated(&scene, TraceOptions::default()).unwrap_err(),
        error
    );
    let control = TraceControl::default();
    assert_eq!(
        trace_generated_controlled(&scene, TraceOptions::default(), &control).unwrap_err(),
        error
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn later_source_preparation_errors_do_not_hide_launch_errors() {
    let mut scene = fixtures::scenes().pop().unwrap().1;
    for source in &mut scene.sources {
        source.count = 128;
    }
    scene.materials[1] = Material::Sellmeier {
        b: [1., 0.5, 0.1],
        c_um2: [0.01, 0.02, 10.],
        valid_range_um: [0.4, 0.8],
        absorption_per_mm: 0.,
    };
    let expected = scene
        .clone()
        .prepare()
        .unwrap()
        .generate_rays()
        .unwrap_err();
    let mut later = scene.sources[1].clone();
    later.count = (1usize << 52) + 1;
    scene.sources.push(later);
    let scene = scene.prepare().unwrap();
    assert_eq!(scene.generate_rays().unwrap_err(), expected);
    assert_eq!(
        trace_generated(&scene, TraceOptions::default()).unwrap_err(),
        expected
    );
}

#[test]
fn cancellation_and_progress() {
    let mut scene = fixtures::scenes()[0].1.clone();
    scene.sources[0].count = 997;
    let scene = scene.prepare().unwrap();
    let control = TraceControl::default();
    control.cancel.store(true, Ordering::Relaxed);
    assert!(
        trace_generated_controlled(&scene, TraceOptions::default(), &control)
            .unwrap()
            .is_none()
    );
    assert_eq!(control.total.load(Ordering::Relaxed), 997);
    assert_eq!(control.completed.load(Ordering::Relaxed), 0);
    control.cancel.store(false, Ordering::Relaxed);
    assert!(
        trace_generated_controlled(&scene, TraceOptions::default(), &control)
            .unwrap()
            .is_some()
    );
    assert_eq!(control.completed.load(Ordering::Relaxed), 997);
    let mut empty = scene.spec.clone();
    empty.sources.clear();
    let empty = empty.prepare().unwrap();
    let generator = empty.ray_generator().unwrap();
    assert!(generator.is_empty());
    assert_eq!(generator.rays(0..0).count(), 0);
    assert_eq!(
        serde_json::to_value(trace(&empty, &[], TraceOptions::default())).unwrap(),
        serde_json::to_value(trace_generated(&empty, TraceOptions::default()).unwrap()).unwrap()
    );
}

#[test]
fn record_replay_streams_and_keeps_validation() {
    let project = raytrace_rs::assembly::demo_project().unwrap();
    let mut scene = project.trace_scene().unwrap();
    for source in &mut scene.sources {
        source.count = 32;
    }
    let options = TraceOptions {
        keep_terminals: true,
        path_launch_count: 3,
    };
    let prepared = scene.clone().prepare().unwrap();
    let rays = prepared.generate_rays().unwrap();
    let expected = trace(&prepared, &rays, options);
    for explicit in [None, Some(rays.as_slice())] {
        let calculation = RunContext::capture(&project, &scene, explicit, 2, options);
        let mut record = RunRecord {
            format: RUN_FORMAT.into(),
            calculation,
            trace_seconds: 0.,
            report: expected.clone(),
        };
        assert_eq!(
            serde_json::to_value(&expected).unwrap(),
            serde_json::to_value(record.replay().unwrap()).unwrap()
        );
        if let raytrace_rs::record::LaunchInput::Generated { count, .. } =
            &mut record.calculation.input
        {
            *count += 1;
            assert!(record.replay().is_err());
            if let raytrace_rs::record::LaunchInput::Generated {
                count,
                sampling_revision,
            } = &mut record.calculation.input
            {
                *count -= 1;
                *sampling_revision = u32::MAX;
            }
            assert!(record.replay().is_err());
        }
    }
}
