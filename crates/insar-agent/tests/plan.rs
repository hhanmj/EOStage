use insar_agent::plan::{approve, build_plan, plan_hash, require_approved, PlanError};
use insar_store::{AgentApprovalRow, AgentPlanRow, TaskStore};
use serde_json::json;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_PLAN_DB_ID: AtomicU64 = AtomicU64::new(0);

struct TempAgentStore {
    path: PathBuf,
    inner: Option<TaskStore>,
}

impl Deref for TempAgentStore {
    type Target = TaskStore;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().expect("TaskStore active")
    }
}

impl DerefMut for TempAgentStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().expect("TaskStore active")
    }
}

impl Drop for TempAgentStore {
    fn drop(&mut self) {
        let _ = self.inner.take();
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}

fn store() -> TempAgentStore {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = NEXT_PLAN_DB_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "agent-plan-test-{}-{nonce}-{seq}.db",
        std::process::id()
    ));
    let opened = TaskStore::open(&path).unwrap();
    TempAgentStore {
        path,
        inner: Some(opened),
    }
}
fn plan() -> AgentPlanRow {
    AgentPlanRow {
        plan_id: "plan-a".into(),
        session_id: "session-a".into(),
        run_id: "run-a".into(),
        created_at: "2026-10-23T09:00:00Z".into(),
        model_route: "manual".into(),
        plan_json: String::new(),
        plan_hash: String::new(),
        max_tier: 2,
        status: "draft".into(),
        expires_at: "2026-10-23T09:30:00Z".into(),
        store_revision_seen: 4,
    }
}
fn approval(hash: &str) -> AgentApprovalRow {
    AgentApprovalRow {
        approval_id: "approval-a".into(),
        plan_id: "plan-a".into(),
        plan_hash_seen: hash.into(),
        snapshot_json:
            json!({"source":"fixture","range":"fixture","estimated_bytes":42,"output":"relative"})
                .to_string(),
        decision: "approved".into(),
        decided_at: "2026-10-23T09:05:00Z".into(),
        decided_via: "desktop_card".into(),
        mode: None,
        rewritten_from_plan_hash: None,
    }
}

#[test]
fn model_generated_plan_cannot_include_an_unregistered_shell_tool() {
    let attempt = build_plan(
        plan(),
        &json!({"steps":[{"tool_id":"shell_exec","input":{"command":"not-executed"}}]}),
    );
    assert!(
        attempt.is_err(),
        "planner must reject unregistered commands before persistence"
    );
}

#[test]
fn canonical_hash_ignores_map_key_order_but_not_step_or_task_identity() {
    let left = json!({"steps":[{"tool_id":"task_page","task_id":"task-a","event_id":"evt-a"}],"a":{"x":1,"y":2}});
    let right: serde_json::Value = serde_json::from_str("{\"a\":{\"y\":2,\"x\":1},\"steps\":[{\"event_id\":\"evt-a\",\"task_id\":\"task-a\",\"tool_id\":\"task_page\"}]}").unwrap();
    assert_eq!(plan_hash(&left).unwrap(), plan_hash(&right).unwrap());
    assert_ne!(plan_hash(&left).unwrap(), plan_hash(&json!({"steps":[{"tool_id":"task_page","task_id":"task-b","event_id":"evt-a"}],"a":{"x":1,"y":2}})).unwrap());
}

#[test]
fn approval_freezes_snapshot_and_requires_matching_hash_revision_and_expiry() {
    let mut store = store();
    let row = build_plan(
        plan(),
        &json!({"steps":[{"tool_id":"update_download_task","task_id":"task-a"}], "card_snapshot":{"source":"fixture","range":"fixture","estimated_bytes":42,"output":"relative"}}),
    )
    .unwrap();
    store.save_agent_plan(&row).unwrap();
    let receipt = approval(&row.plan_hash);
    approve(
        &mut store,
        &row.plan_id,
        &receipt,
        4,
        "2026-10-23T09:05:00Z",
    )
    .unwrap();
    assert_eq!(store.agent_approval("plan-a").unwrap().unwrap(), receipt);
    assert_eq!(
        require_approved(&mut store, "plan-a", 4, "2026-10-23T09:10:00Z")
            .unwrap()
            .plan_hash,
        row.plan_hash
    );
    assert!(matches!(
        require_approved(&mut store, "plan-a", 5, "2026-10-23T09:10:00Z"),
        Err(PlanError::Expired)
    ));
    assert_eq!(
        store.agent_plan("plan-a").unwrap().unwrap().status,
        "expired"
    );
}

#[test]
fn card_approval_rejects_display_snapshot_that_differs_from_the_frozen_plan() {
    let mut store = store();
    let displayed = json!({"source":"asf.sentinel1","range":"fixture AOI","estimated_bytes":null,"output":"task/task-a"});
    let row = build_plan(plan(), &json!({"steps":[{"tool_id":"update_download_task","input":{"task_id":"task-a","event_id":"event-a"}}],"card_snapshot":displayed})).unwrap();
    store.save_agent_plan(&row).unwrap();
    let receipt = approval(&row.plan_hash);
    assert!(
        approve(
            &mut store,
            &row.plan_id,
            &receipt,
            4,
            "2026-10-23T09:05:00Z"
        )
        .is_err(),
        "approved snapshot must equal the planned display"
    );
    assert!(store.agent_approval(&row.plan_id).unwrap().is_none());
}

#[test]
fn execution_rechecks_approved_card_snapshot_not_only_plan_hash() {
    let mut store = store();
    let intent = json!({"steps":[{"tool_id":"update_download_task","input":{"task_id":"task-a","event_id":"event-a"}}],"card_snapshot":{"source":"asf.sentinel1","range":"AOI fixture","estimated_bytes":null,"output":"task/task-a"}});
    let row = build_plan(plan(), &intent).unwrap();
    store.save_agent_plan(&row).unwrap();
    // A direct Store receipt remains constrained on readback by its frozen card.
    let receipt = approval(&row.plan_hash);
    store
        .decide_agent_plan(&receipt, &row.plan_hash, 4, "2026-10-23T09:05:00Z")
        .unwrap();
    assert!(matches!(
        require_approved(&mut store, &row.plan_id, 4, "2026-10-23T09:06:00Z"),
        Err(PlanError::Expired)
    ));
    assert_eq!(
        store.agent_plan(&row.plan_id).unwrap().unwrap().status,
        "expired"
    );
}

#[test]
fn rejects_changed_hash_and_wrong_approval_source() {
    let mut store = store();
    let row = build_plan(
        plan(),
        &json!({"steps":[{"tool_id":"update_download_task","task_id":"task-a"}]}),
    )
    .unwrap();
    store.save_agent_plan(&row).unwrap();
    let mut receipt = approval("a".repeat(64).as_str());
    assert!(matches!(
        approve(&mut store, "plan-a", &receipt, 4, "2026-10-23T09:05:00Z"),
        Err(PlanError::Expired)
    ));
    assert_eq!(store.agent_approval("plan-a").unwrap(), None);
    let mut next = build_plan(
        plan(),
        &json!({"steps":[{"tool_id":"update_download_task"}]}),
    )
    .unwrap();
    next.plan_id = "plan-b".into();
    store.save_agent_plan(&next).unwrap();
    receipt.plan_id = "plan-b".into();
    receipt.plan_hash_seen = next.plan_hash;
    receipt.decided_via = "model".into();
    assert!(approve(&mut store, "plan-b", &receipt, 4, "2026-10-23T09:05:00Z").is_err());
}

/// Owner 2026-10-26: a directory chosen on the card goes into every
/// create_task step and the frozen card; a card about a task control cannot
/// be rewritten.
#[test]
fn rewrite_output_dir_points_every_create_task_and_the_card_at_the_chosen_directory() {
    let create = |output: serde_json::Value| {
        json!({"tool_id": "create_task", "input": {"batch_id": "b", "revision": 0,
            "item_ids": ["x"], "options": output, "event_id": "e"}})
    };
    let intent = json!({
        "steps": [create(json!({})), create(json!({"output_root": "D:/msg"}))],
        "card_snapshot": {"source": "s", "range": {}, "estimated_bytes": 1,
            "output_dir": null, "output_dir_source": null},
    });
    assert!(insar_agent::plan::missing_output_dir(&intent));
    let rewritten = insar_agent::plan::rewrite_output_dir(&intent, "E:/chosen").unwrap();
    assert!(!insar_agent::plan::missing_output_dir(&rewritten));
    for step in rewritten["steps"].as_array().unwrap() {
        assert_eq!(step["input"]["options"]["output_root"], "E:/chosen");
    }
    assert_eq!(rewritten["card_snapshot"]["output_dir"], "E:/chosen");
    assert_eq!(
        rewritten["card_snapshot"]["output_dir_source"],
        "chosen_on_card"
    );
    assert_ne!(plan_hash(&rewritten).unwrap(), plan_hash(&intent).unwrap());
    let control = json!({
        "steps": [{"tool_id": "task_action", "input": {"task_id": "t", "revision": 0,
            "action": "pause", "event_id": "e"}}, create(json!({}))],
        "card_snapshot": {"source": "s", "range": {}, "estimated_bytes": null, "output_dir": "D:/t"},
    });
    assert!(insar_agent::plan::rewrite_output_dir(&control, "E:/chosen").is_err());
}

/// The echoed card is checked against the shown plan before the rewrite;
/// the approval keeps the shown plan's hash and the rewritten card.
#[test]
fn approve_rewritten_checks_the_shown_card_and_records_both_hashes() {
    let mut store = store();
    let card = json!({"source": "s", "range": "r", "estimated_bytes": 1,
        "output_dir": "D:/default", "output_dir_source": "default"});
    let intent = json!({
        "steps": [{"tool_id": "create_task", "input": {"batch_id": "b", "revision": 0,
            "item_ids": ["x"], "options": {"output_root": "D:/default"}, "event_id": "e"}}],
        "card_snapshot": card,
    });
    let row = build_plan(plan(), &intent).unwrap();
    store.save_agent_plan(&row).unwrap();
    let rewritten = insar_agent::plan::rewrite_output_dir(&intent, "E:/chosen").unwrap();
    let mut receipt = approval(&row.plan_hash);
    receipt.snapshot_json = card.to_string();
    let new_hash = insar_agent::plan::approve_rewritten(
        &mut store,
        "plan-a",
        &receipt,
        &rewritten,
        4,
        "2026-10-23T09:05:00Z",
    )
    .unwrap();
    assert_eq!(new_hash, plan_hash(&rewritten).unwrap());
    let decided = store.agent_approval("plan-a").unwrap().unwrap();
    assert_eq!(decided.plan_hash_seen, new_hash);
    assert_eq!(
        decided.rewritten_from_plan_hash.as_deref(),
        Some(row.plan_hash.as_str())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&decided.snapshot_json).unwrap(),
        rewritten["card_snapshot"]
    );
    let approved = require_approved(&mut store, "plan-a", 4, "2026-10-23T09:06:00Z").unwrap();
    assert_eq!(approved.plan_hash, new_hash);

    // An echo that is not the shown card expires the plan; nothing is rewritten.
    let mut other = plan();
    other.plan_id = "plan-b".into();
    let row = build_plan(other, &intent).unwrap();
    store.save_agent_plan(&row).unwrap();
    let mut forged = approval(&row.plan_hash);
    forged.plan_id = "plan-b".into();
    forged.snapshot_json = rewritten["card_snapshot"].to_string();
    assert!(matches!(
        insar_agent::plan::approve_rewritten(
            &mut store,
            "plan-b",
            &forged,
            &rewritten,
            4,
            "2026-10-23T09:05:00Z"
        ),
        Err(PlanError::Expired)
    ));
    let kept = store.agent_plan("plan-b").unwrap().unwrap();
    assert_eq!(
        (kept.plan_hash, kept.status),
        (row.plan_hash, "expired".into())
    );
}
