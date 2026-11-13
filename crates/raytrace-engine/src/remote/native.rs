//! Authenticated loopback transport. Frames are length-prefixed JSON, at most 64 MiB.
//! Each TCP connection carries one command; a single `C` byte cancels that command.
use super::ConnectionInfo;
use crate::{Context, EngineError, Kind, Progress, Response, Session, registry};
use raytrace_core::trace::TraceControl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    net::{Ipv4Addr, Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const FORMAT: &str = "raytrace-connection-v1";
const MAX_FRAME: usize = 64 * 1024 * 1024;
const TICK: Duration = Duration::from_millis(25);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    request: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Progress { completed: usize, total: usize },
    Cases { completed: usize, total: usize },
    Result { response: Value },
}

fn json_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn read_frame<T: serde::de::DeserializeOwned>(stream: &mut impl Read) -> io::Result<T> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_FRAME {
        return Err(json_error("接続メッセージの上限は64 MiBです"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(json_error)
}

fn write_frame(stream: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(json_error)?;
    if bytes.len() > MAX_FRAME {
        return Err(json_error("接続メッセージの上限は64 MiBです"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()
}

fn failure(command: &str, revision: u64, error: EngineError) -> Value {
    serde_json::to_value(Response::failure(command, revision, error)).unwrap_or(Value::Null)
}

fn command(request: &Value) -> &str {
    request
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

struct Shared {
    stopped: AtomicBool,
    clients: AtomicUsize,
}

/// One authenticated request waiting for the owner of the session.
pub struct Pending {
    request: Value,
    control: Arc<TraceControl>,
    reply: SyncSender<Event>,
}

impl Pending {
    pub fn control(&self) -> Arc<TraceControl> {
        self.control.clone()
    }

    /// Run on a worker thread against a snapshot. Nothing changes in the owning session.
    pub fn prepare(self, mut session: Session) -> Prepared {
        let generation = session.generation();
        let name = command(&self.request);
        let mutation = registry()
            .get(name)
            .is_some_and(|spec| spec.kind == Kind::Mutation);
        let response = if self.control.cancel.load(Ordering::Relaxed) {
            failure(name, session.revision(), EngineError::cancelled())
        } else if mutation
            && self
                .request
                .get("expected_revision")
                .and_then(Value::as_u64)
                .is_none()
        {
            failure(
                name,
                session.revision(),
                EngineError::invalid(
                    "GUIへの変更には expected_revision が必要です。session.status で現在の版を確認してください",
                ),
            )
        } else {
            let mut progress = |progress: Progress<'_>| {
                if let Progress::Scan(manifest) = progress {
                    let completed = manifest
                        .cases
                        .iter()
                        .filter(|case| case.state == "completed" || case.state == "failed")
                        .count();
                    let _ = self.reply.try_send(Event::Cases {
                        completed,
                        total: manifest.cases.len(),
                    });
                }
            };
            let response = session.execute_request(
                self.request.clone(),
                &mut Context::with_progress(&self.control, &mut progress),
            );
            serde_json::to_value(response).unwrap_or_else(|e| {
                failure(
                    name,
                    session.revision(),
                    EngineError::new("invalid_json", e.to_string()),
                )
            })
        };
        Prepared {
            pending: self,
            session,
            generation,
            response,
        }
    }
}

/// A command's prepared state. The owning UI thread checks the lease and commits it.
pub struct Prepared {
    pending: Pending,
    session: Session,
    generation: u64,
    response: Value,
}

impl Prepared {
    /// Reply only after committing, so a successful response always describes visible state.
    /// A cancelled/disconnected call, conflict or failed command never replaces the owner.
    pub fn commit(mut self, owner: &mut Session) -> bool {
        let result = if self.pending.control.cancel.load(Ordering::Relaxed) {
            Err(EngineError::cancelled())
        } else if self.response.get("ok") == Some(&Value::Bool(true)) {
            owner.accept_prepared(self.generation, self.session)
        } else {
            let _ = self.pending.reply.send(Event::Result {
                response: self.response,
            });
            return false;
        };
        let accepted = result.is_ok();
        if let Err(error) = result {
            self.response = failure(command(&self.pending.request), owner.revision(), error);
        }
        let _ = self.pending.reply.send(Event::Result {
            response: self.response,
        });
        accepted
    }
}

/// A bounded, opt-in listener. It never owns or runs a session: the GUI consumes `Pending`s.
pub struct Server {
    path: PathBuf,
    token: String,
    shared: Arc<Shared>,
    requests: Receiver<Pending>,
    listener: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(path: &Path, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(json_error)?;
        let info = ConnectionInfo {
            format: FORMAT.into(),
            software_version: env!("CARGO_PKG_VERSION").into(),
            address: listener.local_addr()?,
            token: random.iter().map(|b| format!("{b:02x}")).collect(),
            working_directory: std::env::current_dir()?,
        };
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        if let Err(error) = serde_json::to_writer(&mut file, &info)
            .map_err(json_error)
            .and_then(|()| file.flush())
        {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(error);
        }
        drop(file);
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            clients: AtomicUsize::new(0),
        });
        let state = shared.clone();
        let token = info.token.clone();
        let (queue, requests) = mpsc::sync_channel(8);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let thread = thread::Builder::new()
            .name("gui-listener".into())
            .spawn(move || {
                while !state.stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, address)) => {
                            if !address.ip().is_loopback() {
                                let _ = stream.shutdown(Shutdown::Both);
                                continue;
                            }
                            if state.clients.fetch_add(1, Ordering::Relaxed) >= 8 {
                                state.clients.fetch_sub(1, Ordering::Relaxed);
                                let _ = stream.shutdown(Shutdown::Both);
                                continue;
                            }
                            let (state, queue, token, wake) =
                                (state.clone(), queue.clone(), token.clone(), wake.clone());
                            let release = state.clone();
                            if thread::Builder::new()
                                .name("gui-connection".into())
                                .spawn(move || {
                                    let _ = connection(stream, &token, &queue, &state, &*wake);
                                    state.clients.fetch_sub(1, Ordering::Relaxed);
                                })
                                .is_err()
                            {
                                release.clients.fetch_sub(1, Ordering::Relaxed);
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(TICK),
                        Err(_) => break,
                    }
                }
            });
        match thread {
            Ok(listener) => Ok(Self {
                path: path.into(),
                token: info.token,
                shared,
                requests,
                listener: Some(listener),
            }),
            Err(error) => {
                let _ = fs::remove_file(path);
                Err(error)
            }
        }
    }

    pub fn try_recv(&self) -> Option<Pending> {
        self.requests.try_recv().ok()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        // Do not delete a replacement connection file belonging to another process.
        if fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice::<ConnectionInfo>(&b).ok())
            .is_some_and(|info| info.token == self.token)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn connection(
    mut stream: TcpStream,
    token: &str,
    queue: &SyncSender<Pending>,
    state: &Shared,
    wake: &dyn Fn(),
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request: Request = read_frame(&mut stream)?;
    if request.token != token {
        return write_frame(
            &mut stream,
            &Event::Result {
                response: failure(
                    "unknown",
                    0,
                    EngineError::new("authentication", "GUIの認証に失敗しました"),
                ),
            },
        );
    }
    let name = command(&request.request).to_string();
    let control = Arc::new(TraceControl::default());
    let (reply, response) = mpsc::sync_channel(8);
    if queue
        .try_send(Pending {
            request: request.request,
            control: control.clone(),
            reply,
        })
        .is_err()
    {
        return write_frame(
            &mut stream,
            &Event::Result {
                response: failure(
                    &name,
                    0,
                    EngineError::new("busy", "GUIの要求待ち行列がいっぱいです"),
                ),
            },
        );
    }
    wake();
    stream.set_read_timeout(Some(TICK))?;
    let mut progress = (0, 0);
    loop {
        match response.recv_timeout(TICK) {
            Ok(event) => {
                let last = matches!(event, Event::Result { .. });
                write_frame(&mut stream, &event)?;
                if last {
                    return Ok(());
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("GUIのセッションが終了しました"));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let mut cancel = [0];
        match stream.read(&mut cancel) {
            Ok(0) => {
                control.cancel.store(true, Ordering::Relaxed);
                wake();
                return Ok(());
            }
            Ok(_) => {
                control.cancel.store(true, Ordering::Relaxed);
                wake();
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => {
                control.cancel.store(true, Ordering::Relaxed);
                return Err(e);
            }
        }
        if state.stopped.load(Ordering::Relaxed) {
            control.cancel.store(true, Ordering::Relaxed);
            return Ok(());
        }
        let next = (
            control.completed.load(Ordering::Relaxed),
            control.total.load(Ordering::Relaxed),
        );
        if next != progress {
            progress = next;
            if let Err(e) = write_frame(
                &mut stream,
                &Event::Progress {
                    completed: next.0,
                    total: next.1,
                },
            ) {
                control.cancel.store(true, Ordering::Relaxed);
                return Err(e);
            }
        }
    }
}

/// Capability read from a GUI's connection file. Only loopback endpoints and the current
/// protocol/software version are accepted. Each execute opens an independent connection.
#[derive(Clone)]
pub struct Client {
    info: ConnectionInfo,
}

impl Client {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(65_537).read_to_end(&mut bytes)?;
        if bytes.len() > 65_536 {
            return Err(json_error("接続ファイルが大きすぎます"));
        }
        let info: ConnectionInfo = serde_json::from_slice(&bytes).map_err(json_error)?;
        if info.format != FORMAT
            || info.software_version != env!("CARGO_PKG_VERSION")
            || !info.address.ip().is_loopback()
            || info.address.port() == 0
            || info.token.len() != 64
            || !info.token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(json_error(
                "接続ファイルの形式・ソフトの版・ローカル接続先を確認してください",
            ));
        }
        Ok(Self { info })
    }

    pub fn execute(&self, request: Value, context: &mut Context<'_>) -> Response {
        let name = command(&request).to_string();
        if context.control.cancel.load(Ordering::Relaxed) {
            return Response::failure(&name, 0, EngineError::cancelled());
        }
        self.exchange(request, context).unwrap_or_else(|e| {
            Response::failure(
                &name,
                0,
                EngineError::new("connection", format!("GUIとの接続に失敗しました: {e}")),
            )
        })
    }

    fn exchange(&self, request: Value, context: &mut Context<'_>) -> io::Result<Response> {
        let mut stream = TcpStream::connect_timeout(&self.info.address, Duration::from_secs(3))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        write_frame(
            &mut stream,
            &Request {
                token: self.info.token.clone(),
                request,
            },
        )?;
        let mut reader = stream.try_clone()?;
        thread::scope(|scope| {
            let (sender, events) = mpsc::sync_channel(8);
            scope.spawn(move || {
                loop {
                    let event = read_frame::<Event>(&mut reader);
                    let last = matches!(&event, Ok(Event::Result { .. }) | Err(_));
                    if sender.send(event).is_err() || last {
                        break;
                    }
                }
            });
            let mut cancelled = false;
            let result = loop {
                if !cancelled && context.control.cancel.load(Ordering::Relaxed) {
                    if let Err(e) = stream.write_all(b"C") {
                        break Err(e);
                    }
                    cancelled = true;
                }
                match events.recv_timeout(TICK) {
                    Ok(Ok(Event::Result { response })) => {
                        break serde_json::from_value(response).map_err(json_error);
                    }
                    Ok(Ok(Event::Progress { completed, total })) => {
                        context.control.total.store(total, Ordering::Relaxed);
                        context
                            .control
                            .completed
                            .store(completed, Ordering::Relaxed);
                    }
                    Ok(Ok(Event::Cases { completed, total })) => {
                        context.progress(Progress::Counters { completed, total })
                    }
                    Ok(Err(e)) => break Err(e),
                    Err(RecvTimeoutError::Disconnected) => {
                        break Err(io::Error::other("GUIからの応答が途切れました"));
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            };
            let _ = stream.shutdown(Shutdown::Both);
            // Release a reader waiting to forward progress when the peer disconnected.
            drop(events);
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    fn path() -> PathBuf {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let dir =
            std::env::temp_dir().join(format!("raytrace-remote-{:x}", u64::from_ne_bytes(random)));
        fs::create_dir_all(&dir).unwrap();
        dir.join("session.json")
    }

    fn receive_pending(server: &Server) -> Pending {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(pending) = server.try_recv() {
                return pending;
            }
            assert!(Instant::now() < until, "request did not reach owner");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn request(session: &mut Session, server: &Server, client: &Client, request: Value) -> Value {
        thread::scope(|scope| {
            let response = scope.spawn(|| {
                serde_json::to_value(
                    client.execute(request, &mut Context::new(&TraceControl::default())),
                )
                .unwrap()
            });
            receive_pending(server)
                .prepare(session.clone())
                .commit(session);
            response.join().unwrap()
        })
    }

    #[test]
    fn commands_preserve_revision_history_saved_state_and_ids() {
        let file = path();
        let server = Server::start(&file, || {}).unwrap();
        let client = Client::open(&file).unwrap();
        let mut session = Session::new();
        assert_eq!(
            request(&mut session, &server, &client, json!({"command":"doc.new"}))["error"]["code"],
            "invalid_request"
        );
        assert_eq!(
            request(
                &mut session,
                &server,
                &client,
                json!({"command":"doc.new","name":"Start","expected_revision":0})
            )["revision"],
            1
        );
        assert!(
            request(
                &mut session,
                &server,
                &client,
                json!({"command":"session.status"})
            )["ok"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(session.revision(), 1);
        let set =
            json!({"command":"doc.set","path":"/name","value":"Changed","expected_revision":1});
        assert_eq!(request(&mut session, &server, &client, set)["revision"], 2);
        assert_eq!(
            request(
                &mut session,
                &server,
                &client,
                json!({"command":"session.undo","expected_revision":2})
            )["revision"],
            3
        );
        assert_eq!(
            request(
                &mut session,
                &server,
                &client,
                json!({"command":"session.redo","expected_revision":3})
            )["revision"],
            4
        );
        let saved = file.with_file_name("design.json");
        assert_eq!(
            request(
                &mut session,
                &server,
                &client,
                json!({"command":"doc.save","output":saved})
            )["ok"],
            true
        );
        assert_eq!(session.revision(), 4);
        assert!(!session.is_dirty());
        assert_eq!(session.path(), Some(saved.as_path()));
        assert_eq!(session.log().entries.len(), 1);
        assert_eq!(
            request(
                &mut session,
                &server,
                &client,
                json!({"command":"session.undo","expected_revision":1})
            )["error"]["code"],
            "revision"
        );
        drop(server);
        assert!(!file.exists());
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn prepared_commands_cannot_overwrite_edits_undo_or_a_save_without_revision_change() {
        let file = path();
        let server = Server::start(&file, || {}).unwrap();
        let client = Client::open(&file).unwrap();
        let mut session = Session::new();
        session.execute_request(
            json!({"command":"doc.new","name":"Base"}),
            &mut Context::new(&TraceControl::default()),
        );
        for save_only in [false, true] {
            thread::scope(|scope| {
                let revision = session.revision();
                let client = &client;
                let response = scope.spawn(move || {
                    serde_json::to_value(client.execute(
                        json!({"command":"doc.new","name":"External","expected_revision":revision}),
                        &mut Context::new(&TraceControl::default()),
                    ))
                    .unwrap()
                });
                let prepared = receive_pending(&server).prepare(session.clone());
                if save_only {
                    session.mark_saved(file.with_file_name("other.json"));
                    assert_eq!(session.revision(), revision);
                } else {
                    session.execute_request(
                        json!({"command":"doc.new","name":"GUI"}),
                        &mut Context::new(&TraceControl::default()),
                    );
                }
                let before = serde_json::to_value(session.status()).unwrap();
                assert!(!prepared.commit(&mut session));
                assert_eq!(response.join().unwrap()["error"]["code"], "revision");
                assert_eq!(serde_json::to_value(session.status()).unwrap(), before);
            });
        }
        let generation = session.generation();
        let copy = session.clone();
        session.execute_request(
            json!({"command":"doc.set","path":"/name","value":"edit"}),
            &mut Context::new(&TraceControl::default()),
        );
        session.undo(None).unwrap();
        assert_eq!(
            session.accept_prepared(generation, copy).unwrap_err().code,
            "revision"
        );
        drop(server);
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn authentication_disconnect_and_late_cancellation_never_commit() {
        let file = path();
        let server = Server::start(&file, || {}).unwrap();
        let client = Client::open(&file).unwrap();
        let mut wrong = client.clone();
        wrong.info.token = "0".repeat(64);
        let response = wrong.execute(
            json!({"command":"session.status"}),
            &mut Context::new(&TraceControl::default()),
        );
        assert_eq!(response.error.unwrap().code, "authentication");
        assert!(server.try_recv().is_none());
        let mut stream = TcpStream::connect(client.info.address).unwrap();
        write_frame(
            &mut stream,
            &Request {
                token: client.info.token.clone(),
                request: json!({"command":"doc.new","expected_revision":0}),
            },
        )
        .unwrap();
        let pending = receive_pending(&server);
        stream.shutdown(Shutdown::Both).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !pending.control.cancel.load(Ordering::Relaxed) {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        let mut session = Session::new();
        assert!(!pending.prepare(session.clone()).commit(&mut session));
        assert_eq!(session.revision(), 0);
        thread::scope(|scope| {
            let response = scope.spawn(|| {
                serde_json::to_value(client.execute(
                    json!({"command":"doc.new","expected_revision":0}),
                    &mut Context::new(&TraceControl::default()),
                ))
                .unwrap()
            });
            let pending = receive_pending(&server);
            let control = pending.control();
            let prepared = pending.prepare(session.clone());
            control.cancel.store(true, Ordering::Relaxed);
            assert!(!prepared.commit(&mut session));
            assert_eq!(response.join().unwrap()["error"]["code"], "cancelled");
            assert_eq!(session.revision(), 0);
        });
        drop(server);
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn connection_files_are_private_exclusive_and_restrict_endpoints() {
        let file = path();
        let server = Server::start(&file, || {}).unwrap();
        let bytes = fs::read(&file).unwrap();
        assert!(Server::start(&file, || {}).is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let mut info: ConnectionInfo = serde_json::from_slice(&bytes).unwrap();
        info.address = "192.0.2.1:1234".parse().unwrap();
        let bad = file.with_file_name("bad.json");
        fs::write(&bad, serde_json::to_vec(&info).unwrap()).unwrap();
        assert!(Client::open(&bad).is_err());
        info.address = "127.0.0.1:1234".parse().unwrap();
        info.software_version = "old".into();
        fs::write(&bad, serde_json::to_vec(&info).unwrap()).unwrap();
        assert!(Client::open(&bad).is_err());
        fs::write(&file, b"replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(&file).unwrap(), b"replacement");
        fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn frames_reject_unknown_fields_oversize_and_truncation() {
        let mut frame = Vec::new();
        write_frame(
            &mut frame,
            &json!({"token":"secret","request":{},"extra":true}),
        )
        .unwrap();
        assert!(read_frame::<Request>(&mut frame.as_slice()).is_err());
        assert!(
            read_frame::<Request>(&mut ((MAX_FRAME as u32 + 1).to_be_bytes().as_slice())).is_err()
        );
        assert!(read_frame::<Event>(&mut b"\x00\x00\x00\x10{}".as_slice()).is_err());
    }
}
