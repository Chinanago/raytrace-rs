//! Command-line front end of the JSON engine.
//!
//! `raytrace-rs run REQUEST.json` executes engine requests directly. The classic options
//! (`MODEL --output ...`, `--replay`, `--scan`, ...) are translated into the same requests.
//! stdout always carries one JSON response per request; people read stderr.
use raytrace_app::{
    convergence::ConvergencePlan,
    engine::{
        self, ConvergenceRequest, EngineError, InspectRequest, ModelInput, Outcome, Overrides,
        Progress, RaysInput, RecordInput, ReplayRequest, Request, ResumeScanRequest, ScanRequest,
        SchemaName, TraceRequest,
    },
    scan::ScanPlan,
};
use raytrace_core::{
    model::{Mode, Sampling},
    trace::TraceControl,
};
use serde_json::Value;
use std::{
    env, fs,
    io::{Read, Write},
    path::PathBuf,
};

const HELP: &str = "raytrace-rs run REQUEST.json|-\n\
    raytrace-rs MODEL.json|MODEL.zmx [--output REPORT.json] [--rays-csv INPUT.csv]\n\
    [--mode sequential|non_sequential] [--threads N] [--repeat N]\n\
    [--terminals] [--paths FIRST_N_LAUNCH_RAYS]\n\
    [--inspect] [--export-zmx COPY.zmx] [--ignore-coatings]\n\
    [--wavelength-um VALUE] [--rays-per-source N] [--sampling sobol|pseudorandom]\n\
    raytrace-rs --replay RECORD.json [--output NEW_REPORT.json] [--threads N]\n\
    raytrace-rs --convergence MODEL [--plan PLAN.json] [--output DIRECTORY]\n\
    raytrace-rs --scan MODEL [--plan PLAN.json] [--output DIRECTORY]\n\
    raytrace-rs --resume-scan DIRECTORY\n\
    raytrace-rs --schema NAME [--output FILE] | --schema all --output DIRECTORY\n\
    raytrace-rs --version\n\
run: JSONリクエスト（--schema request）を実行します。- は標準入力で、複数のリクエストを続けて渡せます。\n\
オプション形式も同じリクエストに変換して実行します。\n\
標準出力: リクエストごとに1行のJSON（--schema response）。成功時 \"ok\": true と result、失敗時 \"ok\": false と error.code・error.message。人向けの表示は標準エラーへ出力します。\n\
スキーマ名: request, response, scene, project, run, scan_plan, scan, convergence_plan, convergence\n\
光線CSVヘッダー: id,x_mm,y_mm,z_mm,dx,dy,dz,weight,wavelength_um,medium\n\
単位: mm、um、任意の光量重み。発射光線の重み合計を100%の基準とします。\n\
シーケンシャルは透過光と指定したミラーを追跡し、全反射で光線を終了します。非シーケンシャルは反射光と透過光に分岐します。\n\
オプション形式の追跡はウォームアップを1回行い、光源光線の生成・追跡・レポート集計の時間を計ります。";

/// A CLI argument error before any request runs.
fn usage(message: impl Into<String>) -> EngineError {
    EngineError::new("usage", message)
}

fn value(args: &[String], i: usize) -> Result<&String, EngineError> {
    args.get(i + 1)
        .ok_or_else(|| usage(format!("{} の値を指定してください", args[i])))
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, EngineError>
where
    T::Err: std::fmt::Display,
{
    text.parse()
        .map_err(|e| usage(format!("オプションの数値が不正です: {e}")))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, EngineError> {
    let bytes = fs::read(path).map_err(|e| raytrace_io::Error::io(path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| raytrace_io::Error::json(Some(PathBuf::from(path)), e).into())
}

/// `--scan`, `--resume-scan` and `--convergence`.
fn study(args: &[String]) -> Result<Request, EngineError> {
    let input = args
        .get(1)
        .ok_or_else(|| usage("走査・収束確認のモデルまたはディレクトリを指定してください"))?;
    let mut plan = None;
    let mut output = None;
    let mut i = 2;
    while i < args.len() {
        let v = value(args, i)?;
        match args[i].as_str() {
            "--plan" if args[0] != "--resume-scan" => plan = Some(v.clone()),
            "--output" if args[0] != "--resume-scan" => output = Some(PathBuf::from(v)),
            option => return Err(usage(format!("不明なオプションです: {option}"))),
        }
        i += 2;
    }
    Ok(match args[0].as_str() {
        "--resume-scan" => Request::ResumeScan(ResumeScanRequest {
            directory: input.into(),
            include_manifest: false,
        }),
        "--scan" => Request::Scan(ScanRequest {
            model: ModelInput::Path(input.into()),
            catalogs: Vec::new(),
            plan: plan.map_or_else(|| Ok(ScanPlan::default()), |p| read_json(&p))?,
            output: output.unwrap_or_else(|| "results/scan".into()),
            include_manifest: false,
        }),
        _ => Request::Convergence(ConvergenceRequest {
            model: input.into(),
            plan: plan.map_or_else(|| Ok(ConvergencePlan::default()), |p| read_json(&p))?,
            output: output.unwrap_or_else(|| "results/convergence".into()),
            include_report: false,
        }),
    })
}

/// Classic options → one request.
fn parse(mut args: Vec<String>) -> Result<Request, EngineError> {
    if matches!(
        args[0].as_str(),
        "--scan" | "--resume-scan" | "--convergence"
    ) {
        return study(&args);
    }
    let replay = args[0] == "--replay";
    if replay {
        args.remove(0);
    }
    let input = args
        .first()
        .ok_or_else(|| usage("再計算する記録ファイルを指定してください"))?
        .clone();
    let mut output = None;
    let mut csv = None;
    let mut threads = None;
    let mut repeat = 1usize;
    let mut terminals = false;
    let mut paths = None;
    let mut inspect = false;
    let mut export_zmx = None;
    let mut o = Overrides::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--inspect" => inspect = true,
            "--ignore-coatings" => o.ignore_coatings = true,
            "--terminals" => terminals = true,
            flag => {
                let v = value(&args, i)?;
                match flag {
                    "--output" => output = Some(PathBuf::from(v)),
                    "--rays-csv" => csv = Some(PathBuf::from(v)),
                    "--export-zmx" => export_zmx = Some(PathBuf::from(v)),
                    "--threads" => {
                        let n: usize = number(v)?;
                        if n == 0 {
                            return Err(usage("スレッド数は正の値で指定してください"));
                        }
                        threads = Some(n);
                    }
                    "--repeat" => {
                        repeat = number(v)?;
                        if repeat == 0 {
                            return Err(usage("繰返し回数は正の値で指定してください"));
                        }
                    }
                    "--paths" => paths = Some(number(v)?),
                    "--wavelength-um" => o.wavelength_um = Some(number(v)?),
                    "--rays-per-source" => o.rays_per_source = Some(number(v)?),
                    "--sampling" => {
                        o.sampling = Some(match v.as_str() {
                            "sobol" => Sampling::Sobol,
                            "pseudorandom" => Sampling::Pseudorandom,
                            _ => {
                                return Err(usage(
                                    "サンプリングはsobolまたはpseudorandomで指定してください",
                                ));
                            }
                        })
                    }
                    "--mode" => {
                        o.mode = Some(match v.as_str() {
                            "sequential" => Mode::Sequential,
                            "non_sequential" => Mode::NonSequential,
                            _ => return Err(usage("不明な追跡方式です")),
                        })
                    }
                    option => return Err(usage(format!("不明なオプションです: {option}"))),
                }
                i += 1;
            }
        }
        i += 1;
    }
    if replay {
        if csv.is_some()
            || o.mode.is_some()
            || o.sampling.is_some()
            || o.rays_per_source.is_some()
            || o.wavelength_um.is_some()
            || o.ignore_coatings
            || inspect
            || export_zmx.is_some()
        {
            return Err(usage(
                "再計算では記録された光学条件を使います。変更できるのはoutput・threads・repeat・terminals・pathsだけです",
            ));
        }
        return Ok(Request::Replay(ReplayRequest {
            record: RecordInput::Path(input.into()),
            threads,
            repeat,
            warmup: true,
            terminals: terminals.then_some(true),
            paths,
            output: Some(output.unwrap_or_else(|| "results/replay.json".into())),
            include_record: false,
        }));
    }
    let output = Some(output.unwrap_or_else(|| "results/report.json".into()));
    if inspect {
        return Ok(Request::Inspect(InspectRequest {
            model: ModelInput::Path(input.into()),
            catalogs: Vec::new(),
            output,
            export_zmx,
            ignore_coatings: o.ignore_coatings,
            wavelength_um: o.wavelength_um,
            include_project: false,
        }));
    }
    Ok(Request::Trace(TraceRequest {
        model: ModelInput::Path(input.into()),
        catalogs: Vec::new(),
        rays: csv.map(RaysInput::Csv),
        overrides: o,
        threads,
        repeat,
        warmup: true,
        terminals,
        paths: paths.unwrap_or(0),
        preview_paths: 0,
        max_total_rays: None,
        export_zmx,
        output,
        include_record: false,
    }))
}

/// Japanese lines for stderr.
fn describe(outcome: &Outcome) -> Vec<String> {
    match outcome {
        Outcome::Version { version } => vec![format!("raytrace-rs {version}")],
        Outcome::Schema { .. } => Vec::new(),
        Outcome::Opened(_) => vec!["読込完了".into()],
        Outcome::Inspected(r) => {
            let mut lines = vec![format!(
                "読込結果: 光源{}個、面{}個、材料{}個",
                r.sources, r.surfaces, r.materials
            )];
            for component in &r.compatibility.components {
                lines.push(format!(
                    "{}{}",
                    component.name,
                    if component.enabled { "" } else { " [無効]" }
                ));
                for entry in &component.entries {
                    lines.push(format!("  {}: {}", entry.status.label(), entry.message));
                }
            }
            lines
        }
        Outcome::Traced(r) => {
            let mut lines = vec![
                format!("基準: {} = 100%", r.reference_plane),
                format!(
                    "発射光線数: {}; 検出光量: {:.6}%",
                    r.launched_rays, r.detected_percent_of_reference
                ),
                format!(
                    "追跡時間の中央値: {:.3} ms; {:.3}百万光線/秒; {}スレッド",
                    r.median_seconds * 1000.0,
                    r.launched_rays as f64 / r.median_seconds / 1e6,
                    r.worker_threads
                ),
                format!(
                    "光量収支の誤差: {:.3e}; レポート: {}",
                    r.relative_budget_closure_error,
                    r.output
                        .as_ref()
                        .map_or_else(|| "保存なし".into(), |p| p.display().to_string())
                ),
            ];
            lines.extend(r.warnings.iter().cloned());
            lines
        }
        Outcome::Scanned(r) => vec![format!(
            "走査を保存しました: {}（{}ケース、完了{}、失敗{}）",
            r.output.display(),
            r.cases,
            r.completed,
            r.failed
        )],
        Outcome::Converged(r) => vec![format!(
            "収束確認: {}ケース、{}比較、{}失敗; {}",
            r.cases,
            r.comparisons,
            r.failed,
            r.output.display()
        )],
        Outcome::Saved { output } => vec![format!("保存しました: {}", output.display())],
    }
}

fn print(stdout: &mut impl Write, response: &engine::Response) {
    let line = serde_json::to_string(response).expect("responses serialize");
    let _ = writeln!(stdout, "{line}");
}

/// Run one request, print its response line and the human summary. Returns success.
fn perform(request: Request, stdout: &mut impl Write) -> bool {
    let command = request.command();
    let mut progress = |p: Progress<'_>| {
        if let Progress::Scan(m) = p {
            let done = m
                .cases
                .iter()
                .filter(|c| c.state == "completed" || c.state == "failed")
                .count();
            eprintln!("走査: {done}/{}ケース", m.cases.len());
        }
    };
    let result = engine::execute_with(request, &TraceControl::default(), &mut progress);
    if let Ok(outcome) = &result {
        for line in describe(outcome) {
            eprintln!("{line}");
        }
    }
    let response = engine::respond(command, result);
    if let Some(error) = &response.error {
        eprintln!("エラー: {}", error.message);
    }
    print(stdout, &response);
    response.ok
}

fn fail(stdout: &mut impl Write, command: &str, error: EngineError) -> bool {
    eprintln!("エラー: {error}");
    print(stdout, &engine::respond(command, Err(error)));
    false
}

/// `run FILE|-`: a stream of JSON requests, one response line each.
fn run_requests(source: Option<&String>, stdout: &mut impl Write) -> bool {
    let text = match source.map(String::as_str) {
        None => {
            return fail(
                stdout,
                "run",
                usage("リクエストJSONのファイルか - を指定してください"),
            );
        }
        Some("-") => {
            let mut text = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut text) {
                return fail(stdout, "run", raytrace_io::Error::io("-", e).into());
            }
            text
        }
        Some(path) => match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => return fail(stdout, "run", raytrace_io::Error::io(path, e).into()),
        },
    };
    let mut ok = true;
    let mut any = false;
    for value in serde_json::Deserializer::from_str(&text).into_iter::<Value>() {
        any = true;
        let request = value
            .map_err(|e| e.to_string())
            .and_then(|v| serde_json::from_value::<Request>(v).map_err(|e| e.to_string()));
        ok &= match request {
            Ok(request) => perform(request, stdout),
            Err(e) => fail(
                stdout,
                "unknown",
                EngineError::new(
                    "invalid_request",
                    format!("リクエストJSONが不正です（--schema request を確認してください）: {e}"),
                ),
            ),
        };
    }
    if !any {
        return fail(stdout, "run", usage("リクエストJSONが空です"));
    }
    ok
}

fn render_schema(name: SchemaName) -> Result<String, EngineError> {
    match engine::execute(Request::Schema { name })? {
        Outcome::Schema { schema } => {
            Ok(serde_json::to_string_pretty(&schema).expect("schemas serialize") + "\n")
        }
        _ => unreachable!("schema requests return schemas"),
    }
}

/// `--schema NAME [--output FILE]` or `--schema all --output DIR`.
/// Returns None when the schema itself was printed to stdout.
fn schemas(args: &[String], stdout: &mut impl Write) -> Result<Option<Outcome>, EngineError> {
    let names = || SchemaName::ALL.map(SchemaName::as_str).join(", ");
    let name = args
        .get(1)
        .ok_or_else(|| usage(format!("スキーマ名を指定してください: {}, all", names())))?;
    let output = match args.get(2..) {
        Some([flag, path]) if flag == "--output" => Some(PathBuf::from(path)),
        Some([]) | None => None,
        _ => {
            return Err(usage("--schema に指定できるオプションは --output だけです"));
        }
    };
    let write = |path: &PathBuf, text: String| -> Result<(), EngineError> {
        fs::write(path, text).map_err(|e| raytrace_io::Error::io(path, e).into())
    };
    if name == "all" {
        let dir = output
            .ok_or_else(|| usage("--schema all には --output DIRECTORY を指定してください"))?;
        fs::create_dir_all(&dir).map_err(|e| raytrace_io::Error::io(&dir, e))?;
        for schema in SchemaName::ALL {
            let path = dir.join(format!("{}.schema.json", schema.as_str()));
            write(&path, render_schema(schema)?)?;
        }
        return Ok(Some(Outcome::Saved { output: dir }));
    }
    let schema = SchemaName::parse(name)
        .ok_or_else(|| usage(format!("不明なスキーマ名です: {name}（{}, all）", names())))?;
    let text = render_schema(schema)?;
    match output {
        Some(path) => {
            write(&path, text)?;
            Ok(Some(Outcome::Saved { output: path }))
        }
        None => {
            let _ = stdout.write_all(text.as_bytes());
            Ok(None)
        }
    }
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // Output is always JSON; `--json` remains accepted for earlier scripts.
    args.retain(|a| a != "--json");
    let mut stdout = std::io::stdout().lock();
    let ok = match args.first().map(String::as_str) {
        None | Some("--help" | "-h") => {
            let _ = writeln!(stdout, "{HELP}");
            true
        }
        // Plain text: packaging scripts compare it with the crate version.
        Some("--version" | "-V") => {
            let _ = writeln!(stdout, "raytrace-rs {}", env!("CARGO_PKG_VERSION"));
            true
        }
        Some("run") => run_requests(args.get(1), &mut stdout),
        Some("--schema") => match schemas(&args, &mut stdout) {
            Ok(None) => true,
            Ok(Some(outcome)) => {
                print(&mut stdout, &engine::respond("schema", Ok(outcome)));
                true
            }
            Err(error) => fail(&mut stdout, "schema", error),
        },
        Some(_) => match parse(args) {
            Ok(request) => perform(request, &mut stdout),
            Err(error) => fail(&mut stdout, "usage", error),
        },
    };
    let _ = stdout.flush();
    if !ok {
        std::process::exit(1);
    }
}
