//! The JSON engine: CLI responses, `run` requests, in-process execution and schemas.
#[path = "support/cli.rs"]
mod cli_support;
use raytrace_rs::{
    engine::{self, ModelInput, Outcome, Request, SchemaName, TraceRequest},
    model::Scene,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/engine_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn small_lens() -> Scene {
    let mut scene: Scene =
        serde_json::from_str(include_str!("../examples/sequential_lens.json")).unwrap();
    for source in &mut scene.sources {
        source.count = 16;
    }
    scene
}

/// Run the CLI and parse every stdout line as one JSON response.
fn cli(dir: &PathBuf, args: &[&str], stdin: Option<&str>) -> (bool, Vec<Value>) {
    let mut child = Command::new(cli_support::executable())
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or("").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect();
    (output.status.success(), responses)
}

#[test]
fn classic_options_print_one_json_response_per_command() {
    let dir = folder();
    fs::write(
        dir.join("input.json"),
        serde_json::to_vec(&small_lens()).unwrap(),
    )
    .unwrap();

    let (ok, trace) = cli(
        &dir,
        &["input.json", "--threads", "1", "--output", "run.json"],
        None,
    );
    assert!(ok);
    let [trace] = trace.as_slice() else { panic!() };
    assert_eq!(trace["ok"], true);
    assert_eq!(trace["command"], "trace");
    assert_eq!(trace["result"]["launched_rays"], 32);
    assert!(
        trace["result"]["detected_percent_of_reference"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert!(dir.join("run.json").is_file());

    let (ok, replay) = cli(
        &dir,
        &["--replay", "run.json", "--output", "replay.json"],
        None,
    );
    assert!(ok);
    assert_eq!(replay[0]["command"], "replay");
    assert_eq!(
        replay[0]["result"]["detected_percent_of_reference"],
        trace["result"]["detected_percent_of_reference"]
    );

    // `--json` is accepted and changes nothing.
    let (ok, inspect) = cli(
        &dir,
        &["input.json", "--inspect", "--json", "--output", "p.json"],
        None,
    );
    assert!(ok);
    assert_eq!(inspect[0]["command"], "inspect");
    assert_eq!(inspect[0]["result"]["traceable"], true);

    let (ok, missing) = cli(&dir, &["missing.json"], None);
    assert!(!ok);
    assert_eq!(missing[0]["ok"], false);
    assert_eq!(missing[0]["error"]["code"], "io");
    assert!(
        missing[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing.json")
    );

    let (ok, usage) = cli(&dir, &["input.json", "--threads", "0"], None);
    assert!(!ok);
    assert_eq!(usage[0]["error"]["code"], "usage");
}

#[test]
fn run_reads_a_stream_of_requests_with_inline_models() {
    let dir = folder();
    let requests = [
        json!({"command": "version"}),
        json!({
            "command": "trace",
            "model": {"scene": small_lens()},
            "threads": 1,
            "include_record": true,
        }),
        json!({"command": "trace", "model": {"path": "missing.zmx"}}),
        json!({"command": "nonsense"}),
    ]
    .map(|r| r.to_string())
    .join("\n");
    let (ok, responses) = cli(&dir, &["run", "-"], Some(&requests));
    assert!(!ok, "a failed request makes the exit code non-zero");
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["result"]["version"], env!("CARGO_PKG_VERSION"));
    let record = &responses[1]["result"]["record"];
    assert_eq!(record["format"], "raytrace-run-v1");
    assert_eq!(record["benchmark"]["input_rays"], 32);
    assert_eq!(responses[2]["error"]["code"], "io");
    assert_eq!(responses[3]["error"]["code"], "invalid_request");

    fs::write(dir.join("request.json"), requests.lines().next().unwrap()).unwrap();
    let (ok, from_file) = cli(&dir, &["run", "request.json"], None);
    assert!(ok);
    assert_eq!(from_file[0]["command"], "version");
}

#[test]
fn in_process_requests_match_the_cli_and_round_trip_as_json() {
    let request = Request::Trace(TraceRequest {
        model: ModelInput::Scene(Box::new(small_lens())),
        catalogs: Vec::new(),
        rays: None,
        overrides: Default::default(),
        threads: Some(1),
        repeat: 1,
        warmup: false,
        terminals: false,
        paths: 0,
        preview_paths: 8,
        max_total_rays: Some(1_000),
        export_zmx: None,
        output: None,
        include_record: true,
    });
    // The GUI builds requests in memory; they must mean the same as their JSON form.
    let text = serde_json::to_string(&request).unwrap();
    let reparsed: Request = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_string(&reparsed).unwrap(), text);

    let Outcome::Traced(result) = engine::execute(request).unwrap() else {
        panic!("trace returns a trace result")
    };
    let record = result.record.unwrap();
    assert!(!record.report.paths.is_empty(), "preview paths are traced");

    let response =
        engine::execute_json(&text.replace("\"max_total_rays\":1000", "\"max_total_rays\":10"));
    assert!(!response.ok);
    let error = response.error.unwrap();
    assert_eq!(error.code, "limit");
    assert!(error.message.contains("合計1〜10本"));

    let dir = folder();
    fs::write(
        dir.join("input.json"),
        serde_json::to_vec(&small_lens()).unwrap(),
    )
    .unwrap();
    let (_, cli) = cli(
        &dir,
        &["input.json", "--threads", "1", "--output", "run.json"],
        None,
    );
    assert_eq!(
        serde_json::to_value(&record.budget_percent_of_reference).unwrap(),
        cli[0]["result"]["budget_percent_of_reference"]
    );
}

#[test]
fn published_schemas_match_the_engine() {
    let dir = folder();
    let (ok, _) = cli(&dir, &["--schema", "all", "--output", "schemas"], None);
    assert!(ok);
    let published = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas");
    for name in SchemaName::ALL {
        let file = format!("{}.schema.json", name.as_str());
        assert_eq!(
            fs::read_to_string(published.join(&file)).unwrap_or_default(),
            fs::read_to_string(dir.join("schemas").join(&file)).unwrap(),
            "schemas/{file} is stale: run `cargo run -p raytrace-cli -- --schema all --output schemas`"
        );
    }
    let printed = Command::new(cli_support::executable())
        .args(["--schema", "scan-plan"])
        .output()
        .unwrap();
    assert!(printed.status.success());
    let schema: Value = serde_json::from_slice(&printed.stdout).unwrap();
    assert_eq!(schema["title"], "ScanPlan", "the schema itself is printed");
}
