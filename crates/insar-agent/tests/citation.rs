use insar_agent::{
    citation::{check_answer, Answer, Claim, Reference},
    run::RunWriter,
};
use insar_core::{FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest};
use insar_store::{AgentRunRow, TaskStore};
use serde_json::json;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NONCE: AtomicU64 = AtomicU64::new(0);

fn setup() -> (std::path::PathBuf, TaskStore, RunWriter) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = NONCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "agent-citation-{}-{nonce}-{seq}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut store = TaskStore::open(root.join("store.db")).unwrap();
    let row = AgentRunRow {
        run_id: "run-citations".into(),
        session_id: "session".into(),
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
        record_dir: "agent/runs/run-citations".into(),
        trace_sha256: None,
        reproduces_run_id: None,
        reasoning_off_refused: false,
        truncated: false,
        usage_prompt: None,
        usage_completion: None,
    };
    let writer = RunWriter::start(&root, &mut store, &row).unwrap();
    (root, store, writer)
}
fn claim(category: &str, kind: &str, id: &str, sequence: Option<u64>) -> Claim {
    Claim {
        statement: "Model says file completed".into(),
        category: category.into(),
        refs: vec![Reference {
            kind: kind.into(),
            id: id.into(),
            sequence,
        }],
    }
}
#[test]
fn fake_missing_or_wrong_type_citations_are_unverified_without_changing_model_text() {
    let (root, mut store, mut writer) = setup();
    let item = SnapshotItem {
        item_id: "ITEM-A".into(),
        provider_id: "asf.sentinel1".into(),
        display_name: "A.zip".into(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Default::default(),
        payload: json!({"file_name":"A.zip"}).as_object().unwrap().clone(),
        asset_access: None,
        execution_policy: None,
    };
    store.save_search_batch(&StoreSearchBatchRequest{activate_workbench:true,batch:FrozenSearchBatch{schema_version:1,batch_id:"batch-a".into(),provider_id:"asf.sentinel1".into(),created_at:"2026-10-23T09:00:00Z".into(),aoi_name:"fixture".into(),aoi_geojson:Some(json!({"type":"Polygon","coordinates":[[[100,30],[101,30],[101,31],[100,31],[100,30]]]}).as_object().unwrap().clone()),query:Default::default(),search_stats:None,candidates:vec![item]},workbench:SearchWorkbenchState{batch_id:"batch-a".into(),checked_item_ids:vec!["ITEM-A".into()],page_index:0,page_size:24,revision:0}}).unwrap();
    writer.append("tool_result",&json!({"id":"batch-a","summary":{"search_batch_id":"batch-a"},"observed_at":"2026-10-23T09:00:00Z","source":"fixture"})).unwrap();
    writer
        .append(
            "verification",
            &json!({"id":"file-a","sha256":"a".repeat(64),"verified":true}),
        )
        .unwrap();
    writer
        .finish(
            &mut store,
            "run-citations",
            "completed",
            "2026-10-23T09:01:00Z",
            0,
        )
        .unwrap();
    let text = "The model originally said completed";
    let answer = Answer {
        text: text.into(),
        claims: Some(vec![
            claim("verified", "verification", "invented", Some(42)),
            claim("verified", "tool_result", "batch-a", Some(0)),
            claim("verified", "verification", "file-a", Some(1)),
            claim("discovered", "tool_result", "batch-a", Some(0)),
            Claim {
                statement: "No evidence".into(),
                category: "completed".into(),
                refs: vec![],
            },
            claim("completed", "task_id", "unknown-task", None),
        ]),
    };
    let checked = check_answer(&root, &store, "run-citations", &answer).unwrap();
    assert_eq!(checked.text, text);
    assert_eq!(
        checked
            .claims
            .iter()
            .map(|claim| claim.verified)
            .collect::<Vec<_>>(),
        [false, false, true, true, false, false]
    );
    assert!(!checked.unreferenced);
    let plain = check_answer(
        &root,
        &store,
        "run-citations",
        &Answer {
            text: text.into(),
            claims: None,
        },
    )
    .unwrap();
    assert!(plain.unreferenced);
    assert!(plain.claims.is_empty());
}
#[test]
fn tampered_trace_never_verifies_a_claim() {
    let (root, mut store, mut writer) = setup();
    writer
        .append("verification", &json!({"id":"file-a","verified":true}))
        .unwrap();
    writer
        .finish(
            &mut store,
            "run-citations",
            "completed",
            "2026-10-23T09:01:00Z",
            0,
        )
        .unwrap();
    let path = root.join("agent/runs/run-citations/trace.jsonl");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("file-a", "file-b"),
    )
    .unwrap();
    assert!(check_answer(
        &root,
        &store,
        "run-citations",
        &Answer {
            text: "trust?".into(),
            claims: Some(vec![claim("verified", "verification", "file-b", Some(0))])
        }
    )
    .is_err());
}
