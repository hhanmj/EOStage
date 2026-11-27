use insar_agent_mcp::replay::decode_response;
use insar_agent_model::ErrorKind;
use serde_json::json;

#[test]
fn recorded_initialize_list_and_call_results_are_separate_and_bounded() {
    let list = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"public search","inputSchema":{"type":"object"}},{"name":"exec_command","inputSchema":{}}]}}"#;
    let tools = decode_response(list, 2).unwrap();
    assert_eq!(tools["tools"][0]["name"], "search");
    assert_eq!(tools["tools"][1]["name"], "exec_command");
    let call = r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"found"}]}}"#;
    assert_eq!(
        decode_response(call, 3).unwrap()["content"][0]["text"],
        "found"
    );
}

#[test]
fn mismatched_ids_and_error_responses_cannot_be_used_as_tool_results() {
    let mismatch = r#"{"jsonrpc":"2.0","id":44,"result":{"tools":[]}}"#;
    assert_eq!(
        decode_response(mismatch, 2).unwrap_err().kind,
        ErrorKind::Capability
    );
    let error = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"fake-secret"}}"#;
    let diagnostic = decode_response(error, 2).unwrap_err();
    assert_eq!(diagnostic.kind, ErrorKind::Transient);
    assert!(!format!("{diagnostic:?}").contains("fake-secret"));
    assert_eq!(
        decode_response(&json!({"jsonrpc":"1.0","id":2,"result":{}}).to_string(), 2)
            .unwrap_err()
            .kind,
        ErrorKind::Capability
    );
}
