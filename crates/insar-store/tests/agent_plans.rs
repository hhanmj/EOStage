use insar_store::{AgentApprovalRow, AgentPlanRow, TaskStore, STORE_SCHEMA_VERSION};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn schema_32_adds_plans_and_approvals_without_replacing_task_authority() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agent-plans-{}-{nonce}.db", std::process::id()));
    let store = TaskStore::open(&path).unwrap();
    let tables = store.table_names().unwrap();
    assert_eq!(STORE_SCHEMA_VERSION, 43);
    assert!(tables.contains(&"agent_plans".to_string()));
    assert!(tables.contains(&"agent_approvals".to_string()));
    assert!(
        tables.contains(&"agent_runs".to_string()),
        "each run needs an index and integrity anchor"
    );
    assert!(tables.contains(&"task_runtime".to_string()));
}

#[test]
fn v31_to_v32_adds_only_agent_tables_and_older_writer_rejects_new_schema() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agent-v31-{}-{nonce}.db", std::process::id()));
    let store = TaskStore::open(&path).unwrap();
    let old_tables = store.table_names().unwrap();
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("INSERT INTO search_batches(batch_id,provider_id,created_at,snapshot_json) VALUES('frozen-batch','fixture','2026-10-23T09:00:00Z','{\"fixture\":true}'); DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers; PRAGMA user_version = 31;").unwrap();
    drop(connection);
    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    let tables = store.table_names().unwrap();
    assert_eq!(
        tables
            .iter()
            .filter(|table| !table.starts_with("agent_"))
            .collect::<Vec<_>>(),
        old_tables
            .iter()
            .filter(|table| !table.starts_with("agent_"))
            .collect::<Vec<_>>()
    );
    drop(store);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM search_batches WHERE batch_id='frozen-batch'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let old_writer_supported_version = 31;
    assert!(
        connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap()
            > old_writer_supported_version
    );
}

#[test]
fn store_rejects_plan_hash_that_does_not_match_its_canonical_bytes() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agent-hash-{}-{nonce}.db", std::process::id()));
    let mut store = TaskStore::open(&path).unwrap();
    let json = "{\"steps\":[{\"tool_id\":\"task_page\"}]}";
    let row = AgentPlanRow {
        plan_id: "p".into(),
        session_id: "s".into(),
        run_id: "r".into(),
        created_at: "now".into(),
        model_route: "manual".into(),
        plan_json: json.into(),
        plan_hash: format!("{:x}", Sha256::digest(json.as_bytes())),
        max_tier: 0,
        status: "proposed".into(),
        expires_at: "tomorrow".into(),
        store_revision_seen: 0,
    };
    store.save_agent_plan(&row).unwrap();
    let mut tampered = row.clone();
    tampered.plan_id = "p2".into();
    tampered.plan_hash = "a".repeat(64);
    assert!(store.save_agent_plan(&tampered).is_err());
}

#[test]
fn deleting_session_plans_removes_only_that_sessions_plans_and_approvals() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "agent-session-del-{}-{nonce}.db",
        std::process::id()
    ));
    let mut store = TaskStore::open(&path).unwrap();
    let json = "{\"steps\":[{\"tool_id\":\"task_page\"}]}";
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    for (plan_id, session_id) in [("p1", "s1"), ("p2", "s1"), ("p3", "s2")] {
        store
            .save_agent_plan(&AgentPlanRow {
                plan_id: plan_id.into(),
                session_id: session_id.into(),
                run_id: format!("run-{plan_id}"),
                created_at: "2026-10-24T00:00:00Z".into(),
                model_route: "manual".into(),
                plan_json: json.into(),
                plan_hash: hash.clone(),
                max_tier: 0,
                status: "proposed".into(),
                expires_at: "2999-01-01T00:00:00Z".into(),
                store_revision_seen: 0,
            })
            .unwrap();
        store
            .decide_agent_plan(
                &AgentApprovalRow {
                    approval_id: format!("approval-{plan_id}"),
                    plan_id: plan_id.into(),
                    plan_hash_seen: hash.clone(),
                    snapshot_json: "{\"output_dir\":\"out\"}".into(),
                    decision: "approved".into(),
                    decided_at: "2026-10-24T00:00:01Z".into(),
                    decided_via: "desktop_card".into(),
                    mode: None,
                    rewritten_from_plan_hash: None,
                },
                &hash,
                0,
                "2026-10-24T00:00:01Z",
            )
            .unwrap();
    }
    assert_eq!(store.delete_agent_session_plans("s1").unwrap(), 2);
    for plan_id in ["p1", "p2"] {
        assert!(store.agent_plan(plan_id).unwrap().is_none());
        assert!(store.agent_approval(plan_id).unwrap().is_none());
    }
    assert!(store.agent_plan("p3").unwrap().is_some());
    assert!(store.agent_approval("p3").unwrap().is_some());
    assert_eq!(store.delete_agent_session_plans("s1").unwrap(), 0);
}

fn temp_store(label: &str) -> (std::path::PathBuf, TaskStore) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("agent-{label}-{}-{nonce}.db", std::process::id()));
    let store = TaskStore::open(&path).unwrap();
    (path, store)
}

/// A proposed tier 2 plan of a run with the given origin; returns the plan hash.
fn seed_plan(store: &mut TaskStore, plan_id: &str, origin: &str) -> String {
    seed_plan_tier(store, plan_id, origin, 2)
}

fn seed_plan_tier(store: &mut TaskStore, plan_id: &str, origin: &str, max_tier: u8) -> String {
    let run_id = format!("run-{plan_id}");
    store
        .start_agent_run(&insar_store::AgentRunRow {
            run_id: run_id.clone(),
            session_id: "session-a".into(),
            origin: origin.into(),
            client_label: None,
            started_at: "2026-10-26T00:00:00Z".into(),
            ended_at: None,
            status: "running".into(),
            model_route: None,
            catalog_version: 1,
            catalog_sha256: "0".repeat(64),
            app_version: "test".into(),
            source_commit: "test".into(),
            store_revision_start: 0,
            store_revision_end: None,
            record_dir: format!("agent/runs/{run_id}"),
            trace_sha256: None,
            reproduces_run_id: None,
            reasoning_off_refused: false,
            truncated: false,
            usage_prompt: None,
            usage_completion: None,
        })
        .unwrap();
    let json = if max_tier == 1 {
        "{\"steps\":[{\"tool_id\":\"search_start\"}]}"
    } else {
        "{\"steps\":[{\"tool_id\":\"create_task\"}]}"
    };
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    store
        .save_agent_plan(&AgentPlanRow {
            plan_id: plan_id.into(),
            session_id: "session-a".into(),
            run_id,
            created_at: "2026-10-26T00:00:00Z".into(),
            model_route: "manual".into(),
            plan_json: json.into(),
            plan_hash: hash.clone(),
            max_tier,
            status: "proposed".into(),
            expires_at: "2999-01-01T00:00:00Z".into(),
            store_revision_seen: 0,
        })
        .unwrap();
    hash
}

fn decision(plan_id: &str, hash: &str, via: &str, mode: Option<&str>) -> AgentApprovalRow {
    AgentApprovalRow {
        approval_id: format!("approval-{plan_id}"),
        plan_id: plan_id.into(),
        plan_hash_seen: hash.into(),
        snapshot_json: "{\"source\":\"fixture\",\"output_dir\":\"out\"}".into(),
        decision: "approved".into(),
        decided_at: "2026-10-26T00:00:01Z".into(),
        decided_via: via.into(),
        mode: mode.map(str::to_owned),
        rewritten_from_plan_hash: None,
    }
}

#[test]
fn approvals_record_the_mode_in_effect_and_whether_the_mode_approved() {
    let (_path, mut store) = temp_store("auto-by-mode");
    let hash = seed_plan(&mut store, "plan-auto", "in_app");
    store
        .decide_agent_plan(
            &decision("plan-auto", &hash, "auto_by_mode", Some("full_access")),
            &hash,
            0,
            "2026-10-26T00:00:01Z",
        )
        .expect("an in-app plan can be approved by the permission mode");
    let row = store.agent_approval("plan-auto").unwrap().unwrap();
    assert_eq!(row.decided_via, "auto_by_mode");
    assert_eq!(row.mode.as_deref(), Some("full_access"));
    assert_eq!(row.decision, "approved");
    assert_eq!(row.decided_at, "2026-10-26T00:00:01Z");
    assert_eq!(
        store.agent_plan("plan-auto").unwrap().unwrap().status,
        "approved"
    );
    // A desktop card decision records its mode too.
    let hash = seed_plan(&mut store, "plan-card", "in_app");
    store
        .decide_agent_plan(
            &decision("plan-card", &hash, "desktop_card", Some("confirm_changes")),
            &hash,
            0,
            "2026-10-26T00:00:01Z",
        )
        .unwrap();
    let row = store.agent_approval("plan-card").unwrap().unwrap();
    assert_eq!(row.decided_via, "desktop_card");
    assert_eq!(row.mode.as_deref(), Some("confirm_changes"));
    assert_eq!(row.rewritten_from_plan_hash, None);
}

#[test]
fn an_approval_by_mode_of_a_tier_2_plan_needs_auto_approve_or_full_access() {
    let (_path, mut store) = temp_store("auto-invalid");
    let cases = [
        ("auto_by_mode", Some("confirm_changes"), "approved"),
        ("auto_by_mode", Some("per_item"), "approved"),
        ("auto_by_mode", Some("within_quota"), "approved"),
        ("auto_by_mode", None, "approved"),
        ("auto_by_mode", Some("full_access"), "rejected"),
        ("model", Some("full_access"), "approved"),
        ("desktop_card", Some("anything"), "approved"),
    ];
    for (index, (via, mode, word)) in cases.into_iter().enumerate() {
        let plan_id = format!("plan-{index}");
        let hash = seed_plan(&mut store, &plan_id, "in_app");
        let mut row = decision(&plan_id, &hash, via, mode);
        row.decision = word.into();
        assert!(
            store
                .decide_agent_plan(&row, &hash, 0, "2026-10-26T00:00:01Z")
                .is_err(),
            "{via} {mode:?} {word} must be refused"
        );
        assert!(store.agent_approval(&plan_id).unwrap().is_none());
        assert_eq!(
            store.agent_plan(&plan_id).unwrap().unwrap().status,
            "proposed"
        );
    }
}

/// A1 §6.1 is unchanged by the 2026-10-25 permission modes: a plan of an MCP
/// run is approved on the desktop card only, in every mode.
#[test]
fn a_plan_from_an_mcp_run_is_never_approved_by_mode() {
    let (_path, mut store) = temp_store("auto-mcp");
    let hash = seed_plan(&mut store, "plan-mcp", "mcp");
    for mode in ["auto_approve", "full_access"] {
        assert!(store
            .decide_agent_plan(
                &decision("plan-mcp", &hash, "auto_by_mode", Some(mode)),
                &hash,
                0,
                "2026-10-26T00:00:01Z",
            )
            .is_err());
    }
    assert!(store.agent_approval("plan-mcp").unwrap().is_none());
    store
        .decide_agent_plan(
            &decision("plan-mcp", &hash, "desktop_card", Some("full_access")),
            &hash,
            0,
            "2026-10-26T00:00:01Z",
        )
        .expect("the desktop card still approves an MCP plan");
    let row = store.agent_approval("plan-mcp").unwrap().unwrap();
    assert_eq!(row.decided_via, "desktop_card");
}

const APPROVALS_V33_SQL: &str = "DROP TABLE agent_approvals;
CREATE TABLE agent_approvals (
    approval_id TEXT PRIMARY KEY NOT NULL CHECK(length(approval_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
    plan_hash_seen TEXT NOT NULL CHECK(length(plan_hash_seen) = 64),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
    decided_at TEXT NOT NULL,
    decided_via TEXT NOT NULL CHECK(decided_via = 'desktop_card')
) STRICT;
CREATE INDEX agent_approvals_plan_idx ON agent_approvals(plan_id, decided_at);";

#[test]
fn v33_approvals_are_rebuilt_in_place_and_the_upgrade_is_reentrant() {
    let (path, mut store) = temp_store("v33-approvals");
    let hash = seed_plan(&mut store, "plan-old", "in_app");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(&format!(
            "{APPROVALS_V33_SQL}
             INSERT INTO agent_approvals VALUES('approval-old','plan-old','{hash}','{{}}','approved','2026-10-25T00:00:00Z','desktop_card');
             UPDATE agent_plans SET status='approved' WHERE plan_id='plan-old';
             PRAGMA user_version = 33;"
        ))
        .unwrap();
    // The schema 33 table refuses an approval by mode.
    assert!(connection
        .execute_batch(&format!(
            "INSERT INTO agent_approvals VALUES('approval-x','plan-old','{hash}','{{}}','approved','2026-10-25T00:00:01Z','auto_by_mode');"
        ))
        .is_err());
    drop(connection);

    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    let kept = store.agent_approval("plan-old").unwrap().unwrap();
    assert_eq!(kept.approval_id, "approval-old");
    assert_eq!(kept.decided_via, "desktop_card");
    assert_eq!(
        kept.mode, None,
        "rows decided before schema 34 have no mode"
    );
    drop(store);

    let connection = Connection::open(&path).unwrap();
    let insert = |id: &str, via: &str, word: &str, mode: &str| {
        connection.execute_batch(&format!(
            "INSERT INTO agent_approvals(approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,mode) VALUES('{id}','plan-old','{hash}','{{}}','{word}','2026-10-26T00:00:00Z','{via}',{mode});"
        ))
    };
    assert!(insert("a1", "auto_by_mode", "approved", "'auto_approve'").is_ok());
    assert!(insert("a0", "auto_by_mode", "approved", "'within_quota'").is_err());
    assert!(insert("a2", "auto_by_mode", "approved", "'per_item'").is_err());
    assert!(insert("a3", "auto_by_mode", "rejected", "'full_access'").is_err());
    assert!(insert("a4", "auto_by_mode", "approved", "NULL").is_err());
    assert!(insert("a5", "model", "approved", "'full_access'").is_err());
    assert!(insert("a6", "desktop_card", "approved", "'sideways'").is_err());
    let indexes: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='agent_approvals_plan_idx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 1);
    // A repeated or faked upgrade over the new table converges.
    connection
        .execute_batch("PRAGMA user_version = 33;")
        .unwrap();
    drop(connection);
    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    let newest = store.agent_approval("plan-old").unwrap().unwrap();
    assert_eq!(newest.approval_id, "a1", "rows survive a repeated upgrade");
    assert_eq!(newest.mode.as_deref(), Some("auto_approve"));
}

/// The schema 34 table as W3e created it (the fixture a schema 35 build
/// upgrades).
const APPROVALS_V34_SQL: &str = "DROP TABLE agent_approvals;
CREATE TABLE agent_approvals (
    approval_id TEXT PRIMARY KEY NOT NULL CHECK(length(approval_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
    plan_hash_seen TEXT NOT NULL CHECK(length(plan_hash_seen) = 64),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
    decided_at TEXT NOT NULL,
    decided_via TEXT NOT NULL CHECK(decided_via IN ('desktop_card','auto_by_mode')),
    mode TEXT CHECK(mode IS NULL OR mode IN ('per_item','within_quota','full_access')),
    CHECK(decided_via = 'desktop_card' OR (decision = 'approved' AND mode IS NOT NULL AND mode IN ('within_quota','full_access')))
) STRICT;
CREATE INDEX agent_approvals_plan_idx ON agent_approvals(plan_id, decided_at);";

/// Owner 2026-10-26: schema 35 renames the stored W3e modes (per_item ->
/// confirm_changes, within_quota -> auto_approve) and keeps every row, its
/// snapshot (a card without output_dir_source) and a NULL mode as they were.
#[test]
fn schema_35_renames_stored_modes_and_keeps_old_rows_readable() {
    let (path, mut store) = temp_store("v34-modes");
    let mut hashes = Vec::new();
    for plan_id in ["p-item", "p-quota", "p-full", "p-null"] {
        hashes.push(seed_plan(&mut store, plan_id, "in_app"));
    }
    drop(store);
    let old_card = r#"{"source":"asf.sentinel1","range":{"aoi_name":"a","area_km2":1.0,"time_range":null,"product":"1 个产品"},"estimated_bytes":5,"output_dir":"D:/out"}"#;
    let connection = Connection::open(&path).unwrap();
    let mut sql = format!("{APPROVALS_V34_SQL}\n");
    for ((plan_id, via, mode), hash) in [
        ("p-item", "desktop_card", "'per_item'"),
        ("p-quota", "auto_by_mode", "'within_quota'"),
        ("p-full", "auto_by_mode", "'full_access'"),
        ("p-null", "desktop_card", "NULL"),
    ]
    .into_iter()
    .zip(&hashes)
    {
        sql.push_str(&format!(
            "INSERT INTO agent_approvals VALUES('approval-{plan_id}','{plan_id}','{hash}','{old_card}','approved','2026-10-25T00:00:00Z','{via}',{mode});\n"
        ));
    }
    sql.push_str("PRAGMA user_version = 34;");
    connection.execute_batch(&sql).unwrap();
    drop(connection);

    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    for (plan_id, via, mode) in [
        ("p-item", "desktop_card", Some("confirm_changes")),
        ("p-quota", "auto_by_mode", Some("auto_approve")),
        ("p-full", "auto_by_mode", Some("full_access")),
        ("p-null", "desktop_card", None),
    ] {
        let row = store.agent_approval(plan_id).unwrap().unwrap();
        assert_eq!(row.approval_id, format!("approval-{plan_id}"));
        assert_eq!(row.decided_via, via, "{plan_id}");
        assert_eq!(row.mode.as_deref(), mode, "{plan_id}");
        assert_eq!(row.rewritten_from_plan_hash, None, "{plan_id}");
        assert_eq!(
            row.snapshot_json, old_card,
            "{plan_id}: the snapshot is untouched"
        );
    }
    drop(store);
    // A repeated or faked upgrade converges and keeps the renamed rows.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 34;")
        .unwrap();
    drop(connection);
    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert_eq!(
        store
            .agent_approval("p-quota")
            .unwrap()
            .unwrap()
            .mode
            .as_deref(),
        Some("auto_approve")
    );
    drop(store);
    let connection = Connection::open(&path).unwrap();
    let insert = |id: &str, via: &str, mode: &str, from: &str| {
        connection.execute_batch(&format!(
            "INSERT INTO agent_approvals(approval_id,plan_id,plan_hash_seen,snapshot_json,decision,decided_at,decided_via,mode,rewritten_from_plan_hash) VALUES('{id}','p-null','{}','{{}}','approved','2026-10-26T00:00:00Z','{via}',{mode},{from});",
            hashes[3]
        ))
    };
    let other = format!("'{}'", "b".repeat(64));
    assert!(insert("n1", "desktop_card", "'confirm_changes'", &other).is_ok());
    assert!(
        insert("n2", "auto_by_mode", "'auto_approve'", &other).is_err(),
        "a rewrite is a card approval"
    );
    assert!(insert("n3", "desktop_card", "'per_item'", "NULL").is_err());
    assert!(insert("n4", "desktop_card", "'full_access'", "'short'").is_err());
}

/// confirm_changes runs a disclosed search (tier 1) without a click, so the
/// Store accepts an approval by mode under it for a tier 1 plan only.
#[test]
fn confirm_changes_approves_by_mode_only_a_tier_1_plan() {
    let (_path, mut store) = temp_store("confirm-tier");
    let search = seed_plan_tier(&mut store, "plan-search", "in_app", 1);
    store
        .decide_agent_plan(
            &decision(
                "plan-search",
                &search,
                "auto_by_mode",
                Some("confirm_changes"),
            ),
            &search,
            0,
            "2026-10-26T00:00:01Z",
        )
        .expect("a disclosed search runs under confirm_changes");
    let create = seed_plan(&mut store, "plan-create", "in_app");
    assert!(store
        .decide_agent_plan(
            &decision(
                "plan-create",
                &create,
                "auto_by_mode",
                Some("confirm_changes")
            ),
            &create,
            0,
            "2026-10-26T00:00:01Z",
        )
        .is_err());
    assert!(store.agent_approval("plan-create").unwrap().is_none());
}

/// A card that chose another output directory rewrites the plan and approves
/// it in one transaction; the approval keeps the shown plan's hash.
#[test]
fn a_rewritten_plan_replaces_the_shown_plan_and_keeps_its_hash() {
    let (_path, mut store) = temp_store("rewrite");
    let shown = seed_plan(&mut store, "plan-r", "in_app");
    let rewritten = "{\"steps\":[{\"tool_id\":\"create_task\",\"x\":1}]}";
    let rewritten_hash = format!("{:x}", Sha256::digest(rewritten.as_bytes()));
    let mut approval = decision(
        "plan-r",
        &rewritten_hash,
        "desktop_card",
        Some("confirm_changes"),
    );
    approval.rewritten_from_plan_hash = Some(shown.clone());
    // A rewrite by mode, or a hash that is not the rewritten plan's, is
    // refused, and the plan is still the shown one, awaiting its card.
    let mut by_mode = approval.clone();
    by_mode.decided_via = "auto_by_mode".into();
    let mut wrong_hash = approval.clone();
    wrong_hash.plan_hash_seen = "c".repeat(64);
    for bad in [&by_mode, &wrong_hash] {
        assert!(store
            .decide_rewritten_agent_plan(bad, &shown, rewritten, 0, "2026-10-26T00:00:01Z")
            .is_err());
        let plan = store.agent_plan("plan-r").unwrap().unwrap();
        assert_eq!(
            (plan.plan_hash.as_str(), plan.status.as_str()),
            (shown.as_str(), "proposed")
        );
    }
    // A plain decision cannot claim a rewrite.
    let mut claims = decision("plan-r", &shown, "desktop_card", Some("confirm_changes"));
    claims.rewritten_from_plan_hash = Some(shown.clone());
    assert!(store
        .decide_agent_plan(&claims, &shown, 0, "2026-10-26T00:00:01Z")
        .is_err());

    store
        .decide_rewritten_agent_plan(&approval, &shown, rewritten, 0, "2026-10-26T00:00:01Z")
        .unwrap();
    let plan = store.agent_plan("plan-r").unwrap().unwrap();
    assert_eq!(plan.plan_json, rewritten);
    assert_eq!(plan.plan_hash, rewritten_hash);
    assert_eq!(plan.status, "approved");
    let row = store.agent_approval("plan-r").unwrap().unwrap();
    assert_eq!(row.plan_hash_seen, rewritten_hash);
    assert_eq!(
        row.rewritten_from_plan_hash.as_deref(),
        Some(shown.as_str())
    );

    // A rewrite of a plan the card did not show expires it.
    let other = seed_plan(&mut store, "plan-s", "in_app");
    let mut stale = decision(
        "plan-s",
        &rewritten_hash,
        "desktop_card",
        Some("confirm_changes"),
    );
    stale.rewritten_from_plan_hash = Some("d".repeat(64));
    assert!(store
        .decide_rewritten_agent_plan(&stale, &other, rewritten, 0, "2026-10-26T00:00:01Z")
        .is_err());
    assert_eq!(
        store.agent_plan("plan-s").unwrap().unwrap().status,
        "expired"
    );
}

/// The schema 35 run table (as schema 32 created it), with one sealed run:
/// the fixture a schema 36 build upgrades.
const RUNS_V35_SQL: &str = "DROP TABLE agent_runs;
CREATE TABLE agent_runs (
    run_id TEXT PRIMARY KEY NOT NULL CHECK(length(run_id) BETWEEN 1 AND 128),
    session_id TEXT NOT NULL CHECK(length(session_id) BETWEEN 1 AND 128),
    origin TEXT NOT NULL CHECK(origin IN ('in_app','mcp','manual')),
    client_label TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL CHECK(status IN ('running','completed','failed','cancelled','interrupted')),
    model_route TEXT,
    catalog_version INTEGER NOT NULL CHECK(catalog_version >= 1),
    catalog_sha256 TEXT NOT NULL CHECK(length(catalog_sha256) = 64),
    app_version TEXT NOT NULL,
    source_commit TEXT NOT NULL,
    store_revision_start INTEGER NOT NULL CHECK(store_revision_start >= 0),
    store_revision_end INTEGER,
    record_dir TEXT NOT NULL CHECK(length(record_dir) BETWEEN 1 AND 256),
    trace_sha256 TEXT,
    reproduces_run_id TEXT
) STRICT;
CREATE INDEX agent_runs_session_idx ON agent_runs(session_id, started_at);
INSERT INTO agent_runs(run_id,session_id,origin,started_at,ended_at,status,catalog_version,catalog_sha256,app_version,source_commit,store_revision_start,store_revision_end,record_dir)
    VALUES('run-old','session-a','in_app','2026-10-25T00:00:00Z','2026-10-25T00:01:00Z','completed',1,
        '0000000000000000000000000000000000000000000000000000000000000000','test','test',0,0,'agent/runs/run-old');
PRAGMA user_version = 35;";

/// W3h hand-back (schema 36): each run row says whether a provider refused
/// the reasoning-off switch in it and whether a reply was truncated, so a run
/// list answers without reading run records. Runs sealed before schema 36
/// read false; the upgrade is re-entrant.
#[test]
fn schema_36_adds_run_flags_that_default_to_false_and_the_upgrade_is_reentrant() {
    let (path, store) = temp_store("v35-runs");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(RUNS_V35_SQL).unwrap();
    drop(connection);
    let flags = |path: &std::path::Path| -> (i64, i64) {
        Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT reasoning_off_refused, truncated FROM agent_runs WHERE run_id='run-old'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };

    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert_eq!(STORE_SCHEMA_VERSION, 43);
    assert_eq!(
        store.agent_run("run-old").unwrap().unwrap().status,
        "completed",
        "the old row is kept"
    );
    drop(store);
    assert_eq!(flags(&path), (0, 0));
    let connection = Connection::open(&path).unwrap();
    for (column, value, ok) in [
        ("reasoning_off_refused", "1", true),
        ("truncated", "1", true),
        ("truncated", "2", false),
        ("reasoning_off_refused", "NULL", false),
    ] {
        assert_eq!(
            connection
                .execute_batch(&format!(
                    "UPDATE agent_runs SET {column}={value} WHERE run_id='run-old';"
                ))
                .is_ok(),
            ok,
            "{column}={value}"
        );
    }
    // A repeated or faked upgrade converges and keeps the flags.
    connection
        .execute_batch("PRAGMA user_version = 35;")
        .unwrap();
    drop(connection);
    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    drop(store);
    assert_eq!(flags(&path), (1, 1));
}

/// The schema 36 run table: schema 35's plus the two run flags.
fn runs_v36_sql() -> String {
    RUNS_V35_SQL.replace(
        "PRAGMA user_version = 35;",
        "ALTER TABLE agent_runs ADD COLUMN reasoning_off_refused INTEGER NOT NULL DEFAULT 0 CHECK(reasoning_off_refused IN (0,1));
ALTER TABLE agent_runs ADD COLUMN truncated INTEGER NOT NULL DEFAULT 0 CHECK(truncated IN (0,1));
PRAGMA user_version = 36;",
    )
}

/// W3m (schema 37): each run row keeps the token usage its channel reported
/// (prompt and completion totals), so a run list answers without reading run
/// records. Runs sealed before schema 37 read null; the upgrade is
/// re-entrant; a negative count is refused.
#[test]
fn schema_37_adds_run_usage_that_is_null_before_and_the_upgrade_is_reentrant() {
    let (path, store) = temp_store("v36-runs");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(&runs_v36_sql()).unwrap();
    drop(connection);
    let usage = |path: &std::path::Path| -> (Option<i64>, Option<i64>) {
        Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT usage_prompt, usage_completion FROM agent_runs WHERE run_id='run-old'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert_eq!(STORE_SCHEMA_VERSION, 43);
    drop(store);
    assert_eq!(usage(&path), (None, None));
    let connection = Connection::open(&path).unwrap();
    for (column, value, ok) in [
        ("usage_prompt", "120", true),
        ("usage_completion", "30", true),
        ("usage_completion", "-1", false),
        ("usage_prompt", "NULL", true),
        ("usage_prompt", "120", true),
    ] {
        assert_eq!(
            connection
                .execute_batch(&format!(
                    "UPDATE agent_runs SET {column}={value} WHERE run_id='run-old';"
                ))
                .is_ok(),
            ok,
            "{column}={value}"
        );
    }
    connection
        .execute_batch("PRAGMA user_version = 36;")
        .unwrap();
    drop(connection);
    let store = TaskStore::open_existing(&path).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    drop(store);
    assert_eq!(usage(&path), (Some(120), Some(30)));
}

/// W3m review: a new run row never starts with usage (it is summed from the
/// run record at seal), and a sealed or interrupted row keeps the usage it
/// is given, a count beyond SQLite's range stored as the largest it holds.
#[test]
fn run_usage_is_written_only_at_seal_and_an_untrusted_count_is_clamped() {
    let (_path, mut store) = temp_store("run-usage");
    let row = |run_id: &str, usage_prompt: Option<u64>| insar_store::AgentRunRow {
        run_id: run_id.into(),
        session_id: "session-a".into(),
        origin: "in_app".into(),
        client_label: None,
        started_at: "2026-10-27T00:00:00Z".into(),
        ended_at: None,
        status: "running".into(),
        model_route: None,
        catalog_version: 1,
        catalog_sha256: "0".repeat(64),
        app_version: "test".into(),
        source_commit: "test".into(),
        store_revision_start: 0,
        store_revision_end: None,
        record_dir: format!("agent/runs/{run_id}"),
        trace_sha256: None,
        reproduces_run_id: None,
        reasoning_off_refused: false,
        truncated: false,
        usage_prompt,
        usage_completion: None,
    };
    assert!(store.start_agent_run(&row("run-u0", Some(1))).is_err());
    store.start_agent_run(&row("run-u1", None)).unwrap();
    let flags = insar_store::AgentRunFlags {
        reasoning_off_refused: false,
        truncated: false,
        usage_prompt: Some(u64::MAX),
        usage_completion: Some(30),
    };
    store
        .finish_agent_run(
            "run-u1",
            "completed",
            "2026-10-27T00:01:00Z",
            0,
            &"a".repeat(64),
            flags,
        )
        .unwrap();
    let sealed = store.agent_run("run-u1").unwrap().unwrap();
    assert_eq!(sealed.usage_prompt, Some(i64::MAX as u64));
    assert_eq!(sealed.usage_completion, Some(30));
    store.start_agent_run(&row("run-u2", None)).unwrap();
    store.interrupt_agent_runs("2026-10-27T00:02:00Z").unwrap();
    store
        .set_interrupted_trace_hash("run-u2", &"b".repeat(64), flags)
        .unwrap();
    let interrupted = store.agent_run("run-u2").unwrap().unwrap();
    assert_eq!(interrupted.status, "interrupted");
    assert_eq!(interrupted.usage_prompt, Some(i64::MAX as u64));
}
