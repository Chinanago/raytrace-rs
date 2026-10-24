//! Command-line tracing, project workflows and calculation replay.
use raytrace_core::math::V3;
use raytrace_core::model::{Mode, PreparedScene, Ray, Sampling, Scene};
use raytrace_core::trace::{trace, trace_generated, TraceOptions};
use raytrace_io::record::{LaunchInput, RunContext, RunRecord, RUN_FORMAT};
use raytrace_io::{project::Project, zemax};
use serde::Serialize;
use std::{env, error::Error, fs, path::PathBuf, time::Instant};

const HELP: &str =
    "raytrace-rs MODEL.json|MODEL.zmx [--output REPORT.json] [--rays-csv INPUT.csv]\n\
    [--mode sequential|non_sequential] [--threads N] [--repeat N]\n\
    [--terminals] [--paths FIRST_N_LAUNCH_RAYS]\n\
    [--inspect] [--export-zmx COPY.zmx] [--ignore-coatings]\n\
    [--wavelength-um VALUE] [--rays-per-source N] [--sampling sobol|pseudorandom]\n\
    raytrace-rs --replay RECORD.json [--output NEW_REPORT.json] [--threads N]\n\
    raytrace-rs --convergence MODEL [--plan PLAN.json] [--output DIRECTORY]\n\
    raytrace-rs --scan MODEL [--plan PLAN.json] [--output DIRECTORY]\n\
    raytrace-rs --resume-scan DIRECTORY\n\
光線CSVヘッダー: id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium\n\
単位: mm、um、任意の光量重み。発射光線の重み合計を100%の基準とします。\n\
シーケンシャルは透過光と指定したミラーを追跡し、全反射で光線を終了します。非シーケンシャルは反射光と透過光に分岐します。\n\
ベンチマーク時間は光源光線の生成・追跡・レポート集計を含み、入出力とウォームアップを除きます。";

fn read_csv(path: &str, scene: &PreparedScene) -> Result<Vec<Ray>, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or("CSVが空です")?
        .trim_start_matches('\u{feff}')
        .trim();
    if header != "id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium" {
        return Err("光線CSVのヘッダーが不正です（--helpを確認してください）".into());
    }
    let mut ids = std::collections::HashSet::new();
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: Vec<_> = line.split(',').map(str::trim).collect();
            if v.len() != 10 {
                return Err("CSVは10列で指定してください".into());
            }
            let mut ray = Ray {
                id: v[0]
                    .parse()
                    .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                position_mm: V3([
                    v[1].parse()
                        .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                    v[2].parse()
                        .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                    v[3].parse()
                        .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                ]),
                direction: V3([
                    v[4].parse()
                        .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                    v[5].parse()
                        .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                    v[6].parse()
                        .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                ]),
                weight: v[7]
                    .parse()
                    .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                wavelength_um: v[8]
                    .parse()
                    .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                medium: v[9]
                    .parse()
                    .map_err(|e| format!("CSVの数値が不正です: {e}"))?,
                optical_path_mm: 0.0,
            };
            if !ids.insert(ray.id) {
                return Err("CSVの発射光線IDが重複しています".into());
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
    let mut args: Vec<_> = env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("raytrace-rs {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("{HELP}");
        return Ok(());
    }
    if args[0] == "--convergence" {
        let model = args.get(1).ok_or("収束確認のモデルを指定してください")?;
        let mut plan = raytrace_app::convergence::ConvergencePlan::default();
        let mut output = PathBuf::from("results/convergence");
        let mut i = 2;
        while i < args.len() {
            let value = args
                .get(i + 1)
                .ok_or("収束確認オプションの値を指定してください")?;
            match args[i].as_str() {
                "--plan" => plan = serde_json::from_slice(&fs::read(value)?)?,
                "--output" => output = value.into(),
                option => return Err(format!("不明な収束確認オプションです: {option}").into()),
            }
            i += 2;
        }
        let report = raytrace_app::convergence::run(std::path::Path::new(model), &plan, &output)?;
        let failures = report.cases.iter().filter(|c| c.error.is_some()).count();
        println!(
            "収束確認: {}ケース、{}比較、{}失敗; {}",
            report.cases.len(),
            report.comparisons.len(),
            failures,
            output.join("convergence.json").display()
        );
        if failures > 0 {
            return Err(
                "収束確認で失敗したケースがあります。保存した概要を確認してください".into(),
            );
        }
        return Ok(());
    }
    if args[0] == "--scan" || args[0] == "--resume-scan" {
        use raytrace_app::scan::{ScanManifest, ScanPlan};
        use raytrace_core::trace::TraceControl;
        let input = args
            .get(1)
            .ok_or("走査のモデルまたはディレクトリを指定してください")?;
        let mut output = PathBuf::from("results/scan");
        let mut plan = ScanPlan::default();
        let mut i = 2;
        while i < args.len() {
            let value = args
                .get(i + 1)
                .ok_or("走査オプションの値を指定してください")?;
            match args[i].as_str() {
                "--plan" if args[0] == "--scan" => {
                    plan = serde_json::from_slice(&fs::read(value)?)?
                }
                "--output" if args[0] == "--scan" => output = value.into(),
                _ => return Err("不明な走査オプションです".into()),
            }
            i += 2;
        }
        let mut manifest = if args[0] == "--resume-scan" {
            output = input.into();
            ScanManifest::open(&output)?
        } else {
            if output.join("scan.json").exists() {
                return Err(
                    "走査の出力先は既に存在します。--resume-scanまたは新しいディレクトリを指定してください".into(),
                );
            }
            ScanManifest::new(&Project::open(std::path::Path::new(input), &[])?, plan)?
        };
        manifest.execute(&output, &TraceControl::default(), |m| {
            let completed = m
                .cases
                .iter()
                .filter(|c| c.state == "completed" || c.state == "failed")
                .count();
            println!("走査: {completed}/{}ケース", m.cases.len());
        })?;
        println!("走査を保存しました: {}", output.join("scan.json").display());
        if manifest.cases.iter().any(|c| c.state == "failed") {
            return Err("走査で失敗したケースがあります。scan.jsonを確認してください".into());
        }
        return Ok(());
    }
    let replay = if args[0] == "--replay" {
        args.remove(0);
        Some(RunRecord::open(std::path::Path::new(
            args.first()
                .ok_or("再計算する記録ファイルを指定してください")?,
        ))?)
    } else {
        None
    };
    let mut output = PathBuf::from(if replay.is_some() {
        "results/replay.json"
    } else {
        "results/report.json"
    });
    let mut csv = None;
    let mut threads = None;
    let mut repeat = 1usize;
    let mut mode = None;
    let mut options = replay
        .as_ref()
        .map_or_else(TraceOptions::default, |r| r.calculation.trace_options);
    let mut inspect = false;
    let mut export_zmx = None;
    let mut ignore_coatings = false;
    let mut wavelength = None;
    let mut rays_per_source = None;
    let mut sampling = None;
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
            | "--export-zmx" | "--wavelength-um" | "--rays-per-source" | "--sampling" => (),
            option => return Err(format!("不明なオプションです: {option}").into()),
        }
        let value = args.get(i + 1).ok_or("オプションの値を指定してください")?;
        match args[i].as_str() {
            "--sampling" => {
                sampling = Some(match value.as_str() {
                    "sobol" => Sampling::Sobol,
                    "pseudorandom" => Sampling::Pseudorandom,
                    _ => {
                        return Err("サンプリングはsobolまたはpseudorandomで指定してください".into())
                    }
                })
            }
            "--rays-per-source" => {
                rays_per_source = Some(
                    value
                        .parse::<usize>()
                        .map_err(|e| format!("オプションの数値が不正です: {e}"))?,
                )
            }
            "--export-zmx" => export_zmx = Some(PathBuf::from(value)),
            "--wavelength-um" => {
                wavelength = Some(
                    value
                        .parse::<f64>()
                        .map_err(|e| format!("オプションの数値が不正です: {e}"))?,
                )
            }
            "--output" => output = value.into(),
            "--rays-csv" => csv = Some(value.clone()),
            "--threads" => {
                let n: usize = value
                    .parse()
                    .map_err(|e| format!("オプションの数値が不正です: {e}"))?;
                if n == 0 {
                    return Err("スレッド数は正の値で指定してください".into());
                }
                threads = Some(n);
            }
            "--repeat" => {
                repeat = value
                    .parse()
                    .map_err(|e| format!("オプションの数値が不正です: {e}"))?;
                if repeat == 0 {
                    return Err("繰返し回数は正の値で指定してください".into());
                }
            }
            "--paths" => {
                options.path_launch_count = value
                    .parse()
                    .map_err(|e| format!("オプションの数値が不正です: {e}"))?
            }
            "--mode" => {
                mode = Some(match value.as_str() {
                    "sequential" => Mode::Sequential,
                    "non_sequential" => Mode::NonSequential,
                    _ => return Err("不明な追跡方式です".into()),
                })
            }
            _ => unreachable!(),
        }
        i += 2;
    }
    let mut builder = rayon::ThreadPoolBuilder::new();
    if replay.is_some()
        && (csv.is_some()
            || mode.is_some()
            || sampling.is_some()
            || rays_per_source.is_some()
            || wavelength.is_some()
            || ignore_coatings
            || inspect
            || export_zmx.is_some())
    {
        return Err("再計算では記録された光学条件を使います。変更できるのはoutput・threads・repeat・terminals・pathsだけです".into());
    }
    if let Some(n) = threads.or_else(|| replay.as_ref().map(|r| r.calculation.worker_threads)) {
        if n == 0 {
            return Err("スレッド数は正の値で指定してください".into());
        }
        builder = builder.num_threads(n);
    }
    let pool = builder.build()?;
    let mut project = match &replay {
        Some(r) => r.calculation.project.clone(),
        None => Project::open(std::path::Path::new(&args[0]), &[])?,
    };
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
            "読込結果: 光源{}個、面{}個、材料{}個",
            project.scene.sources.len(),
            project.scene.surfaces.len(),
            project.scene.materials.len()
        );
        for component in project.compatibility_report().components {
            println!(
                "{}{}",
                component.name,
                if component.enabled { "" } else { " [無効]" }
            );
            for entry in component.entries {
                println!("  {}: {}", entry.status.label(), entry.message);
            }
        }
        return Ok(());
    }
    let mut spec: Scene = match &replay {
        Some(r) => r.calculation.scene.clone(),
        None => project.trace_scene()?,
    };
    if let Some(sampling) = sampling {
        for source in &mut spec.sources {
            source.sampling = sampling;
        }
    }
    if let Some(count) = rays_per_source {
        for source in &mut spec.sources {
            source.count = count;
        }
    }
    if let Some(mode) = mode {
        if project.assembly.is_some() && mode == Mode::Sequential {
            return Err("組立光学系は非シーケンシャルで追跡してください".into());
        }
        spec.mode = mode;
    }
    let scene = spec.prepare()?;
    if fs::canonicalize(&output)
        .ok()
        .zip(fs::canonicalize(&args[0]).ok())
        .is_some_and(|(a, b)| a == b)
    {
        return Err("出力先は入力モデル・計算記録と異なるパスを指定してください".into());
    }
    let explicit = csv.is_some()
        || replay
            .as_ref()
            .is_some_and(|r| matches!(r.calculation.input, LaunchInput::Explicit { .. }));
    let rays = if explicit {
        Some(if let Some(r) = &replay {
            r.calculation.rays()?
        } else {
            read_csv(csv.as_ref().unwrap(), &scene)?
        })
    } else {
        if let Some(r) = &replay {
            r.calculation.validate_generated(&scene)?;
        }
        None
    };
    let input_count = match &rays {
        Some(rays) => {
            if rays.is_empty() {
                return Err("光源または光線CSVを指定してください".into());
            }
            if !rays.iter().map(|r| r.weight).sum::<f64>().is_finite()
                || rays.iter().all(|r| r.weight == 0.0)
            {
                return Err("発射光線の重み合計は正の有限値にしてください".into());
            }
            rays.len()
        }
        None => scene.ray_generator()?.len(),
    };
    if input_count == 0 {
        return Err("光源または光線CSVを指定してください".into());
    }
    let run = || match &rays {
        Some(rays) => Ok(trace(&scene, rays, options)),
        None => trace_generated(&scene, options),
    };
    std::hint::black_box(pool.install(run)?);
    let mut timings = Vec::with_capacity(repeat);
    let mut report = None;
    for _ in 0..repeat {
        let start = Instant::now();
        let result = pool.install(run)?;
        timings.push(start.elapsed().as_secs_f64());
        report = Some(result);
    }
    let report = report.unwrap();
    if !report.budget.launched.is_finite() || report.budget.launched <= 0. {
        return Err("発射光線の重み合計は正の有限値にしてください".into());
    }
    timings.sort_by(f64::total_cmp);
    let median = if repeat % 2 == 0 {
        (timings[repeat / 2 - 1] + timings[repeat / 2]) * 0.5
    } else {
        timings[repeat / 2]
    };
    let benchmark = Benchmark {
        repetitions: repeat,
        worker_threads: pool.current_num_threads(),
        input_rays: input_count,
        minimum_seconds: timings[0],
        median_seconds: median,
        launched_rays_per_second: input_count as f64 / median,
        surface_intersections_per_second: report.surface_intersections as f64 / median,
        timed_scope: if explicit {
            "Trace, branch traversal, and report accumulation; excludes scene/ray input, JSON output, and one warmup"
        } else {
            "Source ray generation, trace, branch traversal, and report accumulation; excludes scene input, JSON output, and one warmup"
        },
    };
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
    let calculation = RunContext::capture(
        &project,
        &scene.spec,
        rays.as_deref(),
        pool.current_num_threads(),
        options,
    );
    let result = serde_json::json!({
        "format": RUN_FORMAT, "calculation": calculation, "trace_seconds": median,
        "schema_version": 2, "scene": scene.spec, "reference_weight": reference,
        "budget_percent_of_reference": percentages,
        "detector_percent_of_reference": detector_percentages,
        "surface_encounter_percent_of_reference": surface_percentages,
        "relative_budget_closure_error": closure, "benchmark": benchmark, "report": report,
    });
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
    println!("基準: {} = 100%", scene.spec.reference_plane);
    println!(
        "発射光線数: {}; 検出光量: {:.6}%",
        input_count,
        100.0 * result["report"]["budget"]["detected"].as_f64().unwrap() / reference
    );
    println!(
        "追跡時間の中央値: {:.3} ms; {:.3}百万光線/秒; {}スレッド",
        median * 1000.0,
        input_count as f64 / median / 1e6,
        pool.current_num_threads()
    );
    println!(
        "光量収支の誤差: {:.3e}; レポート: {}",
        closure,
        output.display()
    );
    let budget = &result["report"]["budget"];
    if ["numerical_failure", "medium_mismatch", "interaction_limit"]
        .iter()
        .any(|key| budget[key].as_f64().unwrap() > 0.0)
    {
        eprintln!("未解決の光線重みがあります。保存したレポートのnumerical_failure・medium_mismatch・interaction_limitを確認してください。");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("エラー: {error}");
        std::process::exit(1);
    }
}
