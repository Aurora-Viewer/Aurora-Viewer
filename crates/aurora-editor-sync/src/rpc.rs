//! JSON-RPC 2.0 messages of the external editor protocol: what a client
//! sends, decoded and checked, and the envelopes the viewer sends back.
//!
//! Port of LLJSONRPCConnection::onMessage / processMessage / validateMessage /
//! makeEnvelope / sendError and the RPCError classes
//! (indra/llcorehttp/lljsonrpcws.{h,cpp}, originally LGPL 2.1). Same wire
//! format and error codes, so the editors that speak the protocol work
//! unchanged. Stricter: the text is decoded only once its size is known to
//! be within the limit (the socket refuses larger frames before), an `id`
//! must be a string, a number or null as JSON-RPC 2.0 says, and nothing a
//! client sends is ever written to the log.

use serde_json::{Map, Value, json};

/// The JSON-RPC 2.0 codes and the server range codes of RPCError.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const CONNECTION_CLOSED: i64 = -32000;
    pub const REQUEST_TIMEOUT: i64 = -32001;
    pub const UNAUTHORIZED: i64 = -32002;
    pub const FORBIDDEN: i64 = -32003;
    pub const RATE_LIMITED: i64 = -32004;
    pub const SERVICE_UNAVAILABLE: i64 = -32005;
    pub const MESSAGE_TOO_LARGE: i64 = -32006;
}

/// The longest method name echoed back in "Method not found: …".
const ECHOED_METHOD: usize = 128;

/// An error reply, as the RPCError classes build it: the code, and the
/// message with its details after ": ".
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    fn with(code: i64, base: &str, details: &str) -> RpcError {
        let message = if details.is_empty() {
            base.to_owned()
        } else {
            format!("{base}: {details}")
        };
        RpcError { code, message }
    }
    pub fn parse(details: &str) -> RpcError {
        RpcError::with(code::PARSE_ERROR, "Parse error", details)
    }
    pub fn invalid_request(details: &str) -> RpcError {
        RpcError::with(code::INVALID_REQUEST, "Invalid Request", details)
    }
    pub fn method_not_found(method: &str) -> RpcError {
        RpcError::with(code::METHOD_NOT_FOUND, "Method not found", truncate(method, ECHOED_METHOD))
    }
    pub fn invalid_params(details: &str) -> RpcError {
        RpcError::with(code::INVALID_PARAMS, "Invalid params", details)
    }
    pub fn internal(details: &str) -> RpcError {
        RpcError::with(code::INTERNAL_ERROR, "Internal error", details)
    }
    pub fn unauthorized() -> RpcError {
        RpcError::with(code::UNAUTHORIZED, "Authentication required", "")
    }
    pub fn forbidden(details: &str) -> RpcError {
        let message = if details.is_empty() { "Access denied" } else { details };
        RpcError::with(code::FORBIDDEN, message, "")
    }
    pub fn rate_limited() -> RpcError {
        RpcError::with(code::RATE_LIMITED, "Too many requests", "")
    }
    pub fn unavailable() -> RpcError {
        RpcError::with(code::SERVICE_UNAVAILABLE, "Service temporarily unavailable", "")
    }
    pub fn timeout(details: &str) -> RpcError {
        let message = if details.is_empty() { "Request timed out" } else { details };
        RpcError::with(code::REQUEST_TIMEOUT, message, "")
    }
}

/// The longest prefix of `text` of at most `max` bytes that ends on a
/// character boundary.
pub fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// One message from a client, checked as validateMessage does.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A call that waits for its answer. `params` is `Null` when absent.
    Request { id: Value, method: String, params: Value },
    /// A call without an id: never answered.
    Notification { method: String, params: Value },
    /// The answer to one of the viewer's calls.
    Response { id: Value, outcome: Result<Value, Value> },
}

/// What the decoder makes of a text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    Message(Incoming),
    /// Answer with this error (id null), as onMessage does for bad JSON
    /// and batches.
    Reply(RpcError),
    /// Dropped without an answer, as validateMessage's failures are.
    Ignored,
}

/// Decodes one text frame. Never panics, whatever the input: `serde_json`
/// stops at 128 levels of nesting, and the frame is already within the
/// message limit.
pub fn decode(text: &str) -> Decoded {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        // the parser's own message names a line and a column, never the text
        Err(e) => return Decoded::Reply(RpcError::parse(&e.to_string())),
    };
    let object = match value {
        // JSON-RPC 2.0 batches are not supported, as in onMessage
        Value::Array(_) => return Decoded::Reply(RpcError::invalid_request("Batch requests are not supported")),
        Value::Object(object) => object,
        _ => return Decoded::Ignored,
    };
    decode_object(object)
}

fn decode_object(mut object: Map<String, Value>) -> Decoded {
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Decoded::Ignored;
    }
    let id = object.remove("id");
    if let Some(id) = &id
        && !(id.is_string() || id.is_number() || id.is_null())
    {
        return Decoded::Ignored;
    }
    if let Some(method) = object.remove("method") {
        let Value::String(method) = method else {
            return Decoded::Ignored;
        };
        let params = object.remove("params").unwrap_or(Value::Null);
        if !(params.is_null() || params.is_array() || params.is_object()) {
            return Decoded::Ignored;
        }
        return Decoded::Message(match id {
            Some(id) => Incoming::Request { id, method, params },
            None => Incoming::Notification { method, params },
        });
    }
    let Some(id) = id else {
        return Decoded::Ignored;
    };
    let outcome = match (object.remove("result"), object.remove("error")) {
        (_, Some(error)) => Err(error),
        (Some(result), None) => Ok(result),
        (None, None) => return Decoded::Ignored,
    };
    Decoded::Message(Incoming::Response { id, outcome })
}

/// A call's answer.
pub fn response(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// An error answer; `id` is null for a message that had none to answer.
pub fn error(id: &Value, error: &RpcError) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message },
    })
    .to_string()
}

/// A notification; `params` left out when `None`, as makeEnvelope does for
/// an undefined LLSD (`session.ok` has none).
pub fn notification(method: &str, params: Option<Value>) -> String {
    let mut object = Map::new();
    object.insert("jsonrpc".into(), "2.0".into());
    object.insert("method".into(), method.into());
    if let Some(params) = params {
        object.insert("params".into(), params);
    }
    Value::Object(object).to_string()
}

/// A call from the viewer that waits for the client's answer.
pub fn request(id: &str, method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

/// Whether a response's id is `id`: LLJSONRPCConnection::processResponse
/// compares `asString()`, so a client may echo it as a string or a number.
pub fn same_id(received: &Value, id: &str) -> bool {
    match received {
        Value::String(s) => s == id,
        Value::Number(n) => n.to_string() == id,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Incoming {
        match decode(text) {
            Decoded::Message(m) => m,
            other => panic!("not a message: {other:?}"),
        }
    }

    #[test]
    fn requests_notifications_and_responses() {
        assert_eq!(
            message(r#"{"jsonrpc":"2.0","id":7,"method":"script.list"}"#),
            Incoming::Request {
                id: json!(7),
                method: "script.list".into(),
                params: Value::Null
            }
        );
        assert_eq!(
            message(r#"{"jsonrpc":"2.0","method":"x","params":[1]}"#),
            Incoming::Notification {
                method: "x".into(),
                params: json!([1])
            }
        );
        assert_eq!(
            message(r#"{"jsonrpc":"2.0","id":"rpc_1000","result":{"a":1}}"#),
            Incoming::Response {
                id: json!("rpc_1000"),
                outcome: Ok(json!({"a":1}))
            }
        );
        assert_eq!(
            message(r#"{"jsonrpc":"2.0","id":"rpc_1000","error":{"code":1}}"#),
            Incoming::Response {
                id: json!("rpc_1000"),
                outcome: Err(json!({"code":1}))
            }
        );
        // a null id is still a request (answered with a null id)
        assert!(matches!(
            message(r#"{"jsonrpc":"2.0","id":null,"method":"m"}"#),
            Incoming::Request { .. }
        ));
    }

    #[test]
    fn hostile_or_malformed_input_never_panics() {
        let deep = "[".repeat(100_000);
        let cases: &[&str] = &[
            "",
            "{",
            "null",
            "42",
            "\"text\"",
            "[]",
            "[{\"jsonrpc\":\"2.0\",\"method\":\"m\",\"id\":1}]",
            "{}",
            r#"{"jsonrpc":"1.0","method":"m","id":1}"#,
            r#"{"jsonrpc":2.0,"method":"m","id":1}"#,
            r#"{"jsonrpc":"2.0","method":7,"id":1}"#,
            r#"{"jsonrpc":"2.0","method":"m","params":"x","id":1}"#,
            r#"{"jsonrpc":"2.0","method":"m","params":1,"id":1}"#,
            r#"{"jsonrpc":"2.0","method":"m","id":{"a":1}}"#,
            r#"{"jsonrpc":"2.0","method":"m","id":[1]}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","result":1}"#,
            r#"{"jsonrpc":"2.0","id":1e999999,"method":"m"}"#,
            "{\"jsonrpc\":\"2.0\",\"method\":\"\u{0}\\u0000\",\"id\":1}",
            "\u{feff}{}",
            "\u{fffd}\u{fffd}",
            &deep,
        ];
        for text in cases {
            let _ = decode(text);
        }
        assert!(matches!(
            decode(&deep),
            Decoded::Reply(RpcError {
                code: code::PARSE_ERROR,
                ..
            })
        ));
        assert!(matches!(
            decode("{"),
            Decoded::Reply(RpcError {
                code: code::PARSE_ERROR,
                ..
            })
        ));
        assert_eq!(
            decode("[]"),
            Decoded::Reply(RpcError::invalid_request("Batch requests are not supported"))
        );
        assert_eq!(decode(r#"{"jsonrpc":"2.0","method":"m","id":{"a":1}}"#), Decoded::Ignored);
        assert_eq!(decode(r#"{"jsonrpc":"2.0","method":"m","params":"x","id":1}"#), Decoded::Ignored);
        assert_eq!(decode(r#"{"jsonrpc":"1.0","method":"m","id":1}"#), Decoded::Ignored);
    }

    #[test]
    fn envelopes_match_the_legacy_wire_format() {
        assert_eq!(
            response(&json!(3), json!({"ok":true})),
            r#"{"id":3,"jsonrpc":"2.0","result":{"ok":true}}"#
        );
        assert_eq!(
            error(&Value::Null, &RpcError::method_not_found("a.b")),
            r#"{"error":{"code":-32601,"message":"Method not found: a.b"},"id":null,"jsonrpc":"2.0"}"#
        );
        assert_eq!(notification("session.ok", None), r#"{"jsonrpc":"2.0","method":"session.ok"}"#);
        assert_eq!(
            notification("m", Some(json!({"a":1}))),
            r#"{"jsonrpc":"2.0","method":"m","params":{"a":1}}"#
        );
        assert_eq!(
            RpcError::unauthorized(),
            RpcError {
                code: -32002,
                message: "Authentication required".into()
            }
        );
        assert_eq!(RpcError::rate_limited().code, -32004);
    }

    #[test]
    fn method_names_echoed_back_are_bounded() {
        let long = "é".repeat(500);
        let e = RpcError::method_not_found(&long);
        assert!(e.message.len() <= "Method not found: ".len() + ECHOED_METHOD);
        assert_eq!(truncate("abc", 2), "ab");
        assert_eq!(truncate("é", 1), "");
    }

    #[test]
    fn response_ids_compare_as_the_legacy_viewer_does() {
        assert!(same_id(&json!("rpc_1000"), "rpc_1000"));
        assert!(same_id(&json!(1000), "1000"));
        assert!(!same_id(&json!("rpc_1001"), "rpc_1000"));
        assert!(!same_id(&Value::Null, "rpc_1000"));
        assert!(!same_id(&json!(["rpc_1000"]), "rpc_1000"));
    }
}
