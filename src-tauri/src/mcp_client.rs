//! Real external Model Context Protocol (MCP) JSON-RPC 2.0 client.
//! Supports both Streamable HTTP (`http://` / `https://`) and local `stdio`
//! subprocess transports for user-configured connectors in `connectors_custom.json`.

use insar_agent_model::ToolDescription;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::application::CustomConnectorRecord;
use crate::gee_cloud_suite::sanitize_for_agent_trace;

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const CACHE_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize)]
pub struct McpToolInfo {
    pub name: String,
    pub qualified_name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpProbeResult {
    pub ok: bool,
    pub connector_id: String,
    pub transport: String,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub tools: Vec<McpToolInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedToolsEntry {
    endpoint: String,
    transport: String,
    fetched_at: Instant,
    tools: Vec<McpToolInfo>,
}

fn cache() -> &'static Mutex<HashMap<String, CachedToolsEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedToolsEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn invalidate_connector_cache(connector_id: Option<&str>) {
    if let Ok(mut guard) = cache().lock() {
        match connector_id {
            Some(id) => {
                guard.remove(id);
            }
            None => guard.clear(),
        }
    }
}

fn sanitize_tool_ident(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        "tool".to_owned()
    } else {
        trimmed
    }
}

pub fn qualify_mcp_tool_name(connector_id: &str, tool_name: &str) -> String {
    let conn = sanitize_tool_ident(connector_id);
    let tool = sanitize_tool_ident(tool_name);
    let mut full = format!("mcp__{conn}__{tool}");
    if full.len() > 64 {
        full.truncate(64);
    }
    full
}

fn normalize_tool_schema(raw: Option<&Value>) -> Value {
    let mut obj = raw.and_then(Value::as_object).cloned().unwrap_or_else(|| {
        let mut m = Map::new();
        m.insert("type".into(), json!("object"));
        m.insert("properties".into(), json!({}));
        m
    });
    if !obj.contains_key("type") {
        obj.insert("type".into(), json!("object"));
    }
    if obj.get("type").and_then(Value::as_str) == Some("object") && !obj.contains_key("properties")
    {
        obj.insert("properties".into(), json!({}));
    }
    if !obj.contains_key("additionalProperties") {
        obj.insert("additionalProperties".into(), json!(false));
    }
    Value::Object(obj)
}

fn is_loopback_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("://127.0.0.1") || lower.contains("://localhost") || lower.contains("://[::1]")
}

fn parse_json_or_sse_rpc_response(body: &str, expected_id: u64) -> Result<Value, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("MCP_EMPTY_RESPONSE".to_owned());
    }
    let check_msg = |msg: Value| -> Option<Result<Value, String>> {
        let id_matches = msg
            .get("id")
            .and_then(Value::as_u64)
            .is_some_and(|v| v == expected_id);
        if !id_matches && msg.get("result").is_none() && msg.get("error").is_none() {
            return None;
        }
        if let Some(err) = msg.get("error") {
            let detail = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("MCP JSON-RPC error");
            return Some(Err(insar_agent::history::outbound_text(detail)));
        }
        Some(Ok(msg.get("result").cloned().unwrap_or(Value::Null)))
    };

    if trimmed.starts_with('{') {
        let val: Value =
            serde_json::from_str(trimmed).map_err(|e| format!("MCP_INVALID_JSON: {e}"))?;
        if let Some(res) = check_msg(val) {
            return res;
        }
    }

    if trimmed.starts_with('[') {
        let arr: Vec<Value> =
            serde_json::from_str(trimmed).map_err(|e| format!("MCP_INVALID_JSON_BATCH: {e}"))?;
        for item in arr {
            if let Some(res) = check_msg(item) {
                return res;
            }
        }
    }

    // Streamable HTTP SSE stream (`data: {...}`)
    let mut last_candidate: Option<Result<Value, String>> = None;
    for line in trimmed.lines() {
        let l = line.trim();
        let Some(data) = l.strip_prefix("data:") else {
            continue;
        };
        let payload = data.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(payload) {
            if let Some(res) = check_msg(val) {
                last_candidate = Some(res);
            }
        }
    }
    last_candidate.unwrap_or_else(|| Err("MCP_UNPARSABLE_RESPONSE".to_owned()))
}

fn post_http_jsonrpc(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    session_id: &mut Option<String>,
    body: &Value,
    expected_id: Option<u64>,
) -> Result<Value, String> {
    let mut req = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(ref sid) = *session_id {
        req = req.header("Mcp-Session-Id", sid.as_str());
    }
    let resp = req
        .json(body)
        .send()
        .map_err(|e| insar_agent::history::outbound_text(&format!("MCP_HTTP_ERROR: {e}")))?;
    if let Some(hdr_sid) = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = hdr_sid.trim();
        if !trimmed.is_empty() {
            *session_id = Some(trimmed.to_owned());
        }
    }
    let status = resp.status();
    if !status.is_success() {
        let txt = resp.text().unwrap_or_default();
        return Err(insar_agent::history::outbound_text(&format!(
            "MCP HTTP {status}: {}",
            txt.chars().take(240).collect::<String>()
        )));
    }
    let Some(id) = expected_id else {
        return Ok(Value::Null);
    };
    let text = resp
        .text()
        .map_err(|e| insar_agent::history::outbound_text(&format!("MCP_HTTP_READ_ERROR: {e}")))?;
    parse_json_or_sse_rpc_response(&text, id)
}

fn build_http_client(
    endpoint: &str,
    timeout: Duration,
) -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder().timeout(timeout);
    if is_loopback_url(endpoint) {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|e| format!("MCP_CLIENT_BUILD_ERROR: {e}"))
}

fn parse_shell_command_words(cmd: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;
    for ch in cmd.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

struct StdioSession {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: mpsc::Receiver<String>,
}

impl StdioSession {
    fn spawn(endpoint: &str) -> Result<Self, String> {
        let parts = parse_shell_command_words(endpoint);
        let Some((program, args)) = parts.split_first() else {
            return Err("MCP_STDIO_EMPTY_COMMAND".to_owned());
        };
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                insar_agent::history::outbound_text(&format!("MCP_STDIO_SPAWN_FAILED: {e}"))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP_STDIO_NO_STDIN".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP_STDIO_NO_STDOUT".to_owned())?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { child, stdin, rx })
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        writeln!(self.stdin, "{msg}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("MCP_STDIO_WRITE_ERROR: {e}"))
    }

    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("MCP_STDIO_WRITE_ERROR: {e}"))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("MCP_STDIO_TIMEOUT".to_owned());
            }
            let line = self
                .rx
                .recv_timeout(remaining)
                .map_err(|_| "MCP_STDIO_TIMEOUT".to_owned())?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if val.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(err) = val.get("error") {
                let detail = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP stdio error");
                return Err(insar_agent::history::outbound_text(detail));
            }
            return Ok(val.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl Drop for StdioSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn initialize_params() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {
            "name": "EOStage",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

fn parse_tools_list(connector_id: &str, res: &Value) -> Vec<McpToolInfo> {
    let Some(arr) = res.get("tools").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            let desc = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let input_schema =
                normalize_tool_schema(item.get("inputSchema").or_else(|| item.get("input_schema")));
            Some(McpToolInfo {
                name: name.to_owned(),
                qualified_name: qualify_mcp_tool_name(connector_id, name),
                description: if desc.is_empty() {
                    format!("外部 MCP 工具 ({name})")
                } else {
                    desc.to_owned()
                },
                input_schema,
            })
        })
        .collect()
}

pub fn probe_connector_tools(connector: &CustomConnectorRecord) -> McpProbeResult {
    let endpoint = connector.endpoint.trim();
    let transport = connector.transport.trim().to_ascii_lowercase();
    let is_http = transport == "http"
        || transport == "https"
        || endpoint.starts_with("http://")
        || endpoint.starts_with("https://");

    let outcome = if is_http {
        probe_http(connector, endpoint)
    } else {
        probe_stdio(connector, endpoint)
    };

    match outcome {
        Ok((server_name, server_version, tools)) => {
            if let Ok(mut guard) = cache().lock() {
                guard.insert(
                    connector.id.clone(),
                    CachedToolsEntry {
                        endpoint: endpoint.to_owned(),
                        transport: connector.transport.clone(),
                        fetched_at: Instant::now(),
                        tools: tools.clone(),
                    },
                );
            }
            McpProbeResult {
                ok: true,
                connector_id: connector.id.clone(),
                transport: connector.transport.clone(),
                server_name,
                server_version,
                tools,
                error: None,
            }
        }
        Err(err) => McpProbeResult {
            ok: false,
            connector_id: connector.id.clone(),
            transport: connector.transport.clone(),
            server_name: None,
            server_version: None,
            tools: Vec::new(),
            error: Some(insar_agent::history::outbound_text(&err)),
        },
    }
}

type ProbeHandshake = (Option<String>, Option<String>, Vec<McpToolInfo>);

fn probe_http(connector: &CustomConnectorRecord, endpoint: &str) -> Result<ProbeHandshake, String> {
    let client = build_http_client(endpoint, PROBE_TIMEOUT)?;
    let mut sid: Option<String> = None;
    let init_res = post_http_jsonrpc(
        &client,
        endpoint,
        &mut sid,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": initialize_params()}),
        Some(1),
    )?;
    let _ = post_http_jsonrpc(
        &client,
        endpoint,
        &mut sid,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}),
        None,
    );
    let list_res = post_http_jsonrpc(
        &client,
        endpoint,
        &mut sid,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        Some(2),
    )?;
    let info = init_res.get("serverInfo");
    let server_name = info
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let server_version = info
        .and_then(|s| s.get("version"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok((
        server_name,
        server_version,
        parse_tools_list(&connector.id, &list_res),
    ))
}

fn probe_stdio(
    connector: &CustomConnectorRecord,
    endpoint: &str,
) -> Result<ProbeHandshake, String> {
    let mut session = StdioSession::spawn(endpoint)?;
    let init_res = session.request(1, "initialize", initialize_params(), PROBE_TIMEOUT)?;
    let _ = session.send_notification("notifications/initialized", json!({}));
    let list_res = session.request(2, "tools/list", json!({}), PROBE_TIMEOUT)?;
    let info = init_res.get("serverInfo");
    let server_name = info
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let server_version = info
        .and_then(|s| s.get("version"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok((
        server_name,
        server_version,
        parse_tools_list(&connector.id, &list_res),
    ))
}

pub fn cached_active_extension_tools(connectors: &[CustomConnectorRecord]) -> Vec<ToolDescription> {
    let mut descriptions = Vec::new();
    for conn in connectors {
        if !conn.enabled || conn.endpoint.trim().is_empty() {
            continue;
        }
        let cached = cache().lock().ok().and_then(|guard| {
            guard.get(&conn.id).and_then(|entry| {
                if entry.endpoint == conn.endpoint.trim()
                    && entry.transport == conn.transport
                    && entry.fetched_at.elapsed() < CACHE_TTL
                {
                    Some(entry.tools.clone())
                } else {
                    None
                }
            })
        });
        let tools = match cached {
            Some(list) => list,
            None => {
                // Lazy bounded probe on first turn when an HTTP/stdio connector hasn't been cached yet.
                let probed = probe_connector_tools(conn);
                if !probed.ok {
                    if let Ok(mut guard) = cache().lock() {
                        guard.insert(
                            conn.id.clone(),
                            CachedToolsEntry {
                                endpoint: conn.endpoint.trim().to_owned(),
                                transport: conn.transport.clone(),
                                fetched_at: Instant::now(),
                                tools: Vec::new(),
                            },
                        );
                    }
                    Vec::new()
                } else {
                    probed.tools
                }
            }
        };
        for tool in tools {
            descriptions.push(ToolDescription {
                name: tool.qualified_name,
                description: format!("[MCP:{}] {}", conn.name, tool.description),
                parameters: tool.input_schema,
            });
        }
    }
    descriptions
}

pub fn call_connector_tool(
    connectors: &[CustomConnectorRecord],
    qualified_tool_name: &str,
    input: &Value,
) -> Option<Value> {
    if !qualified_tool_name.starts_with("mcp__") {
        return None;
    }
    let clean_args = sanitize_for_agent_trace(input);
    for conn in connectors {
        if !conn.enabled {
            continue;
        }
        let tools = {
            let cached = cache().lock().ok().and_then(|g| {
                g.get(&conn.id)
                    .filter(|e| e.endpoint == conn.endpoint.trim())
                    .map(|e| e.tools.clone())
            });
            match cached {
                Some(t) if !t.is_empty() => t,
                _ => probe_connector_tools(conn).tools,
            }
        };
        let Some(matched) = tools
            .into_iter()
            .find(|t| t.qualified_name == qualified_tool_name)
        else {
            continue;
        };

        let endpoint = conn.endpoint.trim();
        let transport = conn.transport.trim().to_ascii_lowercase();
        let is_http = transport == "http"
            || transport == "https"
            || endpoint.starts_with("http://")
            || endpoint.starts_with("https://");

        let call_res = if is_http {
            call_http(endpoint, &matched.name, &clean_args)
        } else {
            call_stdio(endpoint, &matched.name, &clean_args)
        };
        return Some(match call_res {
            Ok(result) => sanitize_for_agent_trace(&json!({
                "ok": !result.get("isError").and_then(Value::as_bool).unwrap_or(false),
                "connector_id": conn.id,
                "tool": matched.name,
                "result": result,
            })),
            Err(err) => sanitize_for_agent_trace(&json!({
                "ok": false,
                "connector_id": conn.id,
                "tool": matched.name,
                "error": err,
            })),
        });
    }
    None
}

fn call_http(endpoint: &str, tool_name: &str, args: &Value) -> Result<Value, String> {
    let client = build_http_client(endpoint, CALL_TIMEOUT)?;
    let mut sid: Option<String> = None;
    let _ = post_http_jsonrpc(
        &client,
        endpoint,
        &mut sid,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": initialize_params()}),
        Some(1),
    )?;
    let _ = post_http_jsonrpc(
        &client,
        endpoint,
        &mut sid,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}),
        None,
    );
    post_http_jsonrpc(
        &client,
        endpoint,
        &mut sid,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args,
            }
        }),
        Some(2),
    )
}

fn call_stdio(endpoint: &str, tool_name: &str, args: &Value) -> Result<Value, String> {
    let mut session = StdioSession::spawn(endpoint)?;
    let _ = session.request(1, "initialize", initialize_params(), CALL_TIMEOUT)?;
    let _ = session.send_notification("notifications/initialized", json!({}));
    session.request(
        2,
        "tools/call",
        json!({
            "name": tool_name,
            "arguments": args,
        }),
        CALL_TIMEOUT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn http_loopback_mcp_probe_list_and_call_work_and_scrub_paths() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().unwrap();
        let server_handle = std::thread::spawn(move || {
            // Two transactions: (initialize + notification + tools/list) then (initialize + notification + tools/call)
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
                let resp_body = if body.contains("\"initialize\"") {
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "serverInfo": { "name": "test-eo-mcp", "version": "0.1.0" },
                            "capabilities": {}
                        }
                    })
                    .to_string()
                } else if body.contains("\"notifications/initialized\"") {
                    String::new()
                } else if body.contains("\"tools/list\"") {
                    json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "result": {
                            "tools": [{
                                "name": "raster-echo",
                                "description": "Echo raster summary",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": { "query": { "type": "string" } },
                                    "required": ["query"]
                                }
                            }]
                        }
                    })
                    .to_string()
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "result": {
                            "content": [{ "type": "text", "text": "echoed /Users/alice/secret.tif -> ok" }],
                            "isError": false
                        }
                    })
                    .to_string()
                };
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(http_resp.as_bytes());
            }
        });

        let connector = CustomConnectorRecord {
            id: "loopback-eo".into(),
            name: "Loopback EO".into(),
            transport: "http".into(),
            endpoint: format!("http://{addr}/mcp"),
            description: "test".into(),
            enabled: true,
        };
        let probe = probe_connector_tools(&connector);
        assert!(probe.ok, "{probe:?}");
        assert_eq!(probe.server_name.as_deref(), Some("test-eo-mcp"));
        assert_eq!(probe.tools.len(), 1);
        assert_eq!(
            probe.tools[0].qualified_name,
            "mcp__loopback_eo__raster_echo"
        );

        let tools = cached_active_extension_tools(std::slice::from_ref(&connector));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].parameters["additionalProperties"], json!(false));

        let call_out = call_connector_tool(
            &[connector],
            "mcp__loopback_eo__raster_echo",
            &json!({"query": "dem"}),
        )
        .expect("call routed");
        assert_eq!(call_out["ok"], true);
        let serialized = call_out.to_string();
        assert!(!serialized.contains("/Users/alice"), "{serialized}");

        let _ = server_handle.join();
        invalidate_connector_cache(Some("loopback-eo"));
    }
}
