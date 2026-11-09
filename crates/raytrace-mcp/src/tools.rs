//! The four tools: their definitions, argument parsing and the results of the two that need
//! no session.
//!
//! The server deliberately offers four generic tools instead of one tool per command: the
//! schema of a design document is tens of kilobytes and would be repeated in many tool
//! definitions. `list_commands` and `describe_command` let the agent load only the schemas
//! it needs, and a command added to the engine registry is reachable at once.
use crate::protocol::RpcError;
use raytrace_core::trace::TraceControl;
use raytrace_engine::{Context, Response, Session, registry};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// One command to run: its id and its parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub command: String,
    pub params: Value,
}

/// A parsed `tools/call`.
#[derive(Debug, PartialEq)]
pub enum Call {
    ListCommands,
    DescribeCommand(String),
    Execute(Request),
    Batch {
        requests: Vec<Request>,
        stop_on_error: bool,
    },
}

fn request(value: &Value, what: &str) -> Result<Request, RpcError> {
    let Value::Object(fields) = value else {
        return Err(RpcError::invalid_params(format!(
            "{what} must be an object with `command` and optional `params`"
        )));
    };
    if let Some(unknown) = fields.keys().find(|k| *k != "command" && *k != "params") {
        return Err(RpcError::invalid_params(format!(
            "{what}: unknown field `{unknown}` (expected `command` and `params`)"
        )));
    }
    let command = fields
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params(format!("{what}: `command` must be a string")))?
        .to_string();
    let params = match fields.get("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(params @ Value::Object(_)) => params.clone(),
        Some(_) => {
            return Err(RpcError::invalid_params(format!(
                "{what}: `params` must be an object"
            )));
        }
    };
    Ok(Request { command, params })
}

/// Parse the `name` and `arguments` of a `tools/call`.
pub fn parse_call(name: &str, arguments: &Map<String, Value>) -> Result<Call, RpcError> {
    match name {
        "list_commands" => Ok(Call::ListCommands),
        "describe_command" => arguments
            .get("command")
            .and_then(Value::as_str)
            .map(|command| Call::DescribeCommand(command.to_string()))
            .ok_or_else(|| RpcError::invalid_params("`command` must be a string")),
        "execute" => Ok(Call::Execute(request(
            &Value::Object(arguments.clone()),
            "execute",
        )?)),
        "batch" => {
            let Some(Value::Array(items)) = arguments.get("requests") else {
                return Err(RpcError::invalid_params(
                    "`requests` must be an array of {command, params}",
                ));
            };
            let requests = items
                .iter()
                .enumerate()
                .map(|(i, item)| request(item, &format!("requests[{i}]")))
                .collect::<Result<Vec<_>, _>>()?;
            let stop_on_error = match arguments.get("stop_on_error") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => {
                    return Err(RpcError::invalid_params(
                        "`stop_on_error` must be a boolean",
                    ));
                }
            };
            if let Some(unknown) = arguments
                .keys()
                .find(|k| *k != "requests" && *k != "stop_on_error")
            {
                return Err(RpcError::invalid_params(format!(
                    "unknown argument `{unknown}`"
                )));
            }
            Ok(Call::Batch {
                requests,
                stop_on_error,
            })
        }
        other => Err(RpcError::invalid_params(format!("Unknown tool: {other}"))),
    }
}

/// The response envelope of the engine as a JSON Schema (`result` follows the command).
fn envelope_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "command": {"type": "string", "description": "The command as written in the request."},
            "revision": {"type": "integer", "minimum": 0, "description": "Revision of the session after the command."},
            "error": {
                "type": "object",
                "properties": {"code": {"type": "string"}, "message": {"type": "string"}},
                "required": ["code", "message"]
            },
            "result": {"description": "Result of the command; see describe_command for its schema."}
        },
        "required": ["ok", "command", "revision"]
    })
}

/// A request item of `execute` and `batch`.
fn request_properties() -> Value {
    json!({
        "command": {
            "type": "string",
            "description": "Command id such as `analysis.trace` (see list_commands)."
        },
        "params": {
            "type": "object",
            "description": "Parameters of the command (see describe_command). Omit for commands without parameters."
        }
    })
}

/// The tool definitions of `tools/list`. `outputSchema` is part of protocol 2025-06-18 only.
pub fn definitions(with_output_schema: bool) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "list_commands",
            "title": "List commands",
            "description": "List every command of raytrace-rs with its id, kind (query, mutation or job) and summary. Start here.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "describe_command",
            "title": "Describe a command",
            "description": "Return the JSON Schemas of one command's parameters (`input_schema`) and result (`output_schema`), with its summary and kind. Call this before using a command you do not know.",
            "inputSchema": {
                "type": "object",
                "properties": {"command": {"type": "string", "description": "Command id such as `analysis.trace`."}},
                "required": ["command"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "execute",
            "title": "Execute a command",
            "description": "Run one command in the server's session and return its response `{ok, command, revision, result | error}`. A failed command sets isError and carries `error.code` and `error.message`. The session keeps the open design between calls: doc.open, then edit with doc.set or component.*, then analysis.trace without `model`; session.undo reverts the last change.",
            "inputSchema": {
                "type": "object",
                "properties": request_properties(),
                "required": ["command"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "batch",
            "title": "Execute several commands",
            "description": "Run several commands in order in the same session and return the list of their responses. By default it stops at the first failed command (`stop_on_error`); the failed response is the last one. Use it for sequences such as doc.open followed by edits.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "requests": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": request_properties(),
                            "required": ["command"],
                            "additionalProperties": false
                        }
                    },
                    "stop_on_error": {"type": "boolean", "default": true, "description": "Stop after the first failed command."}
                },
                "required": ["requests"],
                "additionalProperties": false
            }
        }),
    ];
    if with_output_schema {
        for tool in &mut tools {
            let schema = match tool["name"].as_str() {
                Some("execute") => Some(envelope_schema()),
                Some("batch") => Some(json!({
                    "type": "object",
                    "properties": {
                        "ok": {"type": "boolean", "description": "Every request ran and succeeded."},
                        "executed": {"type": "integer", "minimum": 0},
                        "requested": {"type": "integer", "minimum": 0},
                        "responses": {"type": "array", "items": envelope_schema()}
                    },
                    "required": ["ok", "executed", "requested", "responses"]
                })),
                _ => None,
            };
            if let (Some(schema), Some(tool)) = (schema, tool.as_object_mut()) {
                tool.insert("outputSchema".into(), schema);
            }
        }
    }
    tools
}

/// The result object of a `tools/call`: the structured value as text and as
/// `structuredContent`.
pub fn tool_result(text: String, is_error: bool) -> Value {
    let structured = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
        "isError": is_error
    })
}

/// A tool result for a failure that is not a command response.
pub fn tool_error(code: &str, message: &str) -> Value {
    let text = json!({"ok": false, "error": {"code": code, "message": message}}).to_string();
    tool_result(text, true)
}

/// The tool result of one command response.
pub fn response_result(response: &Response) -> Value {
    match serde_json::to_string(response) {
        Ok(text) => tool_result(text, !response.ok),
        Err(e) => tool_error("invalid_json", &format!("応答をJSONにできません: {e}")),
    }
}

/// The structured result of `batch`.
#[derive(Serialize)]
pub struct BatchOutput {
    pub ok: bool,
    pub executed: usize,
    pub requested: usize,
    pub responses: Vec<Response>,
}

/// The tool result of `batch`.
pub fn batch_result(output: &BatchOutput) -> Value {
    match serde_json::to_string(output) {
        Ok(text) => tool_result(text, !output.ok),
        Err(e) => tool_error("invalid_json", &format!("応答をJSONにできません: {e}")),
    }
}

/// `list_commands`: id, kind and summary of every command.
pub fn list_commands() -> Value {
    let commands: Vec<Value> = registry()
        .iter()
        .map(|spec| {
            json!({
                "id": spec.id,
                "kind": spec.kind,
                "summary": spec.summary,
            })
        })
        .collect();
    tool_result(json!({"commands": commands}).to_string(), false)
}

/// `describe_command`: the engine's `command.describe` for one command. An unknown command
/// gives the engine's error with similar ids.
pub fn describe_command(command: &str) -> Value {
    let control = TraceControl::default();
    let response = Session::new().execute(
        "command.describe",
        json!({"id": command}),
        &mut Context::new(&control),
    );
    if response.ok {
        let text = response
            .result
            .as_ref()
            .map(|raw| raw.get().to_string())
            .unwrap_or_default();
        tool_result(text, false)
    } else {
        response_result(&response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::code;

    fn args(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    #[test]
    fn execute_arguments_are_checked() {
        let call = parse_call("execute", &args(json!({"command": "app.version"}))).unwrap();
        assert_eq!(
            call,
            Call::Execute(Request {
                command: "app.version".into(),
                params: json!({})
            })
        );
        for bad in [
            json!({}),
            json!({"command": 3}),
            json!({"command": "a.b", "params": []}),
            json!({"command": "a.b", "extra": 1}),
        ] {
            let error = parse_call("execute", &args(bad.clone())).unwrap_err();
            assert_eq!(error.code, code::INVALID_PARAMS, "{bad}");
        }
        assert_eq!(
            parse_call("nope", &Map::new()).unwrap_err().code,
            code::INVALID_PARAMS
        );
    }

    #[test]
    fn batch_stops_on_error_by_default() {
        let call = parse_call(
            "batch",
            &args(json!({"requests": [{"command": "app.version"}]})),
        )
        .unwrap();
        assert!(matches!(
            call,
            Call::Batch {
                stop_on_error: true,
                ..
            }
        ));
        for bad in [
            json!({}),
            json!({"requests": {}}),
            json!({"requests": [1]}),
            json!({"requests": [], "stop_on_error": "no"}),
        ] {
            assert!(parse_call("batch", &args(bad)).is_err());
        }
    }

    #[test]
    fn four_tools_with_input_schemas() {
        for with_output in [false, true] {
            let tools = definitions(with_output);
            let names: Vec<_> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
            assert_eq!(
                names,
                ["list_commands", "describe_command", "execute", "batch"]
            );
            for tool in &tools {
                assert_eq!(tool["inputSchema"]["type"], "object");
                assert_eq!(
                    tool.get("outputSchema").is_some(),
                    with_output && matches!(tool["name"].as_str(), Some("execute" | "batch"))
                );
            }
        }
    }

    #[test]
    fn listing_and_describing_use_the_registry() {
        let list = list_commands();
        assert_eq!(list["isError"], false);
        let commands = list["structuredContent"]["commands"].as_array().unwrap();
        let trace = commands
            .iter()
            .find(|c| c["id"] == "analysis.trace")
            .unwrap();
        assert_eq!(trace["kind"], "job");
        assert!(trace.get("aliases").is_none());

        let described = describe_command("analysis.trace");
        assert_eq!(described["isError"], false);
        assert_eq!(described["structuredContent"]["id"], "analysis.trace");
        assert!(described["structuredContent"]["input_schema"].is_object());

        let unknown = describe_command("doc.opne");
        assert_eq!(unknown["isError"], true);
        assert_eq!(
            unknown["structuredContent"]["error"]["code"],
            "invalid_request"
        );
        // The short names of the pre-beta interface are not commands.
        let old = describe_command("trace");
        assert_eq!(old["isError"], true);
        assert_eq!(old["structuredContent"]["error"]["code"], "invalid_request");
    }
}
