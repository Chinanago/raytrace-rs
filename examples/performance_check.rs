//! Repeatable stage timings on owned analytic fixtures; no OpticStudio API.
use raytrace_rs::{
    model::Scene,
    spectrum::Spectrum,
    trace::{trace, TraceOptions},
};
use std::{hint::black_box, time::Instant};
fn median(mut a: Vec<f64>) -> f64 {
    a.sort_by(f64::total_cmp);
    a[a.len() / 2]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::args()
        .nth(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(131072);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build()?;
    let mut results = Vec::new();
    for waves in [1, 200] {
        let mut scene: Scene = serde_json::from_str(include_str!("sequential_lens.json"))?;
        scene.sources.truncate(1);
        scene.sources[0].count = count;
        if waves > 1 {
            scene.sources[0].spectrum = Some(Spectrum {
                samples: (0..waves)
                    .map(|i| {
                        [
                            0.45 + 0.2 * i as f64 / (waves - 1) as f64,
                            1. + (i % 7) as f64,
                        ]
                    })
                    .collect(),
            });
        }
        let mut prepare_times = Vec::new();
        let mut generation_times = Vec::new();
        let mut trace_times = Vec::new();
        let mut signature = 0u64;
        let mut report_json = serde_json::Value::Null;
        for run in 0..6 {
            let start = Instant::now();
            let prepared = black_box(scene.clone()).prepare()?;
            let prep = start.elapsed().as_secs_f64();
            let start = Instant::now();
            let rays = prepared.generate_rays()?;
            let generation = start.elapsed().as_secs_f64();
            let start = Instant::now();
            let report =
                pool.install(|| trace(&prepared, black_box(&rays), TraceOptions::default()));
            let seconds = start.elapsed().as_secs_f64();
            if run > 0 {
                prepare_times.push(prep);
                generation_times.push(generation);
                trace_times.push(seconds);
            }
            signature = 0xcbf29ce484222325;
            for r in &rays {
                for f in r
                    .position_mm
                    .0
                    .into_iter()
                    .chain(r.direction.0)
                    .chain([r.wavelength_um, r.weight])
                {
                    signature = (signature ^ f.to_bits()).wrapping_mul(0x100000001b3);
                }
            }
            report_json = serde_json::to_value(&report)?;
        }
        results.push(serde_json::json!({"wavelengths":waves,"rays":count,"threads":1,"repeats":5,"prepare_seconds":median(prepare_times),"generation_seconds":median(generation_times),"trace_seconds":median(trace_times),"launch_signature":format!("{signature:016x}"),"ray_storage_bytes":count*std::mem::size_of::<raytrace_rs::model::Ray>(),"report":report_json}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"software_version":env!("CARGO_PKG_VERSION"),"timing":"Median of five runs after one warmup; preparation, ray generation, and tracing measured separately; output excluded.","cases":results})
        )?
    );
    Ok(())
}
