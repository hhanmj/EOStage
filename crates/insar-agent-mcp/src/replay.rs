//! Recorded JSON-RPC fixture decoder; not a production MCP transport.
use insar_agent_model::{ErrorKind, ModelError};
use serde_json::Value;

pub fn decode_response(line: &str, expected_id: u64) -> Result<Value, ModelError> {
    if line.len() > 1024 * 1024 {
        return Err(ModelError::new(ErrorKind::Capability, None));
    }
    let message: Value =
        serde_json::from_str(line).map_err(|_| ModelError::new(ErrorKind::Capability, None))?;
    if message["jsonrpc"] != "2.0" || message["id"].as_u64() != Some(expected_id) {
        return Err(ModelError::new(ErrorKind::Capability, None));
    }
    if let Some(error) = message.get("error") {
        let kind = match error["code"].as_i64() {
            Some(-32600 | -32602) => ErrorKind::Parameter,
            Some(-32601) => ErrorKind::Capability,
            Some(-32099..=-32000) => ErrorKind::Transient,
            _ => ErrorKind::Unknown,
        };
        return Err(ModelError::new(kind, None));
    }
    message
        .get("result")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or(ModelError::new(ErrorKind::Capability, None))
}
