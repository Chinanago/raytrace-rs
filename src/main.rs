use raytrace_rs::math::V3;
use raytrace_rs::model::{Mode, PreparedScene, Ray, Scene};
use raytrace_rs::trace::{trace, TraceOptions};
use raytrace_rs::{project::Project, zemax};
use serde::Serialize;
use std::{env, error::Error, fs, path::PathBuf, time::Instant};

const HELP: &str =
    "raytrace-rs MODEL.json|MODEL.zmx [--output REPORT.json] [--rays-csv INPUT.csv]\n\
    [--mode sequential|non_sequential] [--threads N] [--repeat N]\n\
    [--terminals] [--paths FIRST_N_LAUNCH_IDS]\n\
    [--inspect] [--export-zmx COPY.zmx] [--ignore-coatings]\n\
    [--wavelength-um VALUE] [--rays-per-source N]\n\
Ray CSV header: id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium\n\
Units: mm, um, arbitrary power weights. Launch-weight sum is the 100% reference.\n\
Sequential follows transmission, explicit mirrors and TIR. Non-sequential splits R/T.\n\
Benchmark timing includes tracing/report accumulation; excludes input/output and warmup.";

fn read_csv(path: &str, scene: &PreparedScene) -> Result<Vec<Ray>, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or("Empty CSV")?
        .trim_start_matches('\u{feff}')
        .trim();
    if header != "id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium" {
        return Err("Unexpected ray CSV header (see --help)".into());
    }
    let mut ids = std::collections::HashSet::new();
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: Vec<_> = line.split(',').map(str::trim).collect();
            if v.len() != 10 {
                return Err("Expected 10 CSV fields".into());
            }
            let mut ray = Ray {
                id: v[0].parse()?,
                position_mm: V3([v[1].parse()?, v[2].parse()?, v[3].parse()?]),
                direction: V3([v[4].parse()?, v[5].parse()?, v[6].parse()?]),
                weight: v[7].parse()?,
                wavelength_um: v[8].parse()?,
                medium: v[9].parse()?,
                optical_path_mm: 0.0,
            };
            if !ids.insert(ray.id) {
                return Err("CSV launch IDs must be unique".into());
            }
            ray.validate(scene)?;
            Ok(ray)
        })
        .collect()
}

#[derive(Serialize)]
struct Benchmark {
    repetitions: usize,
    worker_threads: usize,
    input_rays: usize,
    minimum_seconds: f64,
    median_seconds: f64,
    launched_rays_per_second: f64,
    surface_intersections_per_second: f64,
    timed_scope: &'static str,
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("{HELP}");
        return Ok(());
    }
    let mut output = PathBuf::from("results/report.json");
    let mut csv = None;
    let mut threads = None;
    let mut repeat = 1usize;
    let mut mode = None;
    let mut options = TraceOptions::default();
    let mut inspect = false;
    let mut export_zmx = None;
    let mut ignore_coatings = false;
    let mut wavelength = None;
    let mut rays_per_source = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--inspect" => {
                inspect = true;
                i += 1;
                continue;
            }
            "--ignore-coatings" => {
                ignore_coatings = true;
                i += 1;
                continue;
            }
            "--terminals" => {
                options.keep_terminals = true;
                i += 1;
                continue;
            }
            "--output" | "--rays-csv" | "--threads" | "--repeat" | "--mode" | "--paths"
            | "--export-zmx" | "--wavelength-um" | "--rays-per-source" => (),
            option => return Err(format!("Unknown option: {option}").into()),
        }
        let value = args.get(i + 1).ok_or("Missing option value")?;
        match args[i].as_str() {
            "--rays-per-source" => rays_per_source = Some(value.parse::<usize>()?),
            "--export-zmx" => export_zmx = Some(PathBuf::from(value)),
            "--wavelength-um" => wavelength = Some(value.parse::<f64>()?),
            "--output" => output = value.into(),
            "--rays-csv" => csv = Some(value.clone()),
            "--threads" => {
                let n: usize = value.parse()?;
                if n == 0 {
                    return Err("Thread count must be positive".into());
                }
                threads = Some(n);
            }
            "--repeat" => {
                repeat = value.parse()?;
                if repeat == 0 {
                    return Err("Repeat must be positive".into());
                }
            }
            "--paths" => options.path_launch_count = value.parse()?,
            "--mode" => {
                mode = Some(match value.as_str() {
                    "sequential" => Mode::Sequential,
                    "non_sequential" => Mode::NonSequential,
                    _ => return Err("Unknown mode".into()),
                })
            }
            _ => unreachable!(),
        }
        i += 2;
    }
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(n) = threads {
        builder = builder.num_threads(n);
    }
    let pool = builder.build()?;
    let mut project = Project::open(std::path::Path::new(&args[0]), &[])?;
    project.allow_uncoated = ignore_coatings || project.allow_uncoated;
    if let Some(w) = wavelength {
        project.monochromatic_override_um = Some(w);
    }
    if let Some(path) = export_zmx {
        zemax::export(&project, &path)?;
    }
    if inspect {
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        project.save(&output)?;
        println!(
            "Native import: {} sources, {} surfaces, {} materials",
            project.scene.sources.len(),
            project.scene.surfaces.len(),
            project.scene.materials.len()
        );
        for issue in &project.issues {
            println!(
                "{}: {}",
                if issue.blocking { "ACTION" } else { "INFO" },
                issue.message
            );
        }
        return Ok(());
    }
    let mut spec: Scene = project.trace_scene()?;
    if let Some(count) = rays_per_source {
        for source in &mut spec.sources {
            source.count = count;
        }
    }
    if let Some(mode) = mode {
        spec.mode = mode;
    }
    let scene = spec.prepare()?;
    let rays = if let Some(path) = csv {
        read_csv(&path, &scene)?
    } else {
        scene.generate_rays()?
    };
    if rays.is_empty() {
        return Err("Supply a source or ray CSV".into());
    }
    if !rays.iter().map(|r| r.weight).sum::<f64>().is_finite()
        || rays.iter().all(|r| r.weight == 0.0)
    {
        return Err("Launch-weight sum must be finite and positive".into());
    }
    pool.install(|| {
        std::hint::black_box(trace(&scene, &rays, options));
    });
    let mut timings = Vec::with_capacity(repeat);
    let mut report = None;
    for _ in 0..repeat {
        let start = Instant::now();
        let result = pool.install(|| trace(&scene, &rays, options));
        timings.push(start.elapsed().as_secs_f64());
        report = Some(result);
    }
    let report = report.unwrap();
    timings.sort_by(f64::total_cmp);
    let median = if repeat.is_multiple_of(2) {
        (timings[repeat / 2 - 1] + timings[repeat / 2]) * 0.5
    } else {
        timings[repeat / 2]
    };
    let benchmark = Benchmark { repetitions: repeat, worker_threads: pool.current_num_threads(), input_rays: rays.len(),
        minimum_seconds: timings[0], median_seconds: median, launched_rays_per_second: rays.len() as f64/median,
        surface_intersections_per_second: report.surface_intersections as f64/median,
        timed_scope: "Trace, branch traversal, and report accumulation; excludes scene/ray input, JSON output, and one warmup" };
    let reference = report.budget.launched;
    let raw = serde_json::to_value(&report.budget)?;
    let percentages: serde_json::Map<String, serde_json::Value> = raw
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                serde_json::json!(100.0 * value.as_f64().unwrap() / reference),
            )
        })
        .collect();
    let closure = (report.budget.accounted() - reference) / reference;
    let detector_percentages: serde_json::Map<String, serde_json::Value> = report
        .detectors
        .iter()
        .map(|d| {
            (
                d.name.clone(),
                serde_json::json!(100.0 * d.weights.iter().sum::<f64>() / reference),
            )
        })
        .collect();
    let surface_percentages: Vec<_> = report
        .surfaces
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name, "incident": 100.0*s.incident_weight/reference,
                "transmitted": 100.0*s.transmitted_weight/reference,
                "reflected": 100.0*s.reflected_weight/reference,
                "absorbed": 100.0*s.absorbed_weight/reference,
                "detected": 100.0*s.detected_weight/reference,
                "geometric_miss": 100.0*s.geometric_miss_weight/reference,
            })
        })
        .collect();
    let result = serde_json::json!({
        "schema_version": 1, "scene": scene.spec, "reference_weight": reference,
        "budget_percent_of_reference": percentages,
        "detector_percent_of_reference": detector_percentages,
        "surface_encounter_percent_of_reference": surface_percentages,
        "relative_budget_closure_error": closure, "benchmark": benchmark, "report": report,
    });
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
    println!("Reference: {} = 100%", scene.spec.reference_plane);
    println!(
        "Launches: {}; detected: {:.6}%",
        rays.len(),
        100.0 * result["report"]["budget"]["detected"].as_f64().unwrap() / reference
    );
    println!(
        "Trace median: {:.3} ms; {:.3} million launches/s; {} threads",
        median * 1000.0,
        rays.len() as f64 / median / 1e6,
        pool.current_num_threads()
    );
    println!(
        "Budget closure: {:.3e}; report: {}",
        closure,
        output.display()
    );
    let budget = &result["report"]["budget"];
    if ["numerical_failure", "medium_mismatch", "interaction_limit"]
        .iter()
        .any(|key| budget[key].as_f64().unwrap() > 0.0)
    {
        eprintln!("Unresolved ray weight exists; inspect numerical_failure, medium_mismatch, and interaction_limit in the saved report.");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
