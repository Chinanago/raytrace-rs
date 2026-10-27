//! Re-evaluate frozen launch rays while changing only the interaction allowance.
use raytrace_rs::{
    geometry,
    record::{RunContext, RunRecord},
    trace::{TraceOptions, trace},
};
use std::{fs, path::PathBuf, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("Expected calculation record and output directory")?,
    );
    let out = PathBuf::from(args.next().ok_or("Expected output directory")?);
    if out.exists() {
        return Err("Choose a new output directory".into());
    }
    let record = RunRecord::open(&input)?;
    let rays = record.calculation.rays()?;
    let limits = args
        .map(|s| s.parse::<usize>())
        .collect::<Result<Vec<_>, _>>()?;
    let limits = if limits.is_empty() {
        vec![4096, 16384, 65536]
    } else {
        limits
    };
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    fs::create_dir_all(&out)?;
    let mut results = Vec::new();
    for events in limits {
        let mut scene = record.calculation.scene.clone();
        scene.settings.max_events_per_launch = events;
        let prepared = scene.clone().prepare()?;
        let start = Instant::now();
        let report = pool.install(|| trace(&prepared, &rays, TraceOptions::default()));
        let seconds = start.elapsed().as_secs_f64();
        RunContext::capture(
            &record.calculation.project,
            &scene,
            Some(&rays),
            4,
            TraceOptions::default(),
        )
        .save(&report, seconds, &out.join(format!("events_{events}.json")))?;
        let mut issues = Vec::new();
        for d in report
            .diagnostics
            .iter()
            .filter(|d| d.reason == "medium_mismatch" || d.reason == "numerical_failure")
        {
            let r = &d.example.ray;
            let surface = d.surface.map(|i| &prepared.surfaces[i]);
            let local = surface.map(|s| s.frame.local_point(r.position_mm));
            let residual = surface.zip(local).and_then(|(s, p)| {
                geometry::sag(&s.spec.shape, p.0[0], p.0[1]).map(|v| p.0[2] - v.0)
            });
            issues.push(serde_json::json!({"reason":d.reason,"surface":d.surface,"branches":d.branch_count,"weight":d.weight,"sag_residual_mm":residual,"intersection":d.intersection,"ray":r}));
        }
        results.push(serde_json::json!({"max_events_per_launch":events,"settings":scene.settings,"seconds":seconds,"budget":report.budget,"diagnostics":issues}));
        eprintln!(
            "Interaction allowance {events}: {seconds:.3} s, {} intersections",
            report.surface_intersections
        );
    }
    fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"reference_plane":record.calculation.scene.reference_plane,"rays":rays.len(),"conditions":"Frozen explicit launch rays; only max_events_per_launch changes; four worker threads; preparation and output excluded from timing.","cases":results}),
        )?,
    )?;
    Ok(())
}
