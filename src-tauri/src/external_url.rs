use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static OAUTH_LISTENER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Desktop-shell boundary for handing an approved web URL to the operating
/// system's default browser. URL policy remains in the named command adapter.
fn is_external_browser_open_disabled(env_value: Option<&str>) -> bool {
    matches!(
        env_value.map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn open_in_default_browser_with_flag(url: &str, disabled: bool) -> io::Result<()> {
    if disabled {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    let child = std::process::Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .spawn()?;
    #[cfg(target_os = "macos")]
    let child = std::process::Command::new("open").arg(url).spawn()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    let child = std::process::Command::new("xdg-open").arg(url).spawn()?;

    drop(child);
    Ok(())
}

pub fn open_in_default_browser(url: &str) -> io::Result<()> {
    let disabled = is_external_browser_open_disabled(
        std::env::var("EOSTAGE_DISABLE_EXTERNAL_BROWSER_OPEN")
            .ok()
            .as_deref(),
    );
    open_in_default_browser_with_flag(url, disabled)
}

/// Returns true when the external URL is an OAuth authorize request that
/// expects a local loopback redirect on port 1455 (`/auth/callback`).
pub fn is_local_oauth_authorize_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.contains("localhost%3a1455")
        || lower.contains("localhost:1455")
        || lower.contains("127.0.0.1%3a1455")
        || lower.contains("127.0.0.1:1455"))
        && (lower.contains("response_type=code") || lower.contains("code_challenge="))
}

/// Spawns a short-lived local HTTP loopback listener on `127.0.0.1:1455` (and `[::1]:1455`)
/// to capture the OAuth 2.0 PKCE browser redirect (`/auth/callback?code=...&state=...`)
/// and push it directly to the active WebviewWindow via CustomEvent.
pub fn spawn_oauth_loopback_listener(window: tauri::WebviewWindow) {
    let generation = OAUTH_LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    std::thread::spawn(move || {
        // Give any previous listener up to 120ms to release port 1455.
        std::thread::sleep(Duration::from_millis(80));
        if OAUTH_LISTENER_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }

        let listener_v4 = TcpListener::bind("127.0.0.1:1455").ok();
        if let Some(ref l) = listener_v4 {
            let _ = l.set_nonblocking(true);
        }
        let listener_v6 = TcpListener::bind("[::1]:1455").ok();
        if let Some(ref l) = listener_v6 {
            let _ = l.set_nonblocking(true);
        }

        if listener_v4.is_none() && listener_v6.is_none() {
            return;
        }

        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            if OAUTH_LISTENER_GENERATION.load(Ordering::SeqCst) != generation {
                break;
            }

            let accepted = listener_v4
                .as_ref()
                .and_then(|l| l.accept().ok())
                .or_else(|| listener_v6.as_ref().and_then(|l| l.accept().ok()));

            if let Some((mut stream, _addr)) = accepted {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let _method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");

                if !target.starts_with("/auth/callback")
                    && !target.contains("code=")
                    && !target.contains("error=")
                {
                    let not_found =
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(not_found);
                    continue;
                }

                let callback_url = format!("http://localhost:1455{}", target);
                let html_body = concat!(
                    "<!doctype html><html><head><meta charset=\"utf-8\">",
                    "<title>EOStage 授权完成</title></head>",
                    "<body style=\"font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;",
                    "display:flex;align-items:center;justify-content:center;height:100vh;margin:0;",
                    "background:#f8fafc;color:#0f172a;\">",
                    "<div style=\"background:#ffffff;padding:32px 40px;border-radius:14px;",
                    "border:1px solid #e2e8f0;box-shadow:0 10px 30px rgba(15,23,42,0.08);text-align:center;max-width:420px;\">",
                    "<div style=\"font-size:28px;margin-bottom:12px;\">&#x2705;</div>",
                    "<h2 style=\"margin:0 0 10px;font-size:18px;font-weight:600;\">EOStage 浏览器授权已完成</h2>",
                    "<p style=\"margin:0;color:#475569;font-size:13px;line-height:1.6;\">",
                    "授权回调已自动同步至 EOStage 客户端并为您创建新渠道配置，您可以直接关闭此浏览器标签页。",
                    "</p></div>",
                    "<script>setTimeout(function(){try{window.close();}catch(e){}},1200);</script>",
                    "</body></html>"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                    html_body.len(),
                    html_body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();

                if let Ok(serialized_url) = serde_json::to_string(&callback_url) {
                    let js = format!(
                        "window.__EOSTAGE_LAST_OAUTH_CALLBACK__ = {u}; window.dispatchEvent(new CustomEvent('eostage:oauth-callback', {{ detail: {{ url: {u} }} }}));",
                        u = serialized_url
                    );
                    let _ = window.set_focus();
                    let _ = window.eval(&js);
                }
                break;
            }

            std::thread::sleep(Duration::from_millis(120));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{is_external_browser_open_disabled, open_in_default_browser_with_flag};

    #[test]
    fn disable_external_browser_env_guard_returns_ok_without_spawning() {
        assert!(is_external_browser_open_disabled(Some("1")));
        assert!(is_external_browser_open_disabled(Some("true")));
        assert!(!is_external_browser_open_disabled(None));
        assert!(!is_external_browser_open_disabled(Some("0")));
        assert!(
            open_in_default_browser_with_flag("https://api.eostage.org/auth/login", true).is_ok()
        );
    }
}
