#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Real CLI and MCP processes address one session through the GUI's owner protocol.
#[path = "support/cli.rs"]
mod cli;
use raytrace_core::trace::TraceControl;
use raytrace_engine::{Session, remote::Server};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

struct Host {
    path: PathBuf,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<Session>>,
    active: Arc<Mutex<Option<Arc<TraceControl>>>>,
}
impl Host {
    fn start() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "raytrace-connected-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session.json");
        let server = Server::start(&path, || {}).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let active = Arc::new(Mutex::new(None));
        let running = active.clone();
        let worker = thread::spawn(move || {
            let mut session = Session::new();
            while !stop.load(Ordering::Relaxed) {
                if let Some(request) = server.try_recv() {
                    *running.lock().unwrap() = Some(request.control());
                    request.prepare(session.clone()).commit(&mut session);
                    *running.lock().unwrap() = None;
                } else {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            session
        });
        Self {
            path,
            stopped,
            worker: Some(worker),
            active,
        }
    }
    fn cli(&self, args: &[&str], input: Option<&str>) -> Vec<Value> {
        let mut child = Command::new(cli::executable())
            .arg("--connect")
            .arg(&self.path)
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}

#[test]
fn mcp_cancellation_reaches_a_trace_in_the_shared_session() {
    let host = Host::start();
    host.cli(
        &[
            "doc.open",
            "path=examples/sequential_lens.json",
            "expected_revision:=0",
        ],
        None,
    );
    let mut child = Command::new(cli::executable())
        .args(["mcp", "--connect"])
        .arg(&host.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"execute","arguments":{"command":"analysis.trace","params":{"overrides":{"rays_per_source":1_000_000},"threads":1,"repeat":10}}}})).unwrap();
    input.flush().unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(20);
    let control = loop {
        let current = host.active.lock().unwrap().clone();
        if let Some(control) = current
            && control.total.load(Ordering::Relaxed) > 0
        {
            break control;
        }
        assert!(std::time::Instant::now() < until, "trace did not start");
        thread::sleep(Duration::from_millis(1));
    };
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}})
    )
    .unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        "cancelled"
    );
    assert!(control.cancel.load(Ordering::Relaxed));
    let status = host.cli(&["session.status"], None);
    assert_eq!(status[0]["revision"], 1);
    assert_eq!(status[0]["result"]["run"], Value::Null);
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
        fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
    }
}

#[test]
fn independent_cli_and_mcp_processes_edit_and_trace_the_same_session() {
    let host = Host::start();
    assert_eq!(
        host.cli(
            &[
                "doc.open",
                "path=examples/sequential_lens.json",
                "expected_revision:=0"
            ],
            None
        )[0]["revision"],
        1
    );
    let edit = host.cli(&["run", "-"], Some("{\"command\":\"doc.set\",\"path\":\"/name\",\"value\":\"Shared\",\"expected_revision\":1}\n{\"command\":\"session.status\"}\n"));
    assert_eq!(edit[0]["revision"], 2);
    assert_eq!(
        edit[1]["revision"], 2,
        "reads must not bump the document revision"
    );
    let lines = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"batch","arguments":{"requests":[
            {"command":"session.undo","params":{"expected_revision":2}},
            {"command":"session.redo","params":{"expected_revision":3}},
            {"command":"analysis.trace","params":{"overrides":{"rays_per_source":256},"threads":2,"preview_paths":8}},
            {"command":"analysis.budget"}
        ]}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute","arguments":{"command":"app.version","params":{"command":"doc.new"}}}}),
    ];
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let response = host.cli(&["mcp"], Some(&input));
    let tools = response.iter().find(|r| r["id"] == 2).unwrap()["result"]["tools"]
        .as_array()
        .unwrap();
    assert_eq!(tools.len(), 4);
    let batch = &response.iter().find(|r| r["id"] == 3).unwrap()["result"]["structuredContent"];
    assert_eq!(batch["ok"], true, "{batch}");
    assert_eq!(batch["responses"][2]["revision"], 4);
    assert_eq!(batch["responses"][3]["result"]["stale"], false);
    assert_eq!(
        response.iter().find(|r| r["id"] == 4).unwrap()["result"]["structuredContent"]["error"]["code"],
        "invalid_request"
    );
    let saved = host.path.with_file_name("shared.json");
    host.cli(&["doc.save", &format!("output={}", saved.display())], None);
    assert!(saved.exists());
    let status = host.cli(&["session.status"], None);
    assert_eq!(status[0]["result"]["dirty"], false);
    assert_eq!(status[0]["result"]["run"]["stale"], false);
    let record = host.path.with_file_name("run.json");
    // The owner retains the latest run across CLI and MCP process boundaries.
    let trace = host.cli(
        &[
            "analysis.trace",
            "overrides.rays_per_source:=256",
            "threads:=2",
            &format!("output={}", record.display()),
        ],
        None,
    );
    assert_eq!(trace[0]["result"]["launched_rays"], 512);
    let original = serde_json::from_slice::<Value>(&fs::read(&record).unwrap()).unwrap();
    let replay = host.path.with_file_name("replay.json");
    let output = Command::new(cli::executable())
        .args([
            "run.replay",
            &format!("record.path={}", record.display()),
            "threads:=2",
            &format!("output={}", replay.display()),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(replay).unwrap()).unwrap()["report"],
        original["report"]
    );
    assert!(
        fs::read_to_string(Path::new(&saved))
            .unwrap()
            .contains("Shared")
    );
}

#[test]
fn the_mcp_connect_argument_uses_the_gui_session() {
    let host = Host::start();
    let mut child = Command::new(cli::executable())
        .args(["mcp", "--connect"])
        .arg(&host.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"execute\",\"arguments\":{\"command\":\"doc.new\",\"params\":{\"name\":\"MCP\",\"expected_revision\":0}}}}\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"]["structuredContent"]["revision"], 1);
    assert_eq!(
        host.cli(&["session.status"], None)[0]["result"]["document"]["name"],
        "MCP"
    );
}

#[test]
fn a_connected_scan_reports_case_progress_without_transferring_its_model() {
    let host = Host::start();
    host.cli(
        &[
            "doc.open",
            "path=examples/sequential_lens.json",
            "expected_revision:=0",
        ],
        None,
    );
    let output = host.path.with_file_name("scan");
    let request = json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"execute","_meta":{"progressToken":"cases"},"arguments":{"command":"study.scan","params":{"plan":{"parameter":{"type":"wavelength"},"values":[0.5,0.55],"rays_per_source":64,"threads":1},"output":output}}}});
    let responses = host.cli(&["mcp"], Some(&format!("{request}\n")));
    let result = responses.iter().find(|r| r["id"] == 9).unwrap();
    assert_eq!(result["result"]["isError"], false, "{result}");
    let progress: Vec<_> = responses
        .iter()
        .filter(|r| r["method"] == "notifications/progress")
        .collect();
    assert!(!progress.is_empty());
    assert_eq!(progress.last().unwrap()["params"]["progress"], 2.0);
    assert_eq!(progress.last().unwrap()["params"]["total"], 2.0);
}
