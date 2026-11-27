use insar_agent::run::{delete_run, journal, mark_interrupted, read_sealed, safe_value, RunWriter};
use insar_store::{AgentRunRow, TaskStore};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn setup(label: &str) -> (std::path::PathBuf, TaskStore, AgentRunRow) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("agent-runs-{}-{label}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let store = TaskStore::open(root.join("store.db")).unwrap();
    let row = AgentRunRow {
        run_id: "run-fixture".into(),
        session_id: "session-fixture".into(),
        origin: "manual".into(),
        client_label: None,
        started_at: "2026-10-23T09:00:00Z".into(),
        ended_at: None,
        status: "running".into(),
        model_route: None,
        catalog_version: 1,
        catalog_sha256: "a".repeat(64),
        app_version: "test".into(),
        source_commit: "fixture".into(),
        store_revision_start: 0,
        store_revision_end: None,
        record_dir: "agent/runs/run-fixture".into(),
        trace_sha256: None,
        reproduces_run_id: None,
        reasoning_off_refused: false,
        truncated: false,
        usage_prompt: None,
        usage_completion: None,
    };
    (root, store, row)
}

#[test]
fn trace_is_hashed_sealed_and_journal_is_deterministic() {
    let (root, mut store, row) = setup("trace");
    let mut writer = RunWriter::start(&root, &mut store, &row).unwrap();
    writer
        .append("request", &json!({"text":"prepare SAR","ref":"batch-1"}))
        .unwrap();
    writer
        .outbound(&json!({"model":"fixture","text":"prepare SAR"}))
        .unwrap();
    writer
        .append(
            "verification",
            &json!({"task_id":"task-1","sha256":format!("{:x}",Sha256::digest(b"ok"))}),
        )
        .unwrap();
    let seal = writer
        .finish(
            &mut store,
            "run-fixture",
            "completed",
            "2026-10-23T09:01:00Z",
            0,
        )
        .unwrap();
    let events = read_sealed(&root, &store, "run-fixture").unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(seal, events.last().unwrap().sha256);
    let text = journal(&events);
    assert_eq!(
        fs::read_to_string(root.join("agent/runs/run-fixture/journal.md")).unwrap(),
        text
    );
    assert_eq!(text, journal(&events));
    let trace_path = root.join("agent/runs/run-fixture/trace.jsonl");
    fs::write(
        &trace_path,
        fs::read_to_string(&trace_path)
            .unwrap()
            .replace("prepare SAR", "pretend complete"),
    )
    .unwrap();
    assert!(read_sealed(&root, &store, "run-fixture").is_err());
}

#[test]
fn never_write_sensitive_request_or_local_absolute_path() {
    let (root, mut store, row) = setup("redact");
    let mut writer = RunWriter::start(&root, &mut store, &row).unwrap();
    let raw = json!({"url":"https://example.test/search?signature=not-a-real-token","note":"public resource"});
    writer.outbound(&raw).unwrap();
    for dangerous in [
        json!({"token":"not-a-real-token"}),
        json!({"text":"Bearer not-a-real-token"}),
        json!({"path":"C:\\Users\\someone\\data"}),
        json!({"path":"/home/someone/data"}),
    ] {
        assert!(writer.outbound(&dangerous).is_err());
        assert!(writer.append("request", &dangerous).is_err());
    }
    let serialized =
        fs::read_to_string(root.join("agent/runs/run-fixture/outbound/000000.json")).unwrap();
    assert!(serialized.contains("https://example.test/search"));
    assert!(!serialized.contains("signature"));
    assert!(!serialized.contains("not-a-real-token"));
    assert_eq!(
        safe_value(&raw).unwrap()["url"],
        "https://example.test/search"
    );
    assert!(
        writer
            .outbound(&json!({"note":"asset?signature=not-a-real-key"}))
            .is_err(),
        "embedded URL queries must never enter the request archive"
    );
}

#[test]
fn restart_marks_running_as_interrupted_without_replaying_or_deleting_tasks() {
    let (root, mut store, row) = setup("interrupt");
    let mut writer = RunWriter::start(&root, &mut store, &row).unwrap();
    writer.append("request", &json!({"text":"test"})).unwrap();
    drop(writer);
    let ids = mark_interrupted(&root, &mut store, "2026-10-23T09:01:00Z").unwrap();
    assert_eq!(ids, ["run-fixture"]);
    assert_eq!(
        store.agent_run("run-fixture").unwrap().unwrap().status,
        "interrupted"
    );
    assert_eq!(read_sealed(&root, &store, "run-fixture").unwrap().len(), 1);
    assert!(root.join("agent/runs/run-fixture/journal.md").exists());
    assert!(mark_interrupted(&root, &mut store, "2026-10-23T09:02:00Z")
        .unwrap()
        .is_empty());
    let output = root.join("download.bin");
    fs::write(&output, b"owner fixture output").unwrap();
    delete_run(&root, &mut store, "run-fixture").unwrap();
    assert!(store.agent_run("run-fixture").unwrap().is_none());
    assert!(
        !root.join("agent/runs/run-fixture").exists(),
        "run deletion must remove its owned records"
    );
    assert_eq!(
        fs::read(&output).unwrap(),
        b"owner fixture output",
        "run deletion must leave task output untouched"
    );
}

#[test]
fn scrubbed_request_bodies_are_archivable_but_raw_secrets_are_not() {
    for archivable in [
        json!({"max_tokens": 4096}),
        json!({"content":"see https://example.test/data for details"}),
        json!({"content":"password=[redacted] and Bearer [redacted]"}),
    ] {
        assert_eq!(safe_value(&archivable).unwrap(), archivable);
    }
    for refused in [
        json!({"content":"see https://example.test/data?sig=x for details"}),
        json!({"content":"password=[redacted] then password=real"}),
        json!({"content":"Bearer [redacted] Bearer real"}),
        json!({"content":"open file:///C:/data"}),
        json!({"content":"at https://example.test and C:/Users/x"}),
        json!({"refresh_token": 1}),
    ] {
        assert!(safe_value(&refused).is_err(), "{refused}");
    }
}

/// Second review (2026-11-02, privacy-first ruling): a single-segment CJK
/// path glued to Chinese text, a URL-encoded path and another user's home
/// directory are refused; the words and dates that merely hold a `/` stay,
/// and `他/她` is a false positive the ruling accepts.
#[test]
fn glued_cjk_encoded_and_tilde_home_paths_are_refused() {
    for refused in [
        json!({"content":"下载到/数据"}),
        json!({"content":"保存到/下载"}),
        json!({"content":"在/桌面"}),
        json!({"content":"下载到/数据x"}),
        json!({"content":"下载到/数据.zip"}),
        json!({"content":"他/她"}),
        json!({"content":"下载到%2Fhome%2Fme%2Fx"}),
        json!({"content":"下载到%2fHome 里"}),
        json!({"content":"下载到%2F%E4%B8%8B%E8%BD%BD"}),
        json!({"content":"保存到%2f%e4%b8%8b"}),
        json!({"content":"下载到~zhangsan/out"}),
        json!({"content":"保存到~root.dir/x_data"}),
    ] {
        assert!(safe_value(&refused).is_err(), "{refused}");
    }
    for kept in [
        json!({"content":"见 https://a/b 下载"}),
        json!({"content":"TCP/IP 配置"}),
        json!({"content":"比例 1/2"}),
        json!({"content":"日期 2026/11/02"}),
        json!({"content":"24/7"}),
        // A digit, a space or another `/` after the encoded separator is
        // not a path.
        json!({"content":"查询 ?a=1%2F2 的结果"}),
        json!({"content":"查询 %2F%20x 和 %2F%2Fy"}),
        // `~` without a following `/` is not a path.
        json!({"content":"大约 ~30 个"}),
    ] {
        assert_eq!(safe_value(&kept).unwrap(), kept, "{kept}");
    }
}

#[test]
fn a_leading_url_does_not_exempt_the_rest_of_the_text() {
    for refused in [
        // A leading address must not skip the local path check.
        json!({"content":"https://example.test/a D:/private/x.tif"}),
        // Userinfo carries credentials, in prose or as the whole value.
        json!({"content":"https://user:pw@example.test/a"}),
        json!({"content":"see https://user@example.test/a for details"}),
        // A fragment must not truncate (and hide) the following prose.
        json!({"content":"https://example.test/a#sig=abc 其余文字"}),
        json!({"content":"https://example.test/a?x=1 signature=abc"}),
    ] {
        assert!(safe_value(&refused).is_err(), "{refused}");
    }
    // A lone address still keeps only its path part.
    assert_eq!(
        safe_value(&json!("https://example.test/a?x=1#f")).unwrap(),
        json!("https://example.test/a")
    );
}

#[test]
fn run_record_and_extra_usage_preserve_cached_and_reasoning_counts() {
    let (root, mut store, row) = setup("cached-usage");
    let mut writer = RunWriter::start(&root, &mut store, &row).unwrap();
    writer
        .append(
            "model_reply",
            &json!({
                "reasoning_off_refused": false,
                "truncated": false,
                "usage": {
                    "prompt": 2000,
                    "completion": 150,
                    "total": 2150,
                    "cached": 1500,
                    "cache_creation": 200,
                    "reasoning": 40
                }
            }),
        )
        .unwrap();
    writer
        .finish(
            &mut store,
            "run-fixture",
            "completed",
            "2026-10-23T09:01:00Z",
            0,
        )
        .unwrap();
    let (cached, cache_creation, reasoning) =
        insar_agent::run::run_extra_usage(&root, "run-fixture");
    assert_eq!(cached, Some(1500));
    assert_eq!(cache_creation, Some(200));
    assert_eq!(reasoning, Some(40));

    let record_bytes = fs::read(root.join("agent/runs/run-fixture/run.json")).unwrap();
    let record: insar_agent::run::RunRecord = serde_json::from_slice(&record_bytes).unwrap();
    assert_eq!(record.usage_cached_input, Some(1500));
    assert_eq!(record.usage_cache_creation_input, Some(200));
    assert_eq!(record.usage_reasoning, Some(40));
}
