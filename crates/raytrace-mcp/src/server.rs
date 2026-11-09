//! The server loop: the reading thread answers protocol requests at once and queues tool calls
//! that need the session; one worker thread owns the [`Session`] and runs them one at a time.
use crate::protocol::{
    self, Incoming, RpcError, STRUCTURED_OUTPUT_VERSION, SUPPORTED_VERSIONS, code,
};
use crate::tools::{self, BatchOutput, Call, Request};
use raytrace_core::trace::TraceControl;
use raytrace_engine::{Context, EngineError, Progress, Response, Session, registry};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::Ordering,
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

/// The longest message line the server reads; a longer one is rejected.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// How often the ray progress of a single trace is sampled.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The output stream shared by the reading thread, the worker and the progress sampler. A
/// message is written and flushed as one line.
struct Output {
    writer: Mutex<Box<dyn Write + Send>>,
}
impl Output {
    fn send(&self, message: &Value) {
        let mut line = match serde_json::to_string(message) {
            Ok(line) => line,
            Err(e) => {
                eprintln!("MCP: メッセージをJSONにできません: {e}");
                return;
            }
        };
        line.push('\n');
        let mut writer = lock(&self.writer);
        if let Err(e) = writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.flush())
        {
            eprintln!("MCP: 出力に書き込めません: {e}");
        }
    }
}

/// Tool calls that are queued, running or just done, by request id, so that
/// `notifications/cancelled` can reach their [`TraceControl`].
#[derive(Default)]
struct InFlight {
    jobs: Mutex<Vec<(Value, Arc<TraceControl>)>>,
}
impl InFlight {
    fn add(&self, id: &Value, control: &Arc<TraceControl>) {
        lock(&self.jobs).push((id.clone(), Arc::clone(control)));
    }
    fn remove(&self, id: &Value) {
        lock(&self.jobs).retain(|(other, _)| other != id);
    }
    fn cancel(&self, id: &Value) {
        for (other, control) in lock(&self.jobs).iter() {
            if other == id {
                control.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}

/// A tool call for the worker.
struct Job {
    id: Value,
    call: Call,
    progress_token: Option<Value>,
    control: Arc<TraceControl>,
}

/// Sends `notifications/progress` for one tool call. In a batch the progress is the index of
/// the running request plus its fraction, so it grows over the whole call. Values never go
/// down.
struct ProgressSink<'a> {
    output: &'a Output,
    token: &'a Value,
    index: usize,
    count: usize,
    last: Mutex<Option<f64>>,
}
impl ProgressSink<'_> {
    fn report(&self, completed: usize, total: usize) {
        let (progress, total) = if self.count <= 1 {
            (completed as f64, total as f64)
        } else {
            (
                self.index as f64 + completed as f64 / total.max(1) as f64,
                self.count as f64,
            )
        };
        {
            let mut last = lock(&self.last);
            if last.is_some_and(|last| progress <= last) {
                return;
            }
            *last = Some(progress);
        }
        self.output.send(&protocol::notification_message(
            "notifications/progress",
            json!({"progressToken": self.token, "progress": progress, "total": total}),
        ));
    }
}

/// Report the ray progress of the running trace until `stopped` closes.
fn poll_progress(control: &TraceControl, sink: &ProgressSink<'_>, stopped: mpsc::Receiver<()>) {
    let mut sent = (0, 0);
    while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(POLL_INTERVAL) {
        let completed = control.completed.load(Ordering::Relaxed);
        let total = control.total.load(Ordering::Relaxed);
        if total > 0 && (completed, total) != sent {
            sent = (completed, total);
            sink.report(completed, total);
        }
    }
}

/// Run one command. A cancelled call does not start further commands.
fn run_request(
    session: &mut Session,
    request: &Request,
    control: &TraceControl,
    sink: Option<&ProgressSink<'_>>,
) -> Response {
    if control.cancel.load(Ordering::Relaxed) {
        return Response::failure(
            &request.command,
            session.revision(),
            EngineError::cancelled(),
        );
    }
    let mut on_progress = |progress: Progress<'_>| {
        if let (Some(sink), Progress::Scan(manifest)) = (sink, progress) {
            let done = manifest
                .cases
                .iter()
                .filter(|c| c.state == "completed" || c.state == "failed")
                .count();
            sink.report(done, manifest.cases.len());
        }
    };
    // A scan reports through the callback; a single trace is sampled from its control.
    let sample = sink.is_some()
        && registry()
            .get(&request.command)
            .is_some_and(|spec| spec.id == "analysis.trace");
    thread::scope(|scope| {
        let (stop, stopped) = mpsc::channel::<()>();
        if let (true, Some(sink)) = (sample, sink) {
            scope.spawn(move || poll_progress(control, sink, stopped));
        }
        let mut context = Context::with_progress(control, &mut on_progress);
        let response = session.execute(&request.command, request.params.clone(), &mut context);
        drop(stop);
        response
    })
}

/// Run a queued tool call and build its result object.
fn run_job(session: &mut Session, job: &Job, output: &Output) -> Value {
    let sink = |index: usize, count: usize| {
        job.progress_token.as_ref().map(|token| ProgressSink {
            output,
            token,
            index,
            count,
            last: Mutex::new(None),
        })
    };
    match &job.call {
        Call::Execute(request) => {
            let sink = sink(0, 1);
            let response = run_request(session, request, &job.control, sink.as_ref());
            tools::response_result(&response)
        }
        Call::Batch {
            requests,
            stop_on_error,
        } => {
            let mut responses = Vec::with_capacity(requests.len());
            for (index, request) in requests.iter().enumerate() {
                let sink = sink(index, requests.len());
                let response = run_request(session, request, &job.control, sink.as_ref());
                let failed = !response.ok;
                responses.push(response);
                if failed && *stop_on_error {
                    break;
                }
            }
            tools::batch_result(&BatchOutput {
                ok: responses.len() == requests.len() && responses.iter().all(|r| r.ok),
                executed: responses.len(),
                requested: requests.len(),
                responses,
            })
        }
        // Answered by the reading thread; never queued.
        Call::ListCommands | Call::DescribeCommand(_) => {
            tools::tool_error("internal", "内部エラー: ジョブの種類が不正です")
        }
    }
}

/// The worker: runs the queued calls in order until the queue closes.
fn worker(jobs: mpsc::Receiver<Job>, output: &Output, inflight: &InFlight) {
    let mut session = Session::new();
    for job in jobs {
        let result = panic::catch_unwind(AssertUnwindSafe(|| run_job(&mut session, &job, output)))
            .unwrap_or_else(|_| {
                tools::tool_error("internal", "内部エラー: コマンドの実行中に異常が起きました")
            });
        inflight.remove(&job.id);
        output.send(&protocol::result_message(&job.id, result));
    }
}

/// What the reading thread knows between messages.
struct Dispatcher<'a> {
    output: &'a Output,
    inflight: &'a InFlight,
    queue: mpsc::Sender<Job>,
    /// The protocol version of the last `initialize`; the newest before one.
    version: &'static str,
}

impl Dispatcher<'_> {
    fn handle_line(&mut self, line: &[u8]) {
        match protocol::parse(line) {
            Err((id, error)) => self.output.send(&protocol::error_message(&id, &error)),
            Ok(Incoming::Response) => {}
            Ok(Incoming::Notification { method, params }) => self.notification(&method, params),
            Ok(Incoming::Request { id, method, params }) => {
                if let Some(result) = self.request(&id, &method, params).transpose() {
                    self.output.send(&match result {
                        Ok(result) => protocol::result_message(&id, result),
                        Err(error) => protocol::error_message(&id, &error),
                    });
                }
            }
        }
    }

    fn notification(&self, method: &str, params: Value) {
        if method == "notifications/cancelled"
            && let Some(id) = params.get("requestId")
        {
            self.inflight.cancel(id);
        }
        // `notifications/initialized` and unknown notifications need no action.
    }

    /// The result of a request; `None` when the worker answers later.
    fn request(
        &mut self,
        id: &Value,
        method: &str,
        params: Value,
    ) -> Result<Option<Value>, RpcError> {
        match method {
            "initialize" => self.initialize(params).map(Some),
            "ping" => Ok(Some(json!({}))),
            "tools/list" => Ok(Some(json!({
                "tools": tools::definitions(self.version >= STRUCTURED_OUTPUT_VERSION)
            }))),
            "tools/call" => self.call_tool(id, params),
            other => Err(RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
            )),
        }
    }

    fn initialize(&mut self, params: Value) -> Result<Value, RpcError> {
        let params = protocol::object_params(params)?;
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("`protocolVersion` must be a string"))?;
        self.version = protocol::negotiate(requested);
        Ok(json!({
            "protocolVersion": self.version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "raytrace-rs", "version": env!("CARGO_PKG_VERSION")},
            "instructions": protocol::INSTRUCTIONS,
        }))
    }

    fn call_tool(&self, id: &Value, params: Value) -> Result<Option<Value>, RpcError> {
        let mut params = protocol::object_params(params)?;
        let name = params
            .remove("name")
            .and_then(|name| name.as_str().map(str::to_string))
            .ok_or_else(|| RpcError::invalid_params("`name` must be the name of a tool"))?;
        let arguments = protocol::object_params(params.remove("arguments").unwrap_or(Value::Null))
            .map_err(|_| RpcError::invalid_params("`arguments` must be an object"))?;
        let progress_token = params
            .get("_meta")
            .and_then(|meta| meta.get("progressToken"))
            .filter(|token| token.is_string() || token.is_number())
            .cloned();
        match tools::parse_call(&name, &arguments)? {
            Call::ListCommands => Ok(Some(tools::list_commands())),
            Call::DescribeCommand(command) => Ok(Some(tools::describe_command(&command))),
            call => {
                let control = Arc::new(TraceControl::default());
                self.inflight.add(id, &control);
                let job = Job {
                    id: id.clone(),
                    call,
                    progress_token,
                    control,
                };
                if self.queue.send(job).is_err() {
                    self.inflight.remove(id);
                    return Err(RpcError::new(
                        code::INTERNAL_ERROR,
                        "Internal error: the worker thread has stopped",
                    ));
                }
                Ok(None)
            }
        }
    }
}

/// The next line from `input` into `buffer` (without the line break).
enum Line {
    End,
    Text,
    TooLong,
}

fn read_line<R: BufRead>(input: &mut R, buffer: &mut Vec<u8>) -> io::Result<Line> {
    buffer.clear();
    let limit = MAX_LINE_BYTES as u64 + 1;
    let read = input.by_ref().take(limit).read_until(b'\n', buffer)?;
    if read == 0 {
        return Ok(Line::End);
    }
    if buffer.last() == Some(&b'\n') {
        buffer.pop();
        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        return Ok(Line::Text);
    }
    if buffer.len() > MAX_LINE_BYTES {
        // Skip the rest of the line so that the next message is read from its start.
        loop {
            let chunk = input.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            match chunk.iter().position(|b| *b == b'\n') {
                Some(end) => {
                    input.consume(end + 1);
                    break;
                }
                None => {
                    let length = chunk.len();
                    input.consume(length);
                }
            }
        }
        buffer.clear();
        return Ok(Line::TooLong);
    }
    Ok(Line::Text)
}

/// Serve the MCP protocol on `input` and `output` until `input` ends, then finish the queued
/// calls and return.
///
/// One message per line. Tool calls that run commands are executed one at a time in order on
/// a worker thread that owns the session; the reading thread meanwhile answers `ping`,
/// `tools/list`, `list_commands`, `describe_command` and `notifications/cancelled`.
pub fn serve<R, W>(mut input: R, output: W) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let output = Output {
        writer: Mutex::new(Box::new(output)),
    };
    let inflight = InFlight::default();
    let (queue, jobs) = mpsc::channel::<Job>();
    thread::scope(|scope| {
        let worker_thread = thread::Builder::new()
            .name("mcp-worker".into())
            .spawn_scoped(scope, || worker(jobs, &output, &inflight))?;
        let mut dispatcher = Dispatcher {
            output: &output,
            inflight: &inflight,
            queue,
            version: SUPPORTED_VERSIONS[0],
        };
        let mut buffer = Vec::new();
        let outcome = loop {
            match read_line(&mut input, &mut buffer) {
                Ok(Line::End) => break Ok(()),
                Ok(Line::Text) => {
                    if !buffer.iter().all(u8::is_ascii_whitespace) {
                        dispatcher.handle_line(&buffer);
                    }
                }
                Ok(Line::TooLong) => output.send(&protocol::error_message(
                    &Value::Null,
                    &RpcError::new(
                        code::INVALID_REQUEST,
                        format!("Invalid request: a message is longer than {MAX_LINE_BYTES} bytes"),
                    ),
                )),
                Err(e) => break Err(e),
            }
        };
        // Closing the queue lets the worker finish what was asked and stop.
        drop(dispatcher);
        if worker_thread.join().is_err() {
            eprintln!("MCP: 作業スレッドが異常終了しました");
        }
        outcome
    })
}

/// [`serve`] on the standard input and output.
pub fn serve_stdio() -> io::Result<()> {
    serve(io::stdin().lock(), io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Serve the lines to the end and parse every output line.
    fn run(lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let shared = Shared::default();
        serve(input.as_bytes(), shared.clone()).unwrap();
        let bytes = lock(&shared.0).clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn call(id: u64, name: &str, arguments: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
    }

    fn answer(outputs: &[Value], id: u64) -> &Value {
        outputs.iter().find(|m| m["id"] == json!(id)).unwrap()
    }

    #[test]
    fn handshake_and_listing() {
        let outputs = run(&[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}),
            json!({"jsonrpc": "2.0", "id": 4, "method": "nope"}),
        ]);
        assert_eq!(outputs.len(), 4, "a notification has no answer");
        assert_eq!(
            answer(&outputs, 1)["result"]["protocolVersion"],
            "2024-11-05"
        );
        let tools = answer(&outputs, 2)["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 4);
        // The older protocol has no output schemas.
        assert!(tools.iter().all(|t| t.get("outputSchema").is_none()));
        assert_eq!(answer(&outputs, 3)["result"], json!({}));
        assert_eq!(answer(&outputs, 4)["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn tools_run_in_one_session_in_order() {
        let outputs = run(&[
            call(1, "execute", json!({"command": "session.undo"})),
            call(
                2,
                "batch",
                json!({"requests": [{"command": "app.version"}, {"command": "zzz.zzz"}, {"command": "app.version"}]}),
            ),
            call(3, "list_commands", json!({})),
            call(4, "execute", json!({"command": "app.version"})),
        ]);
        let undo = &answer(&outputs, 1)["result"];
        assert_eq!(undo["isError"], true);
        assert_eq!(undo["structuredContent"]["error"]["code"], "history");
        let batch = &answer(&outputs, 2)["result"]["structuredContent"];
        assert_eq!(batch["executed"], 2);
        assert_eq!(batch["ok"], false);
        assert_eq!(answer(&outputs, 3)["result"]["isError"], false);
        let version = &answer(&outputs, 4)["result"]["structuredContent"];
        assert_eq!(version["result"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn hostile_lines_get_errors_and_never_stop_the_server() {
        let shared = Shared::default();
        let mut input: Vec<u8> = Vec::new();
        for part in [
            &b"\0\n\n"[..],
            &br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":5}"#[..],
            &b"\r\n"[..],
            &br#"{"id":2,"method":"tools/call","params":{"name":7}}"#[..],
            &b"\n\xff\xfe\n"[..],
            &br#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#[..],
        ] {
            input.extend_from_slice(part);
        }
        serve(&input[..], shared.clone()).unwrap();
        let text = String::from_utf8(lock(&shared.0).clone()).unwrap();
        let outputs: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let codes: Vec<_> = outputs
            .iter()
            .map(|m| m["error"]["code"].as_i64())
            .collect();
        assert_eq!(
            codes,
            [
                Some(code::PARSE_ERROR),
                Some(code::INVALID_PARAMS),
                Some(code::INVALID_PARAMS),
                Some(code::PARSE_ERROR),
                None
            ]
        );
        assert_eq!(outputs[4]["result"], json!({}));
    }

    #[test]
    fn progress_only_goes_up() {
        let shared = Shared::default();
        let output = Output {
            writer: Mutex::new(Box::new(shared.clone())),
        };
        let token = json!("t");
        let sink = ProgressSink {
            output: &output,
            token: &token,
            index: 1,
            count: 4,
            last: Mutex::new(None),
        };
        sink.report(0, 10);
        sink.report(5, 10);
        sink.report(3, 10);
        sink.report(5, 10);
        let text = String::from_utf8(lock(&shared.0).clone()).unwrap();
        let progress: Vec<f64> = text
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["params"]["progress"]
                    .as_f64()
                    .unwrap()
            })
            .collect();
        assert_eq!(progress, [1.0, 1.5]);
    }

    #[test]
    fn lines_are_split_at_line_breaks() {
        let mut input: &[u8] = b"one\r\ntwo\nthree";
        let mut buffer = Vec::new();
        let mut lines = Vec::new();
        while let Ok(Line::Text) = read_line(&mut input, &mut buffer) {
            lines.push(String::from_utf8(buffer.clone()).unwrap());
        }
        assert_eq!(lines, ["one", "two", "three"]);
    }

    #[test]
    fn cancelling_reaches_queued_calls_only_by_id() {
        let inflight = InFlight::default();
        let (a, b) = (
            Arc::new(TraceControl::default()),
            Arc::new(TraceControl::default()),
        );
        inflight.add(&json!(1), &a);
        inflight.add(&json!("x"), &b);
        inflight.cancel(&json!("x"));
        inflight.cancel(&json!(99));
        assert!(!a.cancel.load(Ordering::Relaxed));
        assert!(b.cancel.load(Ordering::Relaxed));
        inflight.remove(&json!("x"));
        inflight.cancel(&json!("x"));
        assert_eq!(lock(&inflight.jobs).len(), 1);
    }
}
