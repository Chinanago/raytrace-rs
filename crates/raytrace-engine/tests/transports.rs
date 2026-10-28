use raytrace_engine::{
    Request, Session,
    bridge::{Bridge, Endpoint},
    mcp,
};
use raytrace_io::project::Project;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/transport_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("session.json")
}
#[test]
fn mcp_negotiates_tools_and_returns_structured_success_and_errors() {
    let messages = [
        json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"raytrace_command","arguments":{"command":"set_property","target":"project","change":{"property":"name","value":"MCP"}}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"raytrace_command","arguments":{"command":"query","query":{"query":"project"}}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"raytrace_command","arguments":{"expected_revision":0,"command":"undo"}}}),
        json!({"jsonrpc":"2.0","id":6,"method":"unknown"}),
    ];
    let text = messages
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let mut session = Session::new(Project::new_assembly()).unwrap();
    let mut out = Vec::new();
    mcp::serve(text.as_bytes(), &mut out, |r| session.apply(r)).unwrap();
    let responses: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 7);
    assert_eq!(responses[0]["error"]["code"], -32002);
    assert_eq!(responses[1]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        responses[2]["result"]["tools"][0]["name"],
        "raytrace_command"
    );
    assert!(responses[2]["result"]["tools"][0]["inputSchema"]["$defs"].is_object());
    assert_eq!(responses[3]["result"]["isError"], false);
    assert_eq!(
        responses[4]["result"]["structuredContent"]["result"]["data"]["name"],
        "MCP"
    );
    assert_eq!(responses[5]["result"]["isError"], true);
    assert_eq!(
        responses[5]["result"]["structuredContent"]["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(responses[6]["error"]["code"], -32601);
}
#[test]
fn invalid_mcp_json_does_not_corrupt_the_next_request() {
    let text = b"bad json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
    let mut out = Vec::new();
    mcp::serve(&text[..], &mut out, |_| panic!("no model command")).unwrap();
    let values: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(values[0]["error"]["code"], -32700);
    assert_eq!(values[1]["result"], json!({}));
}
#[test]
fn authenticated_bridge_roundtrips_to_one_session_and_removes_its_endpoint() {
    let path = path();
    let bridge = Bridge::listen(path.clone(), || {}).unwrap();
    let endpoint = Endpoint::open(&path).unwrap();
    assert!(endpoint.address.ip().is_loopback());
    let worker = std::thread::spawn(move || {
        endpoint.apply(serde_json::from_value::<Request>(json!({"command":"set_property","target":"project","change":{"property":"name","value":"connected"}})).unwrap())
    });
    let mut session = Session::new(Project::new_assembly()).unwrap();
    let start = Instant::now();
    loop {
        if let Some(call) = bridge.next_call() {
            call.reply.send(session.apply(call.request)).unwrap();
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(worker.join().unwrap().unwrap().changed);
    assert_eq!(session.project().scene.name, "connected");
    drop(bridge);
    assert!(!path.exists());
}
#[test]
fn bridge_rejects_bad_tokens_before_dispatch_and_refuses_to_overwrite_an_endpoint() {
    let path = path();
    let bridge = Bridge::listen(path.clone(), || {}).unwrap();
    assert!(Bridge::listen(path.clone(), || {}).is_err());
    let endpoint = Endpoint::open(&path).unwrap();
    let mut stream = TcpStream::connect(endpoint.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    writeln!(
        stream,
        "{}",
        json!({"token":"bad","request":{"command":"undo"}})
    )
    .unwrap();
    stream.flush().unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["result"]["Err"]["code"], "authentication");
    assert!(bridge.next_call().is_none());
}
