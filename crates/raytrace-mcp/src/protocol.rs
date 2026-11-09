//! JSON-RPC 2.0 framing and the parts of the MCP handshake that do not depend on the session.
use serde_json::{Map, Value, json};

/// Protocol versions the server speaks, newest first.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The first protocol version with `outputSchema` and `structuredContent` for tools.
pub const STRUCTURED_OUTPUT_VERSION: &str = "2025-06-18";

/// Text the client shows to the model when it connects.
pub const INSTRUCTIONS: &str = "\
raytrace-rs is a geometric optics ray tracer (sequential and non-sequential). \
Every operation is a command with an id such as doc.open or analysis.trace. \
Call list_commands first, then describe_command before using a command you do not know: it returns the JSON Schemas of its parameters and result. \
Run commands with execute (one command) or batch (several, in order, stopping at the first failure). \
The server keeps one session with an open design: doc.open loads a JSON/ZMX/STEP design, doc.set and component.* edit it, \
analysis.trace traces it when you omit `model`, and session.undo / session.redo step through the edits (session.log lists them). \
Commands that take `model` use the session's design when it is omitted. \
Typical workflow: doc.open, then doc.inspect for a ZMX, analysis.trace, then analysis.budget (power budget) and analysis.screen (screen distribution, FWHM, ROI), which read the latest trace. \
Percentages are relative to the launched light (100 %); check the budget closure error and the unresolved share before trusting a result, and trace again when a result says `stale`. \
Units are millimetres for lengths and micrometres for wavelengths. \
A ZMX import may use features that cannot be traced: call doc.inspect and check `traceable` and the compatibility report before tracing. \
Long calculations can be cancelled by the client; send a progressToken to receive progress.";

/// JSON-RPC error codes used by the server.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// A JSON-RPC error object.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}
impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_PARAMS, message)
    }
}

/// The version to answer an `initialize` that asked for `requested`: the same one when the
/// server speaks it, else the newest.
pub fn negotiate(requested: &str) -> &'static str {
    SUPPORTED_VERSIONS
        .iter()
        .find(|v| **v == requested)
        .copied()
        .unwrap_or(SUPPORTED_VERSIONS[0])
}

/// One message from the client.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A response to a request of the server. The server sends none, so these are ignored.
    Response,
}

/// Parse one line. The error carries the id to answer with (`null` when unknown).
pub fn parse(line: &[u8]) -> Result<Incoming, (Value, RpcError)> {
    let value: Value = serde_json::from_slice(line).map_err(|e| {
        (
            Value::Null,
            RpcError::new(code::PARSE_ERROR, format!("Parse error: {e}")),
        )
    })?;
    let Value::Object(mut message) = value else {
        return Err((
            Value::Null,
            RpcError::new(
                code::INVALID_REQUEST,
                "Invalid request: expected a JSON object (JSON-RPC batches are not supported)",
            ),
        ));
    };
    let id = message.remove("id");
    let id_valid = matches!(&id, Some(Value::String(_) | Value::Number(_)));
    let answer_id = if id_valid {
        id.clone().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let invalid = |why: &str| {
        (
            answer_id.clone(),
            RpcError::new(code::INVALID_REQUEST, format!("Invalid request: {why}")),
        )
    };
    let method = match message.remove("method") {
        Some(Value::String(method)) => method,
        Some(_) => return Err(invalid("`method` must be a string")),
        None if message.contains_key("result") || message.contains_key("error") => {
            return Ok(Incoming::Response);
        }
        None => return Err(invalid("`method` is missing")),
    };
    let params = message.remove("params").unwrap_or(Value::Null);
    match id {
        None => Ok(Incoming::Notification { method, params }),
        Some(_) if id_valid => Ok(Incoming::Request {
            id: answer_id,
            method,
            params,
        }),
        Some(_) => Err(invalid("`id` must be a string or a number")),
    }
}

/// `{"jsonrpc":"2.0","id":ID,"result":RESULT}`.
pub fn result_message(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// `{"jsonrpc":"2.0","id":ID,"error":{code,message}}`.
pub fn error_message(id: &Value, error: &RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": error.code, "message": error.message}})
}

/// `{"jsonrpc":"2.0","method":METHOD,"params":PARAMS}`.
pub fn notification_message(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// The parameters of a request as an object; absent parameters are an empty object.
pub fn object_params(params: Value) -> Result<Map<String, Value>, RpcError> {
    match params {
        Value::Null => Ok(Map::new()),
        Value::Object(fields) => Ok(fields),
        _ => Err(RpcError::invalid_params("params must be an object")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_negotiated() {
        assert_eq!(negotiate("2024-11-05"), "2024-11-05");
        assert_eq!(negotiate("2025-03-26"), "2025-03-26");
        assert_eq!(negotiate("2025-06-18"), "2025-06-18");
        assert_eq!(negotiate("2099-01-01"), "2025-06-18");
        assert_eq!(negotiate(""), "2025-06-18");
    }

    #[test]
    fn lines_are_classified() {
        let request = parse(br#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap();
        assert_eq!(
            request,
            Incoming::Request {
                id: json!(7),
                method: "ping".into(),
                params: Value::Null
            }
        );
        let notification = parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert!(matches!(notification, Ok(Incoming::Notification { .. })));
        let response = parse(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        assert_eq!(response.unwrap(), Incoming::Response);
    }

    #[test]
    fn malformed_lines_become_errors() {
        let (id, error) = parse(b"{not json").unwrap_err();
        assert_eq!((id, error.code), (Value::Null, code::PARSE_ERROR));
        let (_, error) = parse(b"[1,2]").unwrap_err();
        assert_eq!(error.code, code::INVALID_REQUEST);
        let (id, error) = parse(br#"{"id":"a","method":5}"#).unwrap_err();
        assert_eq!((id, error.code), (json!("a"), code::INVALID_REQUEST));
        let (id, error) = parse(br#"{"id":{"x":1},"method":"ping"}"#).unwrap_err();
        assert_eq!((id, error.code), (Value::Null, code::INVALID_REQUEST));
        let (_, error) = parse(br#"{"id":null,"method":"ping"}"#).unwrap_err();
        assert_eq!(error.code, code::INVALID_REQUEST);
        let (_, error) = parse(b"{}").unwrap_err();
        assert_eq!(error.code, code::INVALID_REQUEST);
    }

    #[test]
    fn params_must_be_objects() {
        assert!(object_params(Value::Null).unwrap().is_empty());
        assert_eq!(
            object_params(json!([1])).unwrap_err().code,
            code::INVALID_PARAMS
        );
    }
}
