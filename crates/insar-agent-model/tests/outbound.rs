use insar_agent_model::{transport::ModelRoute, Message, Protocol};

#[test]
fn request_scrubs_sensitive_payloads_before_transport_or_trace() {
    let route =
        ModelRoute::new(Protocol::ChatCompletions, "https://example.invalid/v1", "m").unwrap();
    let message = Message { role: "user".into(), content: "Read C:\\Users\\owner\\secret.txt and https://example.invalid/search?api_key=fake-secret ; Authorization: Bearer fake-secret".into() };
    let body = route.request(&[message], &[]).unwrap().1.to_string();
    assert!(!body.contains("owner"), "local path leaked");
    assert!(!body.contains("api_key"), "URL query leaked");
    assert!(!body.contains("fake-secret"), "credential leaked");
    assert!(body.contains("example.invalid/search"));
}

#[test]
fn outbound_scrubbing_drops_url_userinfo() {
    let safe = insar_agent_model::privacy::scrub_text(
        "see (https://user:fake-pw@example.invalid/a?q=1) and http://u@example.invalid",
    );
    assert_eq!(
        safe,
        "see (https://example.invalid/a) and http://example.invalid"
    );
}

#[test]
fn outbound_scrubbing_preserves_whitespace_and_removes_punctuated_paths() {
    let text = "a\n(https://example.invalid/search?q=secret)\nC:\\Users\\owner\\data.txt,\n\t/home/owner/a";
    let safe = insar_agent_model::privacy::scrub_text(text);
    assert!(safe.contains("a\n("));
    assert!(safe.contains("example.invalid/search)"));
    assert!(!safe.contains("q=secret"));
    assert!(!safe.contains("owner"));
}

/// Second review of 2026-11-02 (privacy-first): a single-segment CJK path
/// glued to Chinese text, a URL-encoded path and another user's home are
/// replaced; `他/她` is a false positive the ruling accepts; a digit after
/// `%2F` is not a path. The scrub replaces the whole whitespace-delimited
/// token, as it does for any path glued to the text before it.
#[test]
fn outbound_scrubbing_removes_glued_encoded_and_tilde_home_paths() {
    for (text, gone) in [
        ("下载到/数据 里", "/数据"),
        ("保存到/下载 里", "/下载"),
        ("在/桌面", "/桌面"),
        ("下载到/数据x", "/数据x"),
        ("下载到/数据.zip", "/数据.zip"),
        ("他/她", "她"),
        ("下载到%2Fhome%2Fme%2Fx 里", "home"),
        ("下载到%2F%E4%B8%8B%E8%BD%BD 里", "%E4"),
        ("保存到%2f%e4%b8%8b", "%e4"),
        ("下载到~zhangsan/out 里", "zhangsan"),
    ] {
        let safe = insar_agent_model::privacy::scrub_text(text);
        assert!(!safe.contains(gone), "{text}: {safe}");
        assert!(safe.contains("[local-reference]"), "{text}: {safe}");
    }
    // A digit after the encoded separator, and `~` without a following
    // `/`, stay as written.
    assert_eq!(
        insar_agent_model::privacy::scrub_text("查询 ?a=1%2F2 的结果"),
        "查询 ?a=1%2F2 的结果"
    );
    assert_eq!(
        insar_agent_model::privacy::scrub_text("大约 ~30 个"),
        "大约 ~30 个"
    );
    // A space or another `/` after the code is not a path either (third
    // review).
    assert_eq!(insar_agent_model::privacy::scrub_text("%2F%20x"), "%2F%20x");
    assert_eq!(insar_agent_model::privacy::scrub_text("%2F%2Fy"), "%2F%2Fy");
}
