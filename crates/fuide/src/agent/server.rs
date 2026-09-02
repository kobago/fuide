//! Unix-socket MCP server that lives inside the app. Each client connection gets a thread that
//! reads JSON-RPC lines, forwards tool calls to the UI thread (a channel + repaint request)
//! and writes the answers back. Stopped by dropping the [`Server`] (the settings toggle).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::{mcp, Command, Reply, Request};

/// How long a tool call may take before the client gets an error (brew runs are async in the
/// app, so this only covers the UI thread being unresponsive).
const CALL_TIMEOUT: Duration = Duration::from_secs(90);

/// Where the socket for `app_id` lives: next to the settings file
/// (`~/Library/Application Support/FUIDE/<app_id>.sock` on macOS, `FUIDE_CONFIG_DIR` override),
/// or the temp dir when that is unavailable or too long for a socket address.
pub fn socket_path(app_id: &str) -> PathBuf {
    let name = format!("{app_id}.sock");
    let preferred = crate::Settings::path(app_id).and_then(|p| p.parent().map(|d| d.join(&name)));
    match preferred {
        // sockaddr_un holds ~104 bytes on macOS
        Some(p) if p.as_os_str().len() < 100 => p,
        _ => std::env::temp_dir().join(format!("fuide-{name}")),
    }
}

/// Shared with the UI thread: what the settings window shows.
#[derive(Default)]
pub struct Stats {
    pub clients: AtomicUsize,
    pub calls: AtomicUsize,
}

pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    pub stats: Arc<Stats>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Bind `path` (a stale file from a crashed instance is removed) and start accepting.
    /// `wake` is called after a call is queued so a sleeping UI thread picks it up.
    pub fn start(
        path: PathBuf,
        tx: Sender<Request>,
        pending: Arc<AtomicUsize>,
        wake: impl Fn() + Send + Sync + 'static,
        server_name: String,
        instructions: String,
    ) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Stats::default());
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let thread = {
            let stop = Arc::clone(&stop);
            let stats = Arc::clone(&stats);
            std::thread::Builder::new()
                .name("fuide-agent-listener".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let conn = Conn {
                                    tx: tx.clone(),
                                    pending: Arc::clone(&pending),
                                    wake: Arc::clone(&wake),
                                    stop: Arc::clone(&stop),
                                    stats: Arc::clone(&stats),
                                    server_name: server_name.clone(),
                                    instructions: instructions.clone(),
                                };
                                let _ = std::thread::Builder::new()
                                    .name("fuide-agent-conn".into())
                                    .spawn(move || conn.serve(stream));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            Err(_) => std::thread::sleep(Duration::from_millis(250)),
                        }
                    }
                })?
        };
        log::info!("agent: listening on {}", path.display());
        Ok(Self {
            path,
            stop,
            stats,
            thread: Some(thread),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

struct Conn {
    tx: Sender<Request>,
    pending: Arc<AtomicUsize>,
    wake: Arc<dyn Fn() + Send + Sync>,
    stop: Arc<AtomicBool>,
    stats: Arc<Stats>,
    server_name: String,
    instructions: String,
}

impl Conn {
    fn serve(self, stream: UnixStream) {
        self.stats.clients.fetch_add(1, Ordering::Relaxed);
        (self.wake)();
        let _ = self.serve_inner(stream);
        self.stats.clients.fetch_sub(1, Ordering::Relaxed);
        (self.wake)();
    }

    fn serve_inner(&self, stream: UnixStream) -> std::io::Result<()> {
        // accepted sockets inherit the listener's non-blocking mode on macOS / BSD; this
        // connection is served by its own thread, so make it blocking with a read timeout
        // that lets the thread notice `stop` while idle
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_millis(300)))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        let mut line = String::new();
        loop {
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(()),
                Ok(_) if line.ends_with('\n') => {
                    let taken = std::mem::take(&mut line);
                    if let Some(resp) = self.handle(&taken) {
                        writer.write_all(resp.as_bytes())?;
                        writer.write_all(b"\n")?;
                        writer.flush()?;
                    }
                }
                Ok(_) => return Ok(()), // EOF without a newline
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    // partial line stays in `line`
                    if self.stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn handle(&self, line: &str) -> Option<String> {
        mcp::handle_line(line, &self.server_name, &self.instructions, &mut |cmd| {
            self.call(cmd)
        })
    }

    fn call(&self, cmd: Command) -> Reply {
        self.stats.calls.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = channel();
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self
            .tx
            .send(Request {
                cmd,
                reply: reply_tx,
            })
            .is_err()
        {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            return Reply::Error("the app is shutting down".into());
        }
        (self.wake)();
        match reply_rx.recv_timeout(CALL_TIMEOUT) {
            Ok(r) => r,
            Err(_) => {
                Reply::Error("timed out waiting for the app (is the window responsive?)".into())
            }
        }
    }
}

/// Connect to a running app's server.
pub fn connect(path: &Path) -> std::io::Result<UnixStream> {
    UnixStream::connect(path)
}
