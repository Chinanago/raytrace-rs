//! Owned large-ray fixtures for before/after comparisons, including full reports.
//! Usage: throughput_check [rays] [threads] [repeats] [case|all] [explicit|streaming|both]
use raytrace_rs::{
    geometry::intersect_ordered,
    model::{Mode, Scene},
    spectrum::Spectrum,
    trace::{trace, trace_generated, TraceOptions},
};
use std::{hint::black_box, time::Instant};

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

fn fixture(name: &str, count: usize) -> Result<Scene, Box<dyn std::error::Error>> {
    let mut scene: Scene = serde_json::from_str(if name == "rod" {
        include_str!("integrator_rod.json")
    } else {
        include_str!("sequential_lens.json")
    })?;
    scene.sources.truncate(1);
    scene.sources[0].count = count;
    scene.sources[0].total_weight = 1.0;
    scene.sources[0].wavelength_um = 0.465;
    if name == "lens_200" {
        scene.sources[0].spectrum = Some(Spectrum {
            samples: (0..200)
                .map(|i| [0.45 + 0.2 * i as f64 / 199.0, 1.0 + (i % 7) as f64])
                .collect(),
        });
    } else if name == "asphere" {
        scene.surfaces[0].shape = raytrace_rs::model::Shape::EvenAsphere {
            radius_mm: 25.0,
            conic: -0.6,
            coefficients: vec![1e-6, 1e-10],
        };
        scene.surfaces[1].shape = raytrace_rs::model::Shape::Plane;
        scene.sources[0].disk_radius_mm = 8.0;
        scene.surfaces[2].aperture = raytrace_rs::model::Aperture::Rectangle {
            half_width_mm: 30.0,
            half_height_mm: 30.0,
        };
    } else if name == "array_256" {
        scene.mode = Mode::NonSequential;
        scene.sources[0].disk_radius_mm = 20.0;
        scene.materials.truncate(1);
        scene.surfaces.clear();
        for y in 0..16 {
            for x in 0..16 {
                scene.surfaces.push(serde_json::from_value(serde_json::json!({
                    "name": format!("tile_{x}_{y}"),
                    "pose": {"translation_mm": [(x as f64 - 7.5)*2.0, (y as f64 - 7.5)*2.0, 10.0]},
                    "shape": {"type": "plane"},
                    "aperture": {"type": "rectangle", "half_width_mm": 0.9, "half_height_mm": 0.9},
                    "interaction": {"type": "absorb"}
                }))?);
            }
        }
    } else if name != "lens" && name != "rod" {
        return Err(format!("unknown fixture {name}").into());
    }
    Ok(scene)
}

fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |a, b| {
        (a ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let count: usize = args
        .get(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(1_048_576);
    let threads: usize = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(4);
    let repeats: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(5);
    if count == 0 || threads == 0 || repeats == 0 {
        return Err("rays, threads and repeats must be positive".into());
    }
    let selected = args.get(4).map(String::as_str).unwrap_or("all");
    let mode = args.get(5).map(String::as_str).unwrap_or("both");
    if !["explicit", "streaming", "both"].contains(&mode) {
        return Err("mode must be explicit, streaming or both".into());
    }
    let names: Vec<_> = if selected == "all" {
        vec!["lens", "lens_200", "asphere", "array_256", "rod"]
    } else {
        vec![selected]
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    let mut cases = Vec::new();
    for name in names {
        let scene = fixture(name, count)?;
        let start = Instant::now();
        let prepared = scene.prepare()?;
        let prepare_seconds = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let rays = prepared.generate_rays()?;
        let generation_seconds = start.elapsed().as_secs_f64();
        let source_signature = fingerprint(&serde_json::to_vec(&rays)?);
        let expected = pool.install(|| trace(&prepared, &rays, TraceOptions::default()));
        let expected_json = serde_json::to_value(&expected)?;
        let mut explicit = Vec::new();
        let mut streaming = Vec::new();
        for _ in 0..repeats {
            if mode != "streaming" {
                let start = Instant::now();
                let report =
                    pool.install(|| trace(&prepared, black_box(&rays), TraceOptions::default()));
                explicit.push(start.elapsed().as_secs_f64());
                assert_eq!(serde_json::to_value(&report)?, expected_json);
                black_box(report);
            }
            if mode != "explicit" {
                // Warm generated tracing once; the explicit warmup above does not generate rays.
                if streaming.is_empty() {
                    let report =
                        pool.install(|| trace_generated(&prepared, TraceOptions::default()))?;
                    assert_eq!(serde_json::to_value(report)?, expected_json);
                }
                let start = Instant::now();
                let report =
                    pool.install(|| trace_generated(&prepared, TraceOptions::default()))?;
                streaming.push(start.elapsed().as_secs_f64());
                assert_eq!(serde_json::to_value(&report)?, expected_json);
                black_box(report);
            }
        }
        // Independent first-interface timing helps distinguish geometry from dispatch/report work.
        let mut first_surface = Vec::new();
        if name == "lens" || name == "lens_200" || name == "asphere" {
            for repeat in 0..=repeats {
                let start = Instant::now();
                let mut sum = 0.0;
                for ray in &rays {
                    if let Ok(hit) =
                        intersect_ordered(&prepared.surfaces[0], ray.position_mm, ray.direction)
                    {
                        sum += hit.distance_mm;
                    }
                }
                black_box(sum);
                if repeat != 0 {
                    first_surface.push(start.elapsed().as_secs_f64());
                }
            }
        }
        cases.push(serde_json::json!({
            "case": name, "rays": count, "surfaces": prepared.surfaces.len(), "threads": threads,
            "repeats": repeats, "prepare_seconds": prepare_seconds, "generation_seconds": generation_seconds,
            "explicit_seconds": (!explicit.is_empty()).then(|| median(&explicit)),
            "streaming_seconds": (!streaming.is_empty()).then(|| median(&streaming)),
            "first_surface_single_thread_seconds": (!first_surface.is_empty()).then(|| median(&first_surface)),
            "explicit_samples_seconds": explicit, "streaming_samples_seconds": streaming,
            "source_signature": source_signature,
            "report_signature": fingerprint(&serde_json::to_vec(&expected)?),
            "launch_array_bytes": rays.len()*std::mem::size_of::<raytrace_rs::model::Ray>(),
            "report": expected
        }));
        eprintln!("{name}: completed {count} rays, {threads} threads");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "timing": "Medians after one warmup. Explicit excludes preparation and launch generation; streaming includes launch generation. Both include report accumulation and exclude output/verification.",
            "cases": cases
        }))?
    );
    Ok(())
}
