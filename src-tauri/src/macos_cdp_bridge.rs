use ring::rand::{SecureRandom, SystemRandom};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

const CDP_UI_PORT: u16 = 5173;
const DEFAULT_CDP_PROFILE_DIR: &str = "/private/tmp/eostage-mac-cdp-chrome";

static BRIDGE_ACTIVE: AtomicBool = AtomicBool::new(false);
static ACTIVE_CDP_PORT: AtomicU16 = AtomicU16::new(0);
static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);
static CHROME_CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
static EVENT_RING: OnceLock<Mutex<VecDeque<(u64, String, serde_json::Value)>>> = OnceLock::new();
static BRIDGE_TOKEN: OnceLock<String> = OnceLock::new();

fn resolve_cdp_ui_port_from(raw: Option<&str>) -> u16 {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|&port| port > 0)
        .unwrap_or(CDP_UI_PORT)
}

fn resolve_cdp_ui_port() -> u16 {
    resolve_cdp_ui_port_from(std::env::var("EOSTAGE_CDP_UI_PORT").ok().as_deref())
}

fn resolve_cdp_profile_dir_from(raw: Option<&str>) -> PathBuf {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CDP_PROFILE_DIR))
}

fn resolve_cdp_profile_dir() -> PathBuf {
    resolve_cdp_profile_dir_from(std::env::var("EOSTAGE_CDP_PROFILE_DIR").ok().as_deref())
}

fn cdp_pkill_pattern(cdp_port: u16) -> String {
    if cdp_port > 0 {
        format!("remote-debugging-port={cdp_port}")
    } else {
        "eostage-mac-cdp-chrome".to_owned()
    }
}

fn generate_bridge_token() -> String {
    let mut bytes = [0u8; 32];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        panic!("CSPRNG unavailable for macos_cdp_bridge token");
    }
    let mut out = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn bridge_token() -> &'static str {
    BRIDGE_TOKEN.get_or_init(generate_bridge_token).as_str()
}

fn is_allowed_cdp_origin(origin: Option<&str>, ui_port: u16) -> bool {
    let Some(raw) = origin.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    raw == format!("http://127.0.0.1:{ui_port}") || raw == format!("http://localhost:{ui_port}")
}

fn is_allowed_shim_script_request(
    sec_fetch_site: Option<&str>,
    referer: Option<&str>,
    ui_port: u16,
) -> bool {
    let site_ok = match sec_fetch_site.map(str::trim).filter(|v| !v.is_empty()) {
        Some(site) => site.eq_ignore_ascii_case("same-origin"),
        None => false,
    };
    if !site_ok {
        return false;
    }
    if let Some(raw_ref) = referer.map(str::trim).filter(|v| !v.is_empty()) {
        let p1 = format!("http://127.0.0.1:{ui_port}");
        let p2 = format!("http://localhost:{ui_port}");
        let ref_ok = raw_ref == p1
            || raw_ref.starts_with(&format!("{p1}/"))
            || raw_ref == p2
            || raw_ref.starts_with(&format!("{p2}/"));
        if !ref_ok {
            return false;
        }
    }
    true
}

fn build_remote_allow_origins_arg(cdp_port: u16, ui_port: u16) -> String {
    format!(
        "--remote-allow-origins=http://127.0.0.1:{cdp_port},http://localhost:{cdp_port},http://127.0.0.1:{ui_port},http://localhost:{ui_port}"
    )
}

fn has_valid_bridge_token(provided: Option<&str>, expected: &str) -> bool {
    let Some(candidate) = provided.map(str::trim) else {
        return false;
    };
    let lhs = candidate.as_bytes();
    let rhs = expected.as_bytes();
    if lhs.is_empty() || lhs.len() != rhs.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&a, &b) in lhs.iter().zip(rhs.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn is_secret_reveal_blocked(cmd: &str, args: &serde_json::Value, allow_env: Option<&str>) -> bool {
    let cmd_trimmed = cmd.trim();
    let is_direct_secret_cmd = matches!(
        cmd_trimmed,
        "credential_agent_channel_key_read"
            | "gee_account_reveal"
            | "credential_reveal"
            | "credential_read_for_edit"
    );
    let is_nested_geodata_reveal = cmd_trimmed == "application_open_geodata_query"
        && [
            args.pointer("/request/dataset")
                .and_then(serde_json::Value::as_str),
            args.get("dataset").and_then(serde_json::Value::as_str),
        ]
        .into_iter()
        .flatten()
        .any(|ds| ds.trim().eq_ignore_ascii_case("gee_account_reveal"));
    (is_direct_secret_cmd || is_nested_geodata_reveal) && allow_env != Some("1")
}

fn chrome_child_slot() -> &'static Mutex<Option<Child>> {
    CHROME_CHILD.get_or_init(|| Mutex::new(None))
}

fn event_ring_slot() -> &'static Mutex<VecDeque<(u64, String, serde_json::Value)>> {
    EVENT_RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(256)))
}

pub fn push_event(event: &str, payload: serde_json::Value) {
    if !is_active() {
        return;
    }
    let seq = NEXT_EVENT_SEQ.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut ring) = event_ring_slot().lock() {
        if ring.len() >= 512 {
            ring.pop_front();
        }
        ring.push_back((seq, event.to_owned(), payload));
    }
}

pub fn is_active() -> bool {
    BRIDGE_ACTIVE.load(Ordering::SeqCst)
}

pub fn pop_next_request() -> String {
    std::thread::sleep(Duration::from_secs(60));
    "__cdp_idle__".to_owned()
}

pub fn deliver_reply(_payload: &str) {}

pub fn shutdown() {
    if let Ok(mut guard) = chrome_child_slot().lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let pat = cdp_pkill_pattern(ACTIVE_CDP_PORT.load(Ordering::SeqCst));
    let _ = Command::new("pkill").args(["-9", "-f", &pat]).status();
}

fn parse_cdp_port() -> Option<u16> {
    let raw = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").ok()?;
    for token in raw.split_whitespace() {
        if let Some(port_str) = token.strip_prefix("--remote-debugging-port=") {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                return Some(port);
            }
        }
    }
    None
}

fn resolve_dist_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        for candidate in [resource_dir.join("dist-tauri"), resource_dir] {
            if candidate.join("tauri.html").is_file() || candidate.join("index.html").is_file() {
                return Some(candidate);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(6) {
            let candidate = ancestor.join("ui").join("dist-tauri");
            if candidate.join("tauri.html").is_file() || candidate.join("index.html").is_file() {
                return Some(candidate);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let dev_candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("ui")
            .join("dist-tauri");
        if dev_candidate.join("tauri.html").is_file() || dev_candidate.join("index.html").is_file()
        {
            return Some(dev_candidate);
        }
    }
    None
}

fn read_ui_asset(app: &AppHandle, dist_dir: Option<&Path>, rel_clean: &str) -> Option<Vec<u8>> {
    let resolver = app.asset_resolver();
    if let Some(asset) = resolver
        .get(rel_clean.to_owned())
        .or_else(|| resolver.get(format!("/{rel_clean}")))
    {
        return Some(asset.bytes);
    }
    if let Some(dir) = dist_dir {
        if let Ok(bytes) = std::fs::read(dir.join(rel_clean)) {
            return Some(bytes);
        }
    }
    None
}

fn render_cdp_shim_js(token: &str) -> String {
    let template = r#"(function () {
  const BRIDGE_TOKEN = "__EOSTAGE_BRIDGE_TOKEN__";
  const callbacks = new Map();
  const listeners = new Map();
  let nextId = 1;
  let nextListenerId = 1;
  let eventCursor = 0;
  window.__TAURI_INTERNALS__ = window.__TAURI_INTERNALS__ || {};
  window.__TAURI_INTERNALS__.metadata = {
    currentWindow: { label: "main" },
    currentWebview: { windowLabel: "main", label: "main" },
    windows: [{ label: "main" }],
    webviews: [{ windowLabel: "main", label: "main" }]
  };
  window.__TAURI_INTERNALS__.transformCallback = function (cb, once) {
    const id = nextId++;
    callbacks.set(id, function (payload) {
      if (once) callbacks.delete(id);
      if (cb) cb(payload);
    });
    return id;
  };
  window.__TAURI_INTERNALS__.unregisterCallback = function (id) {
    callbacks.delete(id);
  };
  window.__TAURI_INTERNALS__.runCallback = function (id, payload) {
    const cb = callbacks.get(id);
    if (cb) cb(payload);
  };
  window.__TAURI_INTERNALS__.convertFileSrc = function (filePath, protocol) {
    const p = protocol || "asset";
    return p + "://localhost/" + encodeURIComponent(filePath);
  };
  window.__TAURI_INTERNALS__.invoke = async function (cmd, args) {
    if (cmd === "plugin:event|listen") {
      const id = nextListenerId++;
      if (args && args.event && typeof args.handler === "number") {
        listeners.set(id, { event: args.event, handler: args.handler });
      }
      return id;
    }
    if (cmd && cmd.startsWith("plugin:event|")) {
      if (cmd === "plugin:event|unlisten" && args && typeof args.eventId === "number") {
        listeners.delete(args.eventId);
      }
      return null;
    }
    const res = await fetch("/__eostage_ipc", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-EOStage-Bridge-Token": BRIDGE_TOKEN
      },
      body: JSON.stringify({ cmd: cmd, args: args || {} })
    });
    const data = await res.json();
    if (data && data.ok) {
      return data.value;
    }
    throw (data && data.error) ? data.error : new Error("IPC error: " + cmd);
  };
  setInterval(async function () {
    if (listeners.size === 0) return;
    try {
      const res = await fetch("/__eostage_events?since=" + eventCursor, {
        headers: { "X-EOStage-Bridge-Token": BRIDGE_TOKEN }
      });
      if (!res.ok) return;
      const data = await res.json();
      if (data && typeof data.seq === "number" && data.seq > eventCursor) {
        eventCursor = data.seq;
      }
      if (data && Array.isArray(data.events)) {
        for (const ev of data.events) {
          for (const [lid, entry] of listeners.entries()) {
            if (entry.event === ev.event) {
              window.__TAURI_INTERNALS__.runCallback(entry.handler, {
                event: ev.event,
                id: lid,
                payload: ev.payload
              });
            }
          }
        }
      }
    } catch (_) {}
  }, 120);
})();"#;
    template.replacen("__EOSTAGE_BRIDGE_TOKEN__", token, 1)
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn build_http_response_headers(
    status: &str,
    content_type: &str,
    body_len: usize,
    allowed_origin: Option<&str>,
) -> String {
    let cors_headers = match allowed_origin {
        Some(origin) => format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\nAccess-Control-Allow-Headers: Content-Type, X-EOStage-Bridge-Token\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n"
        ),
        None => String::new(),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {body_len}\r\n{cors_headers}Cache-Control: no-store\r\nConnection: close\r\n\r\n"
    )
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    allowed_origin: Option<&str>,
) {
    let header = build_http_response_headers(status, content_type, body.len(), allowed_origin);
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_client(mut stream: TcpStream, app: AppHandle, dist_dir: Option<PathBuf>, ui_port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let mut buf = Vec::with_capacity(8192);
    let mut temp = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        match stream.read(&mut temp) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                buf.extend_from_slice(&temp[..n]);
                if buf.len() > 2 * 1024 * 1024 {
                    return;
                }
            }
        }
    };

    let header_str = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = header_str.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let raw_path = parts.next().unwrap_or("/");
    let path_only = raw_path.split('?').next().unwrap_or("/");

    let mut content_length = 0usize;
    let mut origin_header: Option<String> = None;
    let mut token_header: Option<String> = None;
    let mut sec_fetch_site_header: Option<String> = None;
    let mut referer_header: Option<String> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = val.parse::<usize>().unwrap_or(0);
            } else if key.eq_ignore_ascii_case("origin") {
                origin_header = Some(val.to_owned());
            } else if key.eq_ignore_ascii_case("x-eostage-bridge-token") {
                token_header = Some(val.to_owned());
            } else if key.eq_ignore_ascii_case("sec-fetch-site") {
                sec_fetch_site_header = Some(val.to_owned());
            } else if key.eq_ignore_ascii_case("referer") {
                referer_header = Some(val.to_owned());
            }
        }
    }

    if !is_allowed_cdp_origin(origin_header.as_deref(), ui_port) {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "application/json",
            b"{\"ok\":false,\"error\":\"forbidden: untrusted origin\"}",
            None,
        );
        return;
    }
    let allowed_origin = origin_header.as_deref();

    while buf.len() < header_end + content_length {
        match stream.read(&mut temp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&temp[..n]),
        }
    }
    let body_bytes = if buf.len() >= header_end + content_length {
        &buf[header_end..header_end + content_length]
    } else {
        &buf[header_end..]
    };

    if method.eq_ignore_ascii_case("OPTIONS") {
        write_http_response(
            &mut stream,
            "204 No Content",
            "text/plain",
            b"",
            allowed_origin,
        );
        return;
    }

    if method.eq_ignore_ascii_case("POST") && path_only == "/__eostage_ipc" {
        if !has_valid_bridge_token(token_header.as_deref(), bridge_token()) {
            write_http_response(
                &mut stream,
                "403 Forbidden",
                "application/json",
                b"{\"ok\":false,\"error\":\"forbidden: invalid bridge token\"}",
                allowed_origin,
            );
            return;
        }
        let Ok(req_json) = serde_json::from_slice::<serde_json::Value>(body_bytes) else {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "application/json",
                b"{\"ok\":false,\"error\":\"invalid json\"}",
                allowed_origin,
            );
            return;
        };
        let cmd = req_json
            .get("cmd")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let args = req_json
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        let req_id = NEXT_REQ_ID.fetch_add(1, Ordering::SeqCst);
        let allow_secret_env = std::env::var("EOSTAGE_CDP_ALLOW_SECRET_REVEAL").ok();
        if is_secret_reveal_blocked(cmd, &args, allow_secret_env.as_deref()) {
            let blocked_payload = serde_json::json!({
                "id": req_id,
                "ok": false,
                "error": "secret reveal disabled over HTTP debug bridge",
            });
            write_http_response(
                &mut stream,
                "200 OK",
                "application/json",
                blocked_payload.to_string().as_bytes(),
                allowed_origin,
            );
            return;
        }

        #[cfg(target_os = "macos")]
        let dispatch_res = crate::commands::dispatch_macos_cdp_command(&app, cmd, &args);
        #[cfg(not(target_os = "macos"))]
        let dispatch_res: Result<serde_json::Value, serde_json::Value> = {
            let _ = (&app, cmd, &args);
            Err(serde_json::json!({"code": "UNSUPPORTED", "message": "macos_cdp_bridge only"}))
        };

        let payload = match dispatch_res {
            Ok(val) => serde_json::json!({
                "id": req_id,
                "ok": true,
                "value": val,
            }),
            Err(err) => serde_json::json!({
                "id": req_id,
                "ok": false,
                "error": err,
            }),
        };
        write_http_response(
            &mut stream,
            "200 OK",
            "application/json",
            payload.to_string().as_bytes(),
            allowed_origin,
        );
        return;
    }

    if method.eq_ignore_ascii_case("GET") && path_only == "/__eostage_events" {
        if !has_valid_bridge_token(token_header.as_deref(), bridge_token()) {
            write_http_response(
                &mut stream,
                "403 Forbidden",
                "application/json",
                b"{\"ok\":false,\"error\":\"forbidden: invalid bridge token\"}",
                allowed_origin,
            );
            return;
        }
        let since = raw_path
            .split_once('?')
            .and_then(|(_, q)| {
                q.split('&').find_map(|kv| {
                    kv.strip_prefix("since=")
                        .and_then(|v| v.parse::<u64>().ok())
                })
            })
            .unwrap_or(0);
        let mut latest_seq = since;
        let mut items = Vec::new();
        if let Ok(ring) = event_ring_slot().lock() {
            if let Some((last_seq, _, _)) = ring.back() {
                latest_seq = (*last_seq).max(since);
            }
            for (seq, ev, payload) in ring.iter() {
                if *seq > since {
                    items.push(serde_json::json!({
                        "seq": seq,
                        "event": ev,
                        "payload": payload,
                    }));
                }
            }
        }
        let resp = serde_json::json!({
            "seq": latest_seq,
            "events": items,
        });
        write_http_response(
            &mut stream,
            "200 OK",
            "application/json",
            resp.to_string().as_bytes(),
            allowed_origin,
        );
        return;
    }

    if path_only == "/__eostage_cdp_shim.js" {
        if !is_allowed_shim_script_request(
            sec_fetch_site_header.as_deref(),
            referer_header.as_deref(),
            ui_port,
        ) {
            write_http_response(
                &mut stream,
                "403 Forbidden",
                "application/javascript; charset=utf-8",
                b"// forbidden: cross-site script load blocked",
                None,
            );
            return;
        }
        let shim_js = render_cdp_shim_js(bridge_token());
        write_http_response(
            &mut stream,
            "200 OK",
            "application/javascript; charset=utf-8",
            shim_js.as_bytes(),
            allowed_origin,
        );
        return;
    }

    let rel = path_only.trim_start_matches('/');
    let rel_clean = if rel.is_empty() || rel == "index.html" || rel == "tauri.html" {
        "tauri.html"
    } else {
        rel
    };
    if rel_clean.contains("..") {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "text/plain",
            b"Forbidden",
            allowed_origin,
        );
        return;
    }

    if rel_clean == "tauri.html" {
        let html_bytes = read_ui_asset(&app, dist_dir.as_deref(), "tauri.html")
            .or_else(|| read_ui_asset(&app, dist_dir.as_deref(), "index.html"));
        let Some(html) = html_bytes.and_then(|bytes| String::from_utf8(bytes).ok()) else {
            write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain",
                b"Not Found",
                allowed_origin,
            );
            return;
        };
        let injected = html.replacen(
            "<head>",
            "<head><script src=\"/__eostage_cdp_shim.js\"></script>",
            1,
        );
        write_http_response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            injected.as_bytes(),
            allowed_origin,
        );
        return;
    }

    match read_ui_asset(&app, dist_dir.as_deref(), rel_clean) {
        Some(bytes) => {
            write_http_response(
                &mut stream,
                "200 OK",
                content_type_for(Path::new(rel_clean)),
                &bytes,
                allowed_origin,
            );
        }
        None => {
            write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain",
                b"Not Found",
                allowed_origin,
            );
        }
    }
}

pub fn start_if_requested(app: AppHandle) {
    let Some(cdp_port) = parse_cdp_port() else {
        return;
    };
    ACTIVE_CDP_PORT.store(cdp_port, Ordering::SeqCst);
    let _ = bridge_token();
    BRIDGE_ACTIVE.store(true, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    let target_ui_port = resolve_cdp_ui_port();
    let profile_dir = resolve_cdp_profile_dir();
    let pkill_pattern = cdp_pkill_pattern(cdp_port);
    let dist_dir = resolve_dist_dir(&app);
    std::thread::spawn(move || {
        let mut listener_opt = None;
        for _ in 0..30 {
            if let Ok(l) = TcpListener::bind(format!("127.0.0.1:{target_ui_port}")) {
                listener_opt = Some(l);
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let Some(listener) = listener_opt else {
            eprintln!("MACOS_CDP_BRIDGE_BIND_FAILED 127.0.0.1:{target_ui_port}");
            return;
        };
        let bound_ui_port = listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or(target_ui_port);

        let _ = Command::new("pkill")
            .args(["-9", "-f", &pkill_pattern])
            .status();
        let _ = std::fs::remove_dir_all(&profile_dir);
        let _ = std::fs::create_dir_all(&profile_dir);

        let chrome_bin = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        if Path::new(chrome_bin).exists() {
            if let Ok(child) = Command::new(chrome_bin)
                .arg(format!("--remote-debugging-port={cdp_port}"))
                .arg(build_remote_allow_origins_arg(cdp_port, bound_ui_port))
                .arg(format!("--user-data-dir={}", profile_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-popup-blocking")
                .arg("--disable-background-timer-throttling")
                .arg("--disable-backgrounding-occluded-windows")
                .arg("--disable-renderer-backgrounding")
                .arg("--lang=zh-CN")
                .arg("--window-size=1320,880")
                .arg(format!("--app=http://127.0.0.1:{bound_ui_port}/index.html"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Ok(mut slot) = chrome_child_slot().lock() {
                    *slot = Some(child);
                }
            }
        }

        for stream in listener.incoming().flatten() {
            let app_clone = app.clone();
            let dist_clone = dist_dir.clone();
            std::thread::spawn(move || {
                handle_client(stream, app_clone, dist_clone, bound_ui_port);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_token_is_32_byte_random_hex_and_embedded_into_shim() {
        let t1 = generate_bridge_token();
        let t2 = generate_bridge_token();
        assert_eq!(t1.len(), 64);
        assert!(t1
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(t1, t2);

        let shim = render_cdp_shim_js(&t1);
        assert!(shim.contains(&format!("const BRIDGE_TOKEN = \"{t1}\";")));
        assert!(shim.contains("\"X-EOStage-Bridge-Token\": BRIDGE_TOKEN"));
        assert!(!shim.contains("__EOSTAGE_BRIDGE_TOKEN__"));
    }

    #[test]
    fn origin_guard_rejects_cross_origin_requests_and_never_emits_wildcard_cors() {
        assert!(is_allowed_cdp_origin(None, 5173));
        assert!(is_allowed_cdp_origin(Some("http://127.0.0.1:5173"), 5173));
        assert!(is_allowed_cdp_origin(Some("http://localhost:5173"), 5173));

        assert!(!is_allowed_cdp_origin(Some("*"), 5173));
        assert!(!is_allowed_cdp_origin(Some("null"), 5173));
        assert!(!is_allowed_cdp_origin(Some("https://evil.example"), 5173));
        assert!(!is_allowed_cdp_origin(Some("http://127.0.0.1:8080"), 5173));
        assert!(!is_allowed_cdp_origin(
            Some("http://127.0.0.1:5173.evil.example"),
            5173
        ));

        let no_origin_hdr = build_http_response_headers("200 OK", "application/json", 2, None);
        assert!(!no_origin_hdr.contains("Access-Control-Allow-Origin"));

        let exact_origin_hdr = build_http_response_headers(
            "200 OK",
            "application/json",
            2,
            Some("http://127.0.0.1:5173"),
        );
        assert!(exact_origin_hdr.contains("Access-Control-Allow-Origin: http://127.0.0.1:5173\r\n"));
        assert!(!exact_origin_hdr.contains("Access-Control-Allow-Origin: *"));
    }

    #[test]
    fn token_guard_requires_exact_per_process_token() {
        let token = generate_bridge_token();
        assert!(has_valid_bridge_token(Some(&token), &token));
        assert!(!has_valid_bridge_token(None, &token));
        assert!(!has_valid_bridge_token(Some(""), &token));
        assert!(!has_valid_bridge_token(Some(&token[..62]), &token));
        let mut flipped = token.clone();
        flipped.replace_range(0..1, if &token[0..1] == "a" { "b" } else { "a" });
        assert!(!has_valid_bridge_token(Some(&flipped), &token));
    }

    #[test]
    fn secret_reveal_commands_are_blocked_over_http_bridge_by_default() {
        let empty_args = serde_json::json!({});
        for secret_cmd in [
            "credential_agent_channel_key_read",
            "gee_account_reveal",
            "credential_reveal",
            "credential_read_for_edit",
        ] {
            assert!(is_secret_reveal_blocked(secret_cmd, &empty_args, None));
            assert!(is_secret_reveal_blocked(secret_cmd, &empty_args, Some("0")));
            assert!(is_secret_reveal_blocked(
                secret_cmd,
                &empty_args,
                Some("true")
            ));
            assert!(!is_secret_reveal_blocked(
                secret_cmd,
                &empty_args,
                Some("1")
            ));
        }
        for nested_reveal_args in [
            serde_json::json!({ "request": { "dataset": "gee_account_reveal" } }),
            serde_json::json!({ "request": { "dataset": " GEE_ACCOUNT_REVEAL " } }),
            serde_json::json!({ "dataset": "gee_account_reveal" }),
        ] {
            assert!(is_secret_reveal_blocked(
                "application_open_geodata_query",
                &nested_reveal_args,
                None
            ));
            assert!(is_secret_reveal_blocked(
                "application_open_geodata_query",
                &nested_reveal_args,
                Some("0")
            ));
            assert!(!is_secret_reveal_blocked(
                "application_open_geodata_query",
                &nested_reveal_args,
                Some("1")
            ));
        }
        assert!(!is_secret_reveal_blocked(
            "application_open_geodata_query",
            &serde_json::json!({ "request": { "dataset": "gee_account_status" } }),
            None
        ));
        assert!(!is_secret_reveal_blocked(
            "agent_channels_get",
            &empty_args,
            None
        ));
        assert!(!is_secret_reveal_blocked(
            "agent_channel_rename",
            &empty_args,
            None
        ));
    }

    #[test]
    fn shim_script_requires_same_origin_sec_fetch_site_and_local_referer() {
        assert!(is_allowed_shim_script_request(
            Some("same-origin"),
            Some("http://127.0.0.1:5173/index.html"),
            5173
        ));
        assert!(is_allowed_shim_script_request(
            Some("same-origin"),
            Some("http://localhost:5173/"),
            5173
        ));
        assert!(!is_allowed_shim_script_request(
            Some("cross-site"),
            Some("https://evil.example/"),
            5173
        ));
        assert!(!is_allowed_shim_script_request(
            Some("same-site"),
            Some("http://127.0.0.1:8080/index.html"),
            5173
        ));
        assert!(!is_allowed_shim_script_request(Some("none"), None, 5173));
        assert!(!is_allowed_shim_script_request(
            None,
            Some("http://127.0.0.1:5173/index.html"),
            5173
        ));
        assert!(!is_allowed_shim_script_request(
            Some("same-origin"),
            Some("http://127.0.0.1:8080/index.html"),
            5173
        ));

        let origins_arg = build_remote_allow_origins_arg(9392, 5173);
        assert_eq!(
            origins_arg,
            "--remote-allow-origins=http://127.0.0.1:9392,http://localhost:9392,http://127.0.0.1:5173,http://localhost:5173"
        );
        assert!(!origins_arg.contains('*'));
    }

    #[test]
    fn isolated_cdp_env_overrides_preserve_defaults_and_scope_pkill_pattern() {
        assert_eq!(resolve_cdp_ui_port_from(None), 5173);
        assert_eq!(resolve_cdp_ui_port_from(Some("")), 5173);
        assert_eq!(resolve_cdp_ui_port_from(Some("0")), 5173);
        assert_eq!(resolve_cdp_ui_port_from(Some("bad")), 5173);
        assert_eq!(resolve_cdp_ui_port_from(Some(" 5179 ")), 5179);

        assert_eq!(
            resolve_cdp_profile_dir_from(None),
            PathBuf::from("/private/tmp/eostage-mac-cdp-chrome")
        );
        assert_eq!(
            resolve_cdp_profile_dir_from(Some("   ")),
            PathBuf::from("/private/tmp/eostage-mac-cdp-chrome")
        );
        assert_eq!(
            resolve_cdp_profile_dir_from(Some("/tmp/isolated_cdp_profile")),
            PathBuf::from("/tmp/isolated_cdp_profile")
        );

        let scoped = cdp_pkill_pattern(9392);
        assert_eq!(scoped, "remote-debugging-port=9392");
        assert!(!scoped.contains("eostage-mac-cdp-chrome"));
        assert_eq!(cdp_pkill_pattern(0), "eostage-mac-cdp-chrome");
    }
}
