//! Small stdio MCP adapter; the same Session API handles every model operation.
use crate::{Error, Outcome, Request, Result};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

pub fn serve(
    reader: impl BufRead,
    mut writer: impl Write,
    mut apply: impl FnMut(Request) -> Result<Outcome>,
) -> Result<()> {
    let mut initialized = false;
    let mut ready = false;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(writer, "{}", rpc_error(Value::Null, -32700, &e.to_string()))?;
                writer.flush()?;
                continue;
            }
        };
        let id = message.get("id").cloned();
        let method = message["method"].as_str();
        if !message.is_object()
            || message["jsonrpc"] != "2.0"
            || method.is_none()
            || id
                .as_ref()
                .is_some_and(|v| !v.is_string() && !v.is_i64() && !v.is_u64())
        {
            writeln!(
                writer,
                "{}",
                rpc_error(Value::Null, -32600, "Invalid JSON-RPC request")
            )?;
            writer.flush()?;
            continue;
        }
        let method = method.unwrap();
        if id.is_none() {
            if method == "notifications/initialized" && initialized {
                ready = true;
            }
            continue;
        }
        let id = id.unwrap();
        let response = match method {
            "initialize" if !initialized => {
                if !message["params"]["protocolVersion"].is_string() {
                    rpc_error(id, -32602, "protocolVersion is required")
                } else {
                    let requested = message["params"]["protocolVersion"].as_str().unwrap();
                    let version = if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
                        .contains(&requested)
                    {
                        requested
                    } else {
                        "2025-11-25"
                    };
                    initialized = true;
                    json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"raytrace-rs","version":env!("CARGO_PKG_VERSION")},"instructions":"Query project/components/properties before editing. In a connected GUI session, use expected_revision for edits. Trace returns a job; query progress or cancel it. Units are mm, deg, um. File paths use the server working directory."}})
                }
            }
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            _ if !ready => rpc_error(id, -32002, "Initialize this session first"),
            "tools/list" => {
                json!({"jsonrpc":"2.0","id":id,"result":{"tools":[{"name":"raytrace_command","description":"Edit, inspect, save and trace an optical system. Use query to discover persistent component IDs and properties. Commands use the same editor session as the GUI when connected.","inputSchema":crate::schema()}]}})
            }
            "tools/call" => {
                if message["params"]["name"] != "raytrace_command" {
                    rpc_error(id, -32602, "Unknown tool")
                } else {
                    let outcome =
                        serde_json::from_value::<Request>(message["params"]["arguments"].clone())
                            .map_err(Error::from)
                            .and_then(&mut apply);
                    let (is_error, data) = match outcome {
                        Ok(o) => (false, json!({"ok":true,"result":o})),
                        Err(e) => (true, json!({"ok":false,"error":e})),
                    };
                    json!({"jsonrpc":"2.0","id":id,"result":{"isError":is_error,"content":[{"type":"text","text":data.to_string()}],"structuredContent":data}})
                }
            }
            _ => rpc_error(id, -32601, "Method not found"),
        };
        writeln!(writer, "{response}")?;
        writer.flush()?;
    }
    Ok(())
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
