//! The MCP server (`raytrace-rs mcp`) over a real stdio pipe.
// Tests and examples may panic on failure; the no-panic rule (DESIGN.md P8) covers product code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "support/cli.rs"]
mod cli_support;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(60);

/// A running server with line-oriented access to its stdio.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    /// Messages read while waiting for another one (notifications).
    unclaimed: Vec<Value>,
    next_id: u64,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(cli_support::executable())
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            unclaimed: Vec::new(),
            next_id: 1,
        }
    }

    fn send_raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    fn send(&mut self, message: Value) {
        self.send_raw(&message.to_string());
    }

    /// The next message of the server, parsed.
    fn read(&mut self, timeout: Duration) -> Option<Value> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Some(serde_json::from_str(&line).unwrap_or_else(|e| {
                panic!("stdout carries only JSON-RPC lines, got {line:?}: {e}")
            })),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Send a request and return its id.
    fn start_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// The message that answers `id`; other messages are kept in `unclaimed`.
    fn wait_for(&mut self, id: u64, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self
                .read(left)
                .unwrap_or_else(|| panic!("no answer to request {id} in {timeout:?}"));
            if message["id"] == json!(id) {
                return message;
            }
            self.unclaimed.push(message);
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.start_request(method, params);
        self.wait_for(id, TIMEOUT)
    }

    /// Call a tool; the result object (not the JSON-RPC envelope).
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let answer = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        assert!(answer.get("error").is_none(), "{answer}");
        answer["result"].clone()
    }

    fn execute(&mut self, command: &str, params: Value) -> Value {
        self.tool("execute", json!({"command": command, "params": params}))
    }

    fn initialize(&mut self) {
        let answer = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    /// Close stdin and wait for the exit status.
    fn finish(mut self) -> Option<i32> {
        drop(self.stdin.take());
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            assert!(Instant::now() < deadline, "the server did not exit");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A failed test must not leave the server running.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn structured(result: &Value) -> &Value {
    &result["structuredContent"]
}

/// The text block is the same JSON as `structuredContent`.
fn assert_text_matches(result: &Value) {
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        &serde_json::from_str::<Value>(text).unwrap(),
        structured(result)
    );
}

#[test]
fn a_session_over_stdio() {
    let rod = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/integrator_rod.json");
    let mut server = Server::start();

    // Version negotiation: the client's version, or the newest for an unknown one.
    for (asked, answered) in [
        ("2024-11-05", "2024-11-05"),
        ("2025-03-26", "2025-03-26"),
        ("2099-01-01", "2025-06-18"),
    ] {
        let answer = server.request(
            "initialize",
            json!({"protocolVersion": asked, "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        );
        let result = &answer["result"];
        assert_eq!(answer["jsonrpc"], "2.0");
        assert_eq!(result["protocolVersion"], answered, "{asked}");
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(result["serverInfo"]["name"], "raytrace-rs");
        let instructions = result["instructions"].as_str().unwrap();
        for word in [
            "list_commands",
            "describe_command",
            "doc.open",
            "doc.inspect",
        ] {
            assert!(instructions.contains(word), "{word}");
        }
    }
    let bad = server.request("initialize", json!({"capabilities": {}}));
    assert_eq!(bad["error"]["code"], -32602);
    server.initialize();

    assert_eq!(server.request("ping", json!({}))["result"], json!({}));

    // Exactly four tools, each with an input schema.
    let listed = server.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["list_commands", "describe_command", "execute", "batch"]
    );
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    for name in ["execute", "batch"] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        assert!(
            tool["outputSchema"]["properties"]["ok"].is_object(),
            "{name}"
        );
    }

    let listed = server.tool("list_commands", json!({}));
    assert_eq!(listed["isError"], false);
    assert_text_matches(&listed);
    let commands = structured(&listed)["commands"].as_array().unwrap();
    let ids: Vec<_> = commands.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for id in [
        "analysis.trace",
        "analysis.budget",
        "analysis.screen",
        "doc.open",
        "doc.set",
        "session.undo",
        "study.scan",
    ] {
        assert!(ids.contains(&id), "{id}");
    }

    let described = server.tool("describe_command", json!({"command": "analysis.trace"}));
    assert_eq!(described["isError"], false);
    let description = structured(&described);
    assert_eq!(description["id"], "analysis.trace");
    assert_eq!(description["kind"], "job");
    assert!(description["input_schema"]["properties"]["overrides"].is_object());
    assert!(description["output_schema"].is_object());
    let unknown = server.tool("describe_command", json!({"command": "doc.opne"}));
    assert_eq!(unknown["isError"], true);
    assert_eq!(structured(&unknown)["error"]["code"], "invalid_request");

    // doc.open -> doc.set -> analysis.trace without a model -> session.undo.
    let opened = server.execute("doc.open", json!({"path": rod}));
    assert_eq!(opened["isError"], false, "{opened}");
    assert_text_matches(&opened);
    assert_eq!(structured(&opened)["ok"], true);
    assert_eq!(structured(&opened)["command"], "doc.open");
    let revision = structured(&opened)["revision"].as_u64().unwrap();

    let edited = server.execute(
        "doc.set",
        json!({"path": "/parts/0/scene/sources/0/count", "value": 8}),
    );
    assert_eq!(edited["isError"], false, "{edited}");
    assert_eq!(structured(&edited)["revision"], revision + 1);

    let traced = server.execute("analysis.trace", json!({}));
    assert_eq!(traced["isError"], false, "{traced}");
    assert_eq!(structured(&traced)["ok"], true);
    assert_eq!(structured(&traced)["result"]["launched_rays"], 2048 + 8);

    let undone = server.execute("session.undo", json!({}));
    assert_eq!(undone["isError"], false, "{undone}");
    assert_eq!(structured(&undone)["revision"], revision + 2);
    let traced = server.execute("analysis.trace", json!({}));
    assert_eq!(structured(&traced)["result"]["launched_rays"], 4096);

    // A batch stops at the first failure and reports it.
    let batch = server.tool(
        "batch",
        json!({"requests": [
            {"command": "app.version"},
            {"command": "doc.set", "params": {"path": "/nope", "value": 1}},
            {"command": "app.version"},
        ]}),
    );
    assert_eq!(batch["isError"], true);
    assert_text_matches(&batch);
    let output = structured(&batch);
    assert_eq!(
        (output["executed"].as_u64(), output["requested"].as_u64()),
        (Some(2), Some(3))
    );
    assert_eq!(output["ok"], false);
    let responses = output["responses"].as_array().unwrap();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], false);
    // Without stop_on_error every request runs.
    let batch = server.tool(
        "batch",
        json!({"stop_on_error": false, "requests": [
            {"command": "nope.nope"},
            {"command": "app.version"},
        ]}),
    );
    assert_eq!(structured(&batch)["responses"].as_array().unwrap().len(), 2);
    assert_eq!(batch["isError"], true);
    let batch = server.tool("batch", json!({"requests": [{"command": "app.version"}]}));
    assert_eq!(batch["isError"], false);
    assert_eq!(structured(&batch)["ok"], true);

    // An unknown command is a failed tool result, not a protocol error.
    let unknown = server.execute("doc.opne", json!({}));
    assert_eq!(unknown["isError"], true);
    assert_eq!(structured(&unknown)["ok"], false);
    assert_eq!(structured(&unknown)["error"]["code"], "invalid_request");

    // Protocol errors.
    let wrong = server.request("tools/call", json!({"name": "nope", "arguments": {}}));
    assert_eq!(wrong["error"]["code"], -32602);
    let wrong = server.request("tools/call", json!({"name": "execute", "arguments": {}}));
    assert_eq!(wrong["error"]["code"], -32602);
    let wrong = server.request(
        "tools/call",
        json!({"name": "execute", "arguments": {"command": "a.b", "params": 3}}),
    );
    assert_eq!(wrong["error"]["code"], -32602);
    server.send_raw("{this is not json");
    let parse = server.read(TIMEOUT).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);
    let unknown = server.request("resources/list", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);
    server.send_raw("[1]");
    assert_eq!(server.read(TIMEOUT).unwrap()["error"]["code"], -32600);
    // The server still answers after the errors.
    assert_eq!(server.request("ping", json!({}))["result"], json!({}));

    // Nothing but responses came back for the notifications.
    assert!(server.unclaimed.is_empty(), "{:?}", server.unclaimed);

    // Closing stdin ends the server normally.
    assert_eq!(server.finish(), Some(0));
}

#[test]
fn a_running_trace_can_be_cancelled() {
    let rod = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/integrator_rod.json");
    let mut server = Server::start();
    server.initialize();
    let opened = server.execute("doc.open", json!({"path": rod}));
    assert_eq!(opened["isError"], false, "{opened}");

    let trace = server.start_request(
        "tools/call",
        json!({
            "name": "execute",
            "arguments": {
                "command": "analysis.trace",
                "params": {"overrides": {"rays_per_source": 3_000_000, "max_rays_per_source": 3_000_000}, "max_total_rays": 10_000_000}
            },
            "_meta": {"progressToken": "trace-1"}
        }),
    );
    // Calls queued behind it wait their turn, but the server keeps answering `ping`.
    let queued = server.start_request(
        "tools/call",
        json!({"name": "execute", "arguments": {"command": "app.version"}}),
    );
    let pinged = server.start_request("ping", json!({}));
    assert_eq!(
        server.wait_for(pinged, Duration::from_secs(10))["result"],
        json!({})
    );

    // Wait for the first progress report (or for a moment) so that the trace is running.
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5)
        && !server
            .unclaimed
            .iter()
            .any(|m| m["method"] == "notifications/progress")
    {
        if let Some(message) = server.read(Duration::from_millis(100)) {
            server.unclaimed.push(message);
        }
    }
    assert!(
        server.unclaimed.iter().all(|m| m.get("id").is_none()),
        "the queued call must not answer before the trace: {:?}",
        server.unclaimed
    );
    server.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": {"requestId": trace, "reason": "test"}
    }));
    let cancelled_at = Instant::now();
    let answer = server.wait_for(trace, Duration::from_secs(20));
    assert!(cancelled_at.elapsed() < Duration::from_secs(20));
    let result = &answer["result"];
    assert_eq!(result["isError"], true, "{answer}");
    assert_eq!(structured(result)["ok"], false);
    assert_eq!(structured(result)["error"]["code"], "cancelled");

    // Progress notifications name the token, rise and carry a total.
    let progress: Vec<&Value> = server
        .unclaimed
        .iter()
        .filter(|m| m["method"] == "notifications/progress")
        .collect();
    let mut last = -1.0;
    for message in &progress {
        let params = &message["params"];
        assert_eq!(params["progressToken"], "trace-1");
        let value = params["progress"].as_f64().unwrap();
        assert!(value > last, "progress must increase: {progress:?}");
        assert!(value <= params["total"].as_f64().unwrap());
        last = value;
    }

    // The queued call ran afterwards, and the session still works.
    let version = server.wait_for(queued, Duration::from_secs(20));
    assert_eq!(version["result"]["isError"], false);
    let version = server.execute("app.version", json!({}));
    assert_eq!(version["isError"], false);
    assert_eq!(server.finish(), Some(0));
}

#[test]
fn a_queued_call_that_is_cancelled_does_not_run() {
    let rod = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/integrator_rod.json");
    let mut server = Server::start();
    server.initialize();
    server.execute("doc.open", json!({"path": rod}));
    let trace = server.start_request(
        "tools/call",
        json!({"name": "execute", "arguments": {
            "command": "analysis.trace",
            "params": {"overrides": {"rays_per_source": 3_000_000, "max_rays_per_source": 3_000_000}, "max_total_rays": 10_000_000}
        }}),
    );
    let edit = server.start_request(
        "tools/call",
        json!({"name": "execute", "arguments": {
            "command": "doc.set",
            "params": {"path": "/name", "value": "never applied"}
        }}),
    );
    server.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": edit}}));
    server.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": trace}}));
    let answer = server.wait_for(trace, Duration::from_secs(20));
    assert_eq!(structured(&answer["result"])["error"]["code"], "cancelled");
    let answer = server.wait_for(edit, Duration::from_secs(20));
    assert_eq!(structured(&answer["result"])["error"]["code"], "cancelled");
    // The edit never ran: the log holds only the opening.
    let log = server.execute("session.log", json!({}));
    let text = log["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("never applied"), "{text}");
}

#[test]
fn help_prints_a_usage_and_other_arguments_are_rejected() {
    let help = Command::new(cli_support::executable())
        .args(["mcp", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("claude mcp add raytrace-rs -- raytrace-rs mcp"));
    let bad = Command::new(cli_support::executable())
        .args(["mcp", "--bogus"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1));
    assert!(bad.stdout.is_empty());
}
