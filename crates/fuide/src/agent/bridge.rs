//! `<app> --mcp`: the stdio side for MCP clients (`claude mcp add fuide-brew -- <binary> --mcp`).
//! Claude Code spawns this process and talks JSON-RPC on stdin/stdout; it answers `initialize` /
//! `tools/list` itself and forwards `tools/call` to the running app over the Unix socket, so
//! the client can start before the app (or before the toggle is on) and simply retries later.
//! If the app is not running, the first call launches it (`open -a` on macOS) and waits for
//! the socket; if the app runs with the agent off, the error says where the switch is.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{mcp, server, Reply};

/// How long to wait for the socket after launching the app.
const LAUNCH_WAIT: Duration = Duration::from_secs(12);

struct Bridge {
    app_name: String,
    path: PathBuf,
    conn: Option<(BufReader<UnixStream>, UnixStream)>,
    launched: bool,
}

impl Bridge {
    fn ensure_connected(&mut self) -> Result<(), String> {
        if self.conn.is_some() {
            return Ok(());
        }
        let deadline = Instant::now() + LAUNCH_WAIT;
        loop {
            match server::connect(&self.path) {
                Ok(stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(120)))
                        .map_err(|e| e.to_string())?;
                    let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
                    self.conn = Some((reader, stream));
                    eprintln!("fuide --mcp: connected to {}", self.path.display());
                    return Ok(());
                }
                Err(_) if !self.launched && cfg!(target_os = "macos") => {
                    self.launched = true;
                    eprintln!("fuide --mcp: launching {}", self.app_name);
                    let _ = std::process::Command::new("open")
                        .arg("-a")
                        .arg(&self.app_name)
                        .status();
                }
                Err(e) if Instant::now() >= deadline => {
                    return Err(format!(
                        "{} is not reachable at {} ({e}). Start the app and turn on Settings (Cmd+,) → AGENT → ON, then call again.",
                        self.app_name,
                        self.path.display()
                    ));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    /// Forward one request line and return the app's response line.
    fn forward(&mut self, line: &str) -> Result<String, String> {
        self.ensure_connected()?;
        let (reader, writer) = self.conn.as_mut().expect("connected");
        let io = (|| -> std::io::Result<String> {
            writer.write_all(line.trim_end().as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            let mut resp = String::new();
            if reader.read_line(&mut resp)? == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the app closed the connection",
                ));
            }
            Ok(resp)
        })();
        match io {
            Ok(resp) => Ok(resp),
            Err(e) => {
                self.conn = None;
                Err(format!(
                    "{} went away ({e}). Is it still running with the agent on? Call again to reconnect.",
                    self.app_name
                ))
            }
        }
    }
}

/// Run the bridge on stdin/stdout until EOF. Returns the process exit code.
pub fn run(app_id: &str, app_name: &str) -> i32 {
    let mut bridge = Bridge {
        app_name: app_name.to_string(),
        path: server::socket_path(app_id),
        conn: None,
        launched: false,
    };
    let instructions = super::instructions(app_name);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let is_call = msg.get("method").and_then(Value::as_str) == Some("tools/call");
        let resp = if is_call {
            match bridge.forward(&line) {
                Ok(resp) => Some(resp.trim_end().to_string()),
                Err(e) => {
                    let id = msg.get("id").cloned().unwrap_or(Value::Null);
                    Some(mcp::response(&id, mcp::call_result(Reply::Error(e))))
                }
            }
        } else {
            mcp::handle_line(&line, app_name, &instructions, &mut |_| {
                Reply::Error("not connected".into())
            })
        };
        if let Some(resp) = resp {
            if writeln!(out, "{resp}").and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    }
    0
}
