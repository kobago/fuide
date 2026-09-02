//! MCP (Model Context Protocol) server side: JSON-RPC 2.0 messages, one per line, as in the
//! stdio transport. Transport-agnostic — [`handle_line`] turns a request line into a response
//! line and hands tool calls to a callback. Used by the in-app socket server and by the `--mcp`
//! stdio bridge (which forwards `tools/call` to the app and answers everything else itself).

use serde_json::{json, Value};

use super::{Command, Reply};

/// Newest protocol revision this server knows; older clients get their own version echoed back.
pub const PROTOCOL: &str = "2025-06-18";
const KNOWN: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Tool definitions (`tools/list`).
pub fn tools() -> Value {
    json!([
        {
            "name": "observe",
            "description": "Describe the app: a state summary plus every interactive widget on screen as `[role] LABEL (state) @x,y`. Call it first and after each action. Labels are exact (upper-case) strings; pass them verbatim to `click` / `type`.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "screenshot",
            "description": "Capture the window as PNG (the app draws it itself; no screen-recording permission needed). Optionally save it to `path`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scale": { "type": "number", "minimum": 0.1, "maximum": 1, "default": 1, "description": "Downscale factor" },
                    "path": { "type": "string", "description": "Also write the PNG here" }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "click",
            "description": "Move the on-screen agent cursor to the widget with this label and click it. Returns the observation after the click.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "Widget label from `observe`" },
                    "nth": { "type": "integer", "minimum": 1, "default": 1, "description": "When several widgets share the label" }
                },
                "required": ["label"],
                "additionalProperties": false
            }
        },
        {
            "name": "type",
            "description": "Type text. With `label`, the input with that label is focused first; otherwise the currently focused input receives it. `submit` presses Enter afterwards.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "label": { "type": "string", "description": "Input label from `observe`" },
                    "submit": { "type": "boolean", "default": false }
                },
                "required": ["text"],
                "additionalProperties": false
            }
        },
        {
            "name": "key",
            "description": "Press a key or shortcut: `enter`, `escape`, `down`, `cmd+3`, `cmd+f`, `cmd+backspace`, `shift+tab` …",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "repeat": { "type": "integer", "minimum": 1, "maximum": 50, "default": 1 }
                },
                "required": ["key"],
                "additionalProperties": false
            }
        },
        {
            "name": "wait",
            "description": "Let the app run (a brew command, a directory load) and return the observation afterwards.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ms": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 500 }
                },
                "additionalProperties": false
            }
        }
    ])
}

/// Turn a `tools/call` into a [`Command`].
pub fn parse_call(name: &str, args: &Value) -> Result<Command, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
    let n = |k: &str| args.get(k).and_then(Value::as_u64);
    Ok(match name {
        "observe" => Command::Observe,
        "screenshot" => Command::Screenshot {
            scale: args.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            path: s("path"),
        },
        "click" => Command::Click {
            label: s("label")
                .filter(|l| !l.is_empty())
                .ok_or("`label` is required")?,
            nth: n("nth").unwrap_or(1).max(1) as usize,
        },
        "type" => Command::Type {
            text: s("text").ok_or("`text` is required")?,
            label: s("label").filter(|l| !l.is_empty()),
            submit: args.get("submit").and_then(Value::as_bool).unwrap_or(false),
        },
        "key" => Command::Key {
            combo: s("key")
                .filter(|k| !k.is_empty())
                .ok_or("`key` is required")?,
            repeat: n("repeat").unwrap_or(1).clamp(1, 50) as usize,
        },
        "wait" => Command::Wait {
            ms: n("ms").unwrap_or(500).clamp(1, 10_000),
        },
        other => return Err(format!("unknown tool `{other}`")),
    })
}

/// A tool result (`tools/call` response `result`).
pub fn call_result(reply: Reply) -> Value {
    match reply {
        Reply::Text(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Reply::Image { png, text } => json!({
            "content": [
                { "type": "image", "data": super::encode::base64(&png), "mimeType": "image/png" },
                { "type": "text", "text": text }
            ]
        }),
        Reply::Error(text) => {
            json!({ "content": [{ "type": "text", "text": text }], "isError": true })
        }
    }
}

pub fn response(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub fn error(id: &Value, code: i64, message: impl Into<String>) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
        .to_string()
}

/// Handle one request line. `None` = nothing to send (a notification, or a response to us).
/// `call` runs a parsed tool call and blocks until the app has answered.
pub fn handle_line(
    line: &str,
    server_name: &str,
    instructions: &str,
    call: &mut dyn FnMut(Command) -> Reply,
) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(&Value::Null, -32700, format!("parse error: {e}"))),
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return None; // a response (to a request we never send) — ignore
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let is_notification = msg.get("id").is_none();
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL);
            let version = if KNOWN.contains(&requested) {
                requested
            } else {
                PROTOCOL
            };
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": server_name, "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match parse_call(name, &args) {
                Ok(cmd) => call_result(call(cmd)),
                Err(e) => call_result(Reply::Error(e)),
            }
        }
        _ if is_notification => return None,
        _ => return Some(error(&id, -32601, format!("method not found: {method}"))),
    };
    if is_notification {
        return None;
    }
    Some(response(&id, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(line: &str, call: &mut dyn FnMut(Command) -> Reply) -> Option<Value> {
        handle_line(line, "test", "hi", call).map(|s| serde_json::from_str(&s).unwrap())
    }
    fn no_call(_: Command) -> Reply {
        panic!("unexpected tool call")
    }

    #[test]
    fn initialize_negotiates_version_and_lists_tools() {
        let r = handle(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            &mut no_call,
        )
        .unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "test");
        let r = handle(
            r#"{"jsonrpc":"2.0","id":"x","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            &mut no_call,
        )
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL);
        let r = handle(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            &mut no_call,
        )
        .unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["observe", "screenshot", "click", "type", "key", "wait"]
        );
    }

    #[test]
    fn notifications_and_responses_are_silent() {
        assert!(handle(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut no_call
        )
        .is_none());
        assert!(handle(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#, &mut no_call).is_none());
        assert!(handle("   ", &mut no_call).is_none());
    }

    #[test]
    fn unknown_method_and_bad_json_are_errors() {
        let r = handle(
            r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#,
            &mut no_call,
        )
        .unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = handle("{not json", &mut no_call).unwrap();
        assert_eq!(r["error"]["code"], -32700);
    }

    #[test]
    fn tool_calls_are_parsed_and_answered() {
        let mut seen = Vec::new();
        let mut call = |c: Command| {
            seen.push(c);
            Reply::Text("ok".into())
        };
        let r = handle(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"click","arguments":{"label":"REFRESH","nth":2}}}"#,
            &mut call,
        )
        .unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "ok");
        assert_eq!(
            seen,
            [Command::Click {
                label: "REFRESH".into(),
                nth: 2
            }]
        );
        // missing argument → tool error, not a protocol error
        let r = handle(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"click","arguments":{}}}"#,
            &mut no_call,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let r = handle(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope"}}"#,
            &mut no_call,
        )
        .unwrap();
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }

    #[test]
    fn image_replies_carry_base64_png() {
        let v = call_result(Reply::Image {
            png: b"\x89PNG".to_vec(),
            text: "shot".into(),
        });
        assert_eq!(v["content"][0]["type"], "image");
        assert_eq!(v["content"][0]["mimeType"], "image/png");
        assert_eq!(v["content"][0]["data"], "iVBORw==");
        assert_eq!(v["content"][1]["text"], "shot");
    }

    #[test]
    fn defaults_and_clamps() {
        assert_eq!(
            parse_call("wait", &json!({})).unwrap(),
            Command::Wait { ms: 500 }
        );
        assert_eq!(
            parse_call("wait", &json!({"ms": 99999})).unwrap(),
            Command::Wait { ms: 10_000 }
        );
        assert_eq!(
            parse_call("key", &json!({"key": "cmd+3", "repeat": 0})).unwrap(),
            Command::Key {
                combo: "cmd+3".into(),
                repeat: 1
            }
        );
        assert_eq!(
            parse_call("observe", &json!(null)).unwrap(),
            Command::Observe
        );
    }
}
