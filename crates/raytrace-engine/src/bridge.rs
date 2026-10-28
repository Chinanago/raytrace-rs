//! An opt-in authenticated local connection to an interactive editor session.
use crate::{Error, Outcome, Request, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub format: String,
    pub address: SocketAddr,
    pub token: String,
    pub process_id: u32,
}
impl Endpoint {
    pub fn open(path: &Path) -> Result<Self> {
        let endpoint: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        if endpoint.format != "raytrace-session-v1"
            || !endpoint.address.ip().is_loopback()
            || endpoint.token.len() != 64
        {
            return Err(Error::new(
                "endpoint",
                "ローカルSessionの接続情報を確認してください",
            ));
        }
        Ok(endpoint)
    }
    pub fn apply(&self, request: Request) -> Result<Outcome> {
        let mut stream = TcpStream::connect_timeout(&self.address, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(65)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        serde_json::to_writer(
            &mut stream,
            &Envelope {
                token: self.token.clone(),
                request,
            },
        )?;
        writeln!(stream)?;
        stream.flush()?;
        let mut line = String::new();
        BufReader::new(stream)
            .take(64 * 1024 * 1024 + 1)
            .read_line(&mut line)?;
        if line.len() > 64 * 1024 * 1024 {
            return Err(Error::new(
                "response_size",
                "結果は計算記録へ保存して取得してください",
            ));
        }
        let response: Response = serde_json::from_str(&line)?;
        response.result.map_err(|e| Error::new(&e.code, e.message))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    request: Request,
}
#[derive(Serialize, Deserialize)]
struct WireError {
    code: String,
    message: String,
}
#[derive(Serialize, Deserialize)]
struct Response {
    result: std::result::Result<Outcome, WireError>,
}
pub struct Call {
    pub request: Request,
    pub reply: SyncSender<Result<Outcome>>,
    pub expires_at: std::time::Instant,
}
pub struct Bridge {
    rx: Receiver<Call>,
    stop: Arc<AtomicBool>,
    endpoint: Endpoint,
    path: PathBuf,
}
impl Bridge {
    pub fn listen(path: PathBuf, repaint: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|e| Error::new("random", e))?;
        let endpoint = Endpoint {
            format: "raytrace-session-v1".into(),
            address: listener.local_addr()?,
            token: random.iter().map(|b| format!("{b:02x}")).collect(),
            process_id: std::process::id(),
        };
        if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        serde_json::to_writer(&mut file, &endpoint)?;
        file.flush()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (tx, rx) = mpsc::sync_channel::<Call>(32);
        let token = endpoint.token.clone();
        let repaint = Arc::new(repaint);
        let active = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = connection else {
                    break;
                };
                if active.load(Ordering::Relaxed) >= 32 {
                    continue;
                }
                let tx = tx.clone();
                let token = token.clone();
                let repaint = repaint.clone();
                let active = active.clone();
                active.fetch_add(1, Ordering::Relaxed);
                std::thread::spawn(move || {
                    let result = (|| -> Result<Outcome> {
                        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                        let mut line = String::new();
                        BufReader::new(stream.try_clone()?)
                            .take(1024 * 1024 + 1)
                            .read_line(&mut line)?;
                        if line.len() > 1024 * 1024 {
                            return Err(Error::new(
                                "request_size",
                                "操作のJSONは1 MiB以内で指定してください",
                            ));
                        }
                        let envelope: Envelope = serde_json::from_str(&line)?;
                        if envelope.token != token {
                            return Err(Error::new(
                                "authentication",
                                "Sessionの接続情報が一致しません",
                            ));
                        }
                        let (reply, response) = mpsc::sync_channel(1);
                        tx.try_send(Call {
                            request: envelope.request,
                            reply,
                            expires_at: std::time::Instant::now() + Duration::from_secs(63),
                        })
                        .map_err(|e| Error::new("busy", e))?;
                        repaint();
                        response
                            .recv_timeout(Duration::from_secs(64))
                            .map_err(|e| Error::new("session_timeout", e))?
                    })();
                    let response = Response {
                        result: result.map_err(|e| WireError {
                            code: e.code,
                            message: e.message,
                        }),
                    };
                    let _ = serde_json::to_writer(&mut stream, &response);
                    let _ = writeln!(stream);
                    let _ = stream.flush();
                    active.fetch_sub(1, Ordering::Relaxed);
                });
            }
        });
        Ok(Self {
            rx,
            stop,
            endpoint,
            path,
        })
    }
    pub fn next_call(&self) -> Option<Call> {
        self.rx.try_recv().ok()
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&self.endpoint.address, Duration::from_millis(100));
        if Endpoint::open(&self.path).is_ok_and(|e| e.token == self.endpoint.token) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
