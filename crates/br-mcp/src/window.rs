//! A running BR window as the MCP server's backend (`br mcp --connect`): every tool works on what
//! the user sees, through the window's control channel (`br --control`, see `br_ui::control`).
//!
//! The connection signs in with the window's token first. As LightCraft's `Remote`, a request
//! that may have reached the window is never sent twice (an edit would happen twice): only a
//! request that could not be written is retried, on a fresh connection.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use lightcraft_mcp::Backend;
use serde_json::{Value, json};

/// A BR window's control channel.
pub struct Window {
    addr: String,
    token: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
}

impl Window {
    pub fn new(addr: impl Into<String>, token: impl Into<String>) -> Self {
        Self { addr: addr.into(), token: token.into(), conn: None, next_id: 1 }
    }

    /// The window that wrote `path` (`control.json`: its port and token).
    pub fn from_info(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            format!(
                "no BR window to connect to ({}: {e}). Start one with `br --control`, or name its address with --connect HOST:PORT",
                path.display()
            )
        })?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let port = v["port"].as_u64().filter(|p| *p > 0 && *p < 65536).ok_or_else(|| format!("{}: no port", path.display()))?;
        let token = v["token"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| format!("{}: no token", path.display()))?;
        Ok(Self::new(format!("127.0.0.1:{port}"), token))
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Connect and sign in.
    fn connect(&mut self) -> Result<(), String> {
        let addr = self.addr.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut w = stream;
        writeln!(w, "{}", json!({"id": 0, "method": "auth", "params": {"token": self.token}})).map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(line.trim()).map_err(|_| "the window did not answer the sign-in".to_string())?;
        if v["ok"] != true {
            return Err(format!("the window refused the sign-in: {}", v["error"].as_str().unwrap_or("wrong token")));
        }
        self.conn = Some((reader, w));
        Ok(())
    }

    /// Send `line` and read the reply. `Err((e, sent))`: `sent` when the request may have reached
    /// the window.
    fn roundtrip(&mut self, line: &str) -> Result<String, (String, bool)> {
        if self.conn.is_none() {
            self.connect().map_err(|e| (e, false))?;
        }
        let Some((reader, w)) = self.conn.as_mut() else { return Err(("not connected".into(), false)) };
        writeln!(w, "{line}").and_then(|()| w.flush()).map_err(|e| (e.to_string(), false))?;
        let mut reply = String::new();
        match reader.read_line(&mut reply) {
            Ok(0) => Err(("the window closed the connection".into(), true)),
            Ok(_) => Ok(reply),
            Err(e) => Err((e.to_string(), true)),
        }
    }
}

impl Backend for Window {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let reply = match self.roundtrip(&line) {
            Ok(r) => r,
            Err((_, false)) => {
                self.conn = None;
                self.roundtrip(&line).map_err(|(e, sent)| {
                    self.conn = None;
                    if sent {
                        format!("{method}: the connection to the window broke ({e}); it may or may not have run: check before repeating it")
                    } else {
                        format!("the BR window at {} is not reachable: {e} (start it with `br --control`)", self.addr)
                    }
                })?
            }
            Err((e, true)) => {
                self.conn = None;
                return Err(format!("{method}: the connection to the window broke ({e}); it may or may not have run: check before repeating it"));
            }
        };
        let v: Value = serde_json::from_str(reply.trim()).map_err(|e| format!("bad reply from the window: {e}"))?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("the BR window at {} (what the user sees)", self.addr)
    }
}
