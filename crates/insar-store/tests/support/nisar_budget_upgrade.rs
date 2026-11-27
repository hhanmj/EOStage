use super::*;
use insar_core::{DownloadItemProgress, DownloadItemStatus};
use insar_store::{
    NisarBudgetUpgradeLimitsV2, StoreNisarBudgetUpgradeDownloadProgressV2,
    StoreNisarBudgetUpgradeEvidenceV2, StoreNisarBudgetUpgradePreReconciliationV2,
    StoredNisarBudgetUpgradeProofV2,
};

fn proof_for(store: &TaskStore, task_id: &str) -> StoredNisarBudgetUpgradeProofV2 {
    proof_for_item(store, task_id, ITEM_ID)
}

fn proof_for_item(
    store: &TaskStore,
    task_id: &str,
    item_id: &str,
) -> StoredNisarBudgetUpgradeProofV2 {
    let context = store
        .load_nisar_budget_upgrade_context_v2(task_id, item_id)
        .unwrap();
    let intent = parse_nisar_subset_prepared_intent_v2_envelope(&context.item.metadata)
        .unwrap()
        .unwrap();
    let plan = parse_nisar_subset_plan_v2_envelope(&context.item.metadata)
        .unwrap()
        .unwrap();
    let old_science = json!({
        "task_id": task_id, "item_id": item_id, "anchor_generation": 7,
        "checkpoint_sha256": "c".repeat(64), "pending_attempt": null, "state": "Interrupted",
        "lineage_key_sha256": "a".repeat(64), "execution_binding_sha256": "b".repeat(64),
        "prepared_intent_sha256": intent.canonical_intent_sha256,
        "final_plan_sha256": plan.canonical_semantic_plan_digest,
        "physical_plan_sha256": "d".repeat(64), "reserved_science_request_count": 935,
        "reserved_science_response_body_bytes": 500, "completed_ranges": [],
    });
    let old_planning = json!({
        "task_id": task_id, "item_id": item_id,
        "physical_plan": {"physical_plan_sha256": "d".repeat(64), "combined_budget": {
            "budget_policy": "physical_execution_union_v1", "max_request_count": 965,
            "remaining_science_request_count": 934, "max_response_body_bytes": 2000
        }}
    });
    StoredNisarBudgetUpgradeProofV2::new(
        context,
        "2026-10-16T12:00:00Z".to_owned(),
        StoreNisarBudgetUpgradeEvidenceV2 {
            planning_checkpoint_canonical_json: old_planning.to_string(),
            science_checkpoint_canonical_json: old_science.to_string(),
            old_limits: NisarBudgetUpgradeLimitsV2 {
                max_request_count: 965,
                max_science_request_count: 934,
                max_response_body_bytes: 2000,
            },
            new_limits: NisarBudgetUpgradeLimitsV2 {
                max_request_count: 1054,
                max_science_request_count: 1023,
                max_response_body_bytes: 2000,
            },
            completed_ranges_sha256: "e".repeat(64),
            remaining_ranges_sha256: "f".repeat(64),
            pre_reconciliation: None,
        },
    )
    .unwrap()
}

fn runtime_revision(store: &TaskStore, task_id: &str) -> u64 {
    store
        .load_task_record(task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision
}

fn upgrade_events(store: &TaskStore, task_id: &str) -> Vec<insar_store::StoredTaskEvent> {
    store
        .list_task_events(task_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "nisar_budget_upgrade")
        .collect()
}

fn next_anchor(
    proof: &StoredNisarBudgetUpgradeProofV2,
) -> StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
    next_anchor_with_target(proof, |_| {})
}

fn next_anchor_with_target(
    proof: &StoredNisarBudgetUpgradeProofV2,
    mutate: impl FnOnce(&mut Value),
) -> StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
    let context = proof.context();
    let old_anchor: Value = serde_json::from_str(&context.science_anchor.canonical_json).unwrap();
    let mut next: Value =
        serde_json::from_str(&proof.evidence().science_checkpoint_canonical_json).unwrap();
    next["anchor_generation"] = json!(8);
    next["previous_checkpoint_sha256"] = json!("c".repeat(64));
    next["checkpoint_sha256"] = json!("d".repeat(64));
    next["budget_upgrade_receipt"] = json!({
        "contract": "nisar_science_budget_upgrade_v1", "proof_sha256": proof.proof_sha256(),
        "limits": proof.evidence().new_limits,
    });
    mutate(&mut next);
    let canonical = next.to_string();
    #[derive(Serialize)]
    struct Pending {
        predecessor_checkpoint_sha256: Option<String>,
        predecessor_generation: Option<u64>,
        checkpoint_generation: u64,
        checkpoint_sha256: String,
        checkpoint_canonical_json: String,
        transition_sha256: String,
    }
    let transition_sha256 = test_sha256(&(
        Some("c".repeat(64)),
        Some(7_u64),
        8_u64,
        "d".repeat(64),
        &canonical,
    ));
    let pending = Pending {
        predecessor_checkpoint_sha256: Some("c".repeat(64)),
        predecessor_generation: Some(7),
        checkpoint_generation: 8,
        checkpoint_sha256: "d".repeat(64),
        checkpoint_canonical_json: canonical,
        transition_sha256,
    };
    let telemetry = TestRecoveryTelemetry {
        contract: TEST_RECOVERY_TELEMETRY_CONTRACT.to_owned(),
        checkpoint_sha256: "c".repeat(64),
        reusable_verified_bytes: 500,
        reused_verified_bytes: 100,
    };
    let digest = test_sha256(&(
        TEST_RECOVERY_TELEMETRY_DIGEST_CONTRACT,
        &old_anchor["lineage_key_sha256"],
        &old_anchor["execution_binding_sha256"],
        7_u64,
        Some("c".repeat(64)),
        &telemetry,
        Some(&pending),
    ));
    // Preserve typed field ordering; the production Store reuses the strict anchor reader.
    #[derive(Serialize)]
    struct Anchor<'a> {
        lineage_key_sha256: String,
        execution_binding_sha256: String,
        generation: u64,
        committed_checkpoint_sha256: Option<String>,
        committed_recovery_telemetry: TestRecoveryTelemetry,
        pending_transition: Option<&'a Pending>,
        record_sha256: String,
    }
    let text = serde_json::to_string(&Anchor {
        lineage_key_sha256: "a".repeat(64),
        execution_binding_sha256: "b".repeat(64),
        generation: 7,
        committed_checkpoint_sha256: Some("c".repeat(64)),
        committed_recovery_telemetry: telemetry,
        pending_transition: Some(&pending),
        record_sha256: digest.clone(),
    })
    .unwrap();
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
        task_id: context.task_id.clone(),
        item_id: context.item_id.clone(),
        anchor_kind: NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.to_owned(),
        lineage_key_sha256: "a".repeat(64),
        expected_record_sha256: Some(context.science_anchor.record_sha256.clone()),
        record_sha256: digest,
        canonical_json: text,
    }
}

#[test]
fn budget_upgrade_proof_and_pending_anchor_commit_once_and_survive_reopen() {
    let database = TempDatabase::new("budget-proof-atomic");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-atomic");
    save_extended_recovery_anchor(&mut store, "upgrade-atomic", ITEM_ID, 500, 100);
    let proof = proof_for(&store, "upgrade-atomic");
    let next = next_anchor(&proof);
    let revision_before = runtime_revision(&store, "upgrade-atomic");
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next)
        .unwrap());
    assert!(!store
        .prepare_nisar_budget_upgrade_v2(&proof, &next)
        .unwrap());
    // Exactly one visible task event per committed upgrade, sequenced by the
    // runtime revision it advances; a repeated prepare records nothing.
    let events = upgrade_events(&store, "upgrade-atomic");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].item_id.as_deref(), Some(ITEM_ID));
    assert_eq!(
        events[0].payload["proof_sha256"],
        json!(proof.proof_sha256())
    );
    assert_eq!(
        events[0].payload["new_limits"],
        json!(proof.evidence().new_limits)
    );
    assert_eq!(events[0].sequence, revision_before + 1);
    assert_eq!(
        runtime_revision(&store, "upgrade-atomic"),
        revision_before + 1
    );
    drop(store);
    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store
            .load_nisar_budget_upgrade_proof_v2("upgrade-atomic", ITEM_ID)
            .unwrap(),
        Some(proof)
    );
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                "upgrade-atomic",
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
            )
            .unwrap()
            .unwrap()
            .canonical_json,
        next.canonical_json
    );
}

#[test]
fn budget_upgrade_second_sql_failure_rolls_back_proof_and_anchor() {
    let database = TempDatabase::new("budget-proof-rollback");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-rollback");
    save_extended_recovery_anchor(&mut store, "upgrade-rollback", ITEM_ID, 500, 100);
    let proof = proof_for(&store, "upgrade-rollback");
    let revision_before = runtime_revision(&store, "upgrade-rollback");
    let connection = Connection::open(database.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_upgrade_anchor BEFORE UPDATE ON nisar_subset_execution_anchors_v2 BEGIN SELECT RAISE(ABORT,'test anchor write failure'); END;").unwrap();
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .is_err());
    assert!(upgrade_events(&store, "upgrade-rollback").is_empty());
    assert_eq!(
        runtime_revision(&store, "upgrade-rollback"),
        revision_before
    );
    assert!(store
        .load_nisar_budget_upgrade_proof_v2("upgrade-rollback", ITEM_ID)
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                "upgrade-rollback",
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
            )
            .unwrap(),
        Some(proof.context().science_anchor.clone())
    );
}

#[test]
fn budget_upgrade_zero_row_anchor_update_also_rolls_back_proof() {
    let database = TempDatabase::new("budget-proof-zero-row");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-zero");
    save_extended_recovery_anchor(&mut store, "upgrade-zero", ITEM_ID, 500, 100);
    let proof = proof_for(&store, "upgrade-zero");
    let connection = Connection::open(database.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER ignore_upgrade_anchor BEFORE UPDATE ON nisar_subset_execution_anchors_v2 BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(matches!(
        store.prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof)),
        Err(StoreError::RevisionConflict)
    ));
    assert!(store
        .load_nisar_budget_upgrade_proof_v2("upgrade-zero", ITEM_ID)
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                "upgrade-zero",
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
            )
            .unwrap(),
        Some(proof.context().science_anchor.clone())
    );
}

#[test]
fn budget_upgrade_rejects_resealed_target_changes_without_mutation() {
    let database = TempDatabase::new("budget-target-tamper");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-target");
    save_extended_recovery_anchor(&mut store, "upgrade-target", ITEM_ID, 500, 100);
    let proof = proof_for(&store, "upgrade-target");
    for key in [
        "completed",
        "charges",
        "receipt-contract",
        "receipt-sha",
        "receipt-limits",
        "generation",
        "predecessor",
        "state",
        "source",
    ] {
        let next = next_anchor_with_target(&proof, |target| match key {
            "completed" => target["completed_ranges"] = json!([{"index": 0}]),
            "charges" => target["reserved_science_request_count"] = json!(0),
            "receipt-contract" => target["budget_upgrade_receipt"]["contract"] = json!("wrong"),
            "receipt-sha" => {
                target["budget_upgrade_receipt"]["proof_sha256"] = json!("a".repeat(64))
            }
            "receipt-limits" => {
                target["budget_upgrade_receipt"]["limits"]["max_request_count"] = json!(u64::MAX)
            }
            "generation" => target["anchor_generation"] = json!(9),
            "predecessor" => target["previous_checkpoint_sha256"] = json!("a".repeat(64)),
            "state" => target["state"] = json!("Complete"),
            "source" => target["physical_plan_sha256"] = json!("a".repeat(64)),
            _ => unreachable!(),
        });
        assert!(
            store
                .prepare_nisar_budget_upgrade_v2(&proof, &next)
                .is_err(),
            "{key}"
        );
        assert!(store
            .load_nisar_budget_upgrade_proof_v2("upgrade-target", ITEM_ID)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .load_nisar_subset_execution_anchor_v2(
                    "upgrade-target",
                    ITEM_ID,
                    NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
                )
                .unwrap(),
            Some(proof.context().science_anchor.clone())
        );
    }
}

#[test]
fn budget_upgrade_two_real_items_cannot_exchange_proof_or_target() {
    let database = TempDatabase::new("budget-two-items");
    let mut store = TaskStore::open(database.path()).unwrap();
    let items = vec![source_item(), source_item_two()];
    let ids = vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()];
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "nisar-source-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-09-04T00:00:00Z".to_owned(),
                aoi_name: "Batch AOI".to_owned(),
                aoi_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
                query: Map::new(),
                search_stats: None,
                candidates: items.clone(),
            },
            workbench: SearchWorkbenchState {
                batch_id: "nisar-source-batch".to_owned(),
                checked_item_ids: ids.clone(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    let intents = [prepared_intent(), prepared_intent_two()];
    let task_items: Vec<_> = intents
        .iter()
        .zip(items)
        .map(|(intent, item)| task_item_for(intent, item))
        .collect();
    let mut request = task_request(database.path(), "upgrade-two");
    request.selected_item_ids = ids.clone();
    request.checked_item_ids = ids.clone();
    store
        .create_paused_task_with_explicit_items(&request, 0, &ids, &task_items)
        .unwrap();
    for (revision, kind, selected) in [
        (0, TaskStateCommandKind::ResumeTask, vec![]),
        (1, TaskStateCommandKind::ResumeItems, ids.clone()),
        (2, TaskStateCommandKind::ActivateItems, ids.clone()),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: request.task_id.clone(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: selected,
                },
                event_id: format!("upgrade-two-{revision}"),
                occurred_at: "2026-09-04T00:00:10Z".to_owned(),
            })
            .unwrap();
    }
    for (index, intent) in intents.into_iter().enumerate() {
        store
            .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
                task_id: request.task_id.clone(),
                item_id: ids[index].clone(),
                prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
                expected_revision: 3 + index as u64,
                occurred_at: "2026-09-04T00:00:40Z".to_owned(),
                plan: plan_with_graph(intent, graph()),
            })
            .unwrap();
        save_extended_recovery_anchor(&mut store, &request.task_id, &ids[index], 500, 100);
    }
    let first = proof_for_item(&store, &request.task_id, ITEM_ID);
    let second = proof_for_item(&store, &request.task_id, ITEM_ID_2);
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&first, &next_anchor(&second))
        .is_err());
    let mut substituted = next_anchor(&second);
    substituted.item_id = ITEM_ID.to_owned();
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&first, &substituted)
        .is_err());
    for item in &ids {
        assert!(store
            .load_nisar_budget_upgrade_proof_v2(&request.task_id, item)
            .unwrap()
            .is_none());
    }
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&first, &next_anchor(&first))
        .unwrap());
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&second, &next_anchor(&second))
        .unwrap());
    assert_eq!(
        store
            .load_nisar_budget_upgrade_proof_v2(&request.task_id, ITEM_ID_2)
            .unwrap(),
        Some(second)
    );
}

#[test]
fn budget_upgrade_refuses_changed_admission_without_writing_proof() {
    // Review P1: a task-level download revision bump alone (a sibling item's
    // progress or result write) is no longer a refusal; see the admission test.
    for change in [
        "runtime",
        "result",
        "progress",
        "writer",
        "anchor",
        "other-item",
    ] {
        let database = TempDatabase::new(change);
        let mut store = TaskStore::open(database.path()).unwrap();
        create_active_planned_task(&mut store, database.path(), "upgrade-stale");
        save_extended_recovery_anchor(&mut store, "upgrade-stale", ITEM_ID, 500, 100);
        let proof = proof_for(&store, "upgrade-stale");
        let mut next = next_anchor(&proof);
        let connection = Connection::open(database.path()).unwrap();
        match change {
            "runtime" => {
                let revision = proof.context().runtime_revision;
                store
                    .update_task(&StoreUpdateTaskRequest {
                        task_id: "upgrade-stale".to_owned(),
                        expected_revision: revision,
                        command: StoreTaskCommand {
                            kind: TaskStateCommandKind::PauseItems,
                            item_ids: vec![ITEM_ID.to_owned()],
                        },
                        event_id: "upgrade-pause".to_owned(),
                        occurred_at: "2026-10-16T12:00:01Z".to_owned(),
                    })
                    .unwrap();
            }
            "result" => {
                connection.execute("INSERT INTO download_results(task_id,item_id,outcome,finished_at,result_json,bytes_written) VALUES ('upgrade-stale',?1,'interrupted','changed','{}',0)", [ITEM_ID]).unwrap();
            }
            "progress" => {
                connection.execute("INSERT INTO download_progress(task_id,item_id,status,updated_at,progress_json) VALUES ('upgrade-stale',?1,'running','changed','{}')", [ITEM_ID]).unwrap();
            }
            "writer" => {
                connection.execute("INSERT INTO nisar_subset_execution_anchors_v2 SELECT task_id,item_id,?1,lineage_key_sha256,record_sha256,canonical_json FROM nisar_subset_execution_anchors_v2", [NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND]).unwrap();
            }
            "anchor" => {
                connection
                    .execute(
                        "UPDATE nisar_subset_execution_anchors_v2 SET record_sha256=?1",
                        ["f".repeat(64)],
                    )
                    .unwrap();
            }
            "other-item" => next.item_id = ITEM_ID_2.to_owned(),
            _ => unreachable!(),
        }
        assert!(
            !store
                .prepare_nisar_budget_upgrade_v2(&proof, &next)
                .unwrap_or(false),
            "{change}"
        );
        assert!(
            store
                .load_nisar_budget_upgrade_proof_v2("upgrade-stale", ITEM_ID)
                .unwrap()
                .is_none(),
            "{change}"
        );
    }
}

#[test]
fn budget_upgrade_preserves_exact_failure_row_after_later_result_replaces_it() {
    let database = TempDatabase::new("budget-raw-result");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-raw");
    save_extended_recovery_anchor(&mut store, "upgrade-raw", ITEM_ID, 500, 100);
    let mut result = DownloadItemResult {
        task_id: "upgrade-raw".to_owned(),
        item_id: ITEM_ID.to_owned(),
        outcome: DownloadOutcome::Failed,
        destination: database.path().join("output"),
        bytes_written: 500,
        attempt_count: 1,
        failure: Some(insar_core::DownloadFailure {
            code: "NISAR_SUBSET_V2_RECOVERY_INVALID".to_owned(),
            reason_zh: "NISAR V2 science recovery evidence is invalid".to_owned(),
            retryable: false,
        }),
        finished_at: "2026-10-16T11:54:35Z".to_owned(),
    };
    store.save_scheduler_download_result(&result).unwrap();
    let raw = serde_json::to_string_pretty(&result).unwrap();
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("UPDATE download_results SET result_json=?1,last_known_total_bytes=15699279872 WHERE task_id='upgrade-raw'", [&raw]).unwrap();
    let proof = proof_for(&store, "upgrade-raw");
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .unwrap());
    result.outcome = DownloadOutcome::Interrupted;
    result.failure = None;
    result.finished_at = "2026-10-16T12:01:00Z".to_owned();
    store.save_scheduler_download_result(&result).unwrap();
    drop(store);
    let store = TaskStore::open_existing(database.path()).unwrap();
    let preserved = store
        .load_nisar_budget_upgrade_proof_v2("upgrade-raw", ITEM_ID)
        .unwrap()
        .unwrap();
    assert_eq!(preserved, proof);
    assert_eq!(
        preserved.context().previous_result,
        insar_store::StoreNisarBudgetUpgradeDownloadResultV2::Present {
            outcome: "failed".to_owned(),
            finished_at: "2026-10-16T11:54:35Z".to_owned(),
            result_json: raw,
            last_known_total_bytes: Some(15_699_279_872),
            bytes_written: 500
        }
    );
}

fn rejection_result(task_id: &str, database: &Path, code: &str) -> DownloadItemResult {
    DownloadItemResult {
        task_id: task_id.to_owned(),
        item_id: ITEM_ID.to_owned(),
        outcome: DownloadOutcome::Failed,
        destination: database.join("output"),
        bytes_written: 0,
        attempt_count: 0,
        failure: Some(insar_core::DownloadFailure {
            code: code.to_owned(),
            reason_zh: "本地校验未通过：已下载片段的摘要与断点记录不一致。".to_owned(),
            retryable: false,
        }),
        finished_at: "2026-10-16T12:00:00Z".to_owned(),
    }
}

fn download_revision(connection: &Connection, task_id: &str) -> u64 {
    connection
        .query_row(
            "SELECT revision FROM download_engine_state WHERE task_id=?1",
            [task_id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap() as u64
}

#[test]
fn budget_upgrade_rejection_retains_replaced_result_row_and_shows_local_failure_atomically() {
    let database = TempDatabase::new("budget-rejection");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-rejection");
    save_extended_recovery_anchor(&mut store, "upgrade-rejection", ITEM_ID, 500, 100);
    let old = DownloadItemResult {
        task_id: "upgrade-rejection".to_owned(),
        item_id: ITEM_ID.to_owned(),
        outcome: DownloadOutcome::Failed,
        destination: database.path().join("output"),
        bytes_written: 500,
        attempt_count: 1,
        failure: Some(insar_core::DownloadFailure {
            code: "old-provider-error".to_owned(),
            reason_zh: "Original provider response".to_owned(),
            retryable: false,
        }),
        finished_at: "2026-10-16T11:54:35Z".to_owned(),
    };
    store.save_scheduler_download_result(&old).unwrap();
    let connection = Connection::open(database.path()).unwrap();
    // Byte-exact retention: a non-canonical JSON text and a stored total survive.
    connection.execute("UPDATE download_results SET result_json=?1,last_known_total_bytes=15699279872 WHERE task_id='upgrade-rejection'", [serde_json::to_string_pretty(&old).unwrap()]).unwrap();
    let before = store
        .load_nisar_budget_upgrade_context_v2("upgrade-rejection", ITEM_ID)
        .unwrap();
    assert_eq!(
        before.previous_progress,
        StoreNisarBudgetUpgradeDownloadProgressV2::Absent
    );
    let request = StoreUpdateTaskRequest {
        task_id: "upgrade-rejection".to_owned(),
        expected_revision: before.runtime_revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::MarkFailed,
            item_ids: vec![ITEM_ID.to_owned()],
        },
        event_id: "upgrade-rejected".to_owned(),
        occurred_at: "2026-10-16T12:00:00Z".to_owned(),
    };
    let rejection = rejection_result(
        "upgrade-rejection",
        database.path(),
        "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
    );
    let runtime = store
        .finish_nisar_budget_upgrade_rejection(&request, &rejection)
        .unwrap();
    assert!(runtime.failed_item_ids.contains(&ITEM_ID.to_owned()));
    assert!(!runtime.active_item_ids.contains(&ITEM_ID.to_owned()));
    assert_eq!(runtime.revision, before.runtime_revision + 1);
    assert_eq!(
        store
            .finish_nisar_budget_upgrade_rejection(&request, &rejection)
            .unwrap(),
        runtime,
        "an exact replay is idempotent"
    );
    let mut conflicting = rejection.clone();
    conflicting
        .failure
        .as_mut()
        .unwrap()
        .reason_zh
        .push_str(" altered");
    assert!(matches!(
        store.finish_nisar_budget_upgrade_rejection(&request, &conflicting),
        Err(StoreError::AlreadyExists)
    ));
    let events = store.list_task_events("upgrade-rejection").unwrap();
    let event = events.last().unwrap();
    assert_eq!(event.event_type, "mark_failed");
    assert_eq!(
        event.error,
        Some(serde_json::to_value(rejection.failure.as_ref().unwrap()).unwrap())
    );
    // The visible item failure is the local diagnostic, not the old provider text.
    let snapshot = store.load_download_snapshot("upgrade-rejection").unwrap();
    assert_eq!(snapshot.results, vec![rejection.clone()]);
    assert!(snapshot.progress.is_empty());
    assert_eq!(
        download_revision(&connection, "upgrade-rejection"),
        before.download_revision + 1
    );
    let refusals = store
        .list_nisar_budget_upgrade_refusals_v2("upgrade-rejection", ITEM_ID)
        .unwrap();
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].event_id(), "upgrade-rejected");
    assert_eq!(Some(refusals[0].failure()), rejection.failure.as_ref());
    assert_eq!(refusals[0].previous_result(), &before.previous_result);
    assert!(matches!(
        refusals[0].previous_result(),
        insar_store::StoreNisarBudgetUpgradeDownloadResultV2::Present {
            last_known_total_bytes: Some(15_699_279_872),
            bytes_written: 500,
            ..
        }
    ));
    assert_eq!(
        refusals[0].previous_progress(),
        &StoreNisarBudgetUpgradeDownloadProgressV2::Absent
    );
    assert!(connection
        .execute(
            "UPDATE nisar_budget_upgrade_refusal_v2 SET canonical_json='{}'",
            []
        )
        .is_err());
    assert!(connection
        .execute("DELETE FROM nisar_budget_upgrade_refusal_v2", [])
        .is_err());
    assert!(store
        .load_nisar_budget_upgrade_proof_v2("upgrade-rejection", ITEM_ID)
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                "upgrade-rejection",
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
            )
            .unwrap(),
        Some(before.science_anchor)
    );
    drop(store);
    let reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        reopened
            .list_nisar_budget_upgrade_refusals_v2("upgrade-rejection", ITEM_ID)
            .unwrap(),
        refusals
    );
}

#[test]
fn budget_upgrade_rejection_retains_a_stale_progress_row_before_showing_failure() {
    let database = TempDatabase::new("budget-rejection-progress");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-progress");
    save_extended_recovery_anchor(&mut store, "upgrade-progress", ITEM_ID, 500, 100);
    store
        .save_scheduler_download_progress(&DownloadItemProgress {
            task_id: "upgrade-progress".to_owned(),
            item_id: ITEM_ID.to_owned(),
            status: DownloadItemStatus::Active,
            bytes_completed: 7,
            total_bytes: Some(100),
            bytes_per_second: 1.5,
            attempt: 2,
            updated_at: "2026-10-16T11:59:00Z".to_owned(),
        })
        .unwrap();
    let before = store
        .load_nisar_budget_upgrade_context_v2("upgrade-progress", ITEM_ID)
        .unwrap();
    assert_eq!(
        before.previous_result,
        insar_store::StoreNisarBudgetUpgradeDownloadResultV2::Absent
    );
    assert!(matches!(
        &before.previous_progress,
        StoreNisarBudgetUpgradeDownloadProgressV2::Present {
            bytes_completed: 7,
            total_bytes: Some(100),
            ..
        }
    ));
    let rejection = rejection_result(
        "upgrade-progress",
        database.path(),
        "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
    );
    store
        .finish_nisar_budget_upgrade_rejection(
            &StoreUpdateTaskRequest {
                task_id: "upgrade-progress".to_owned(),
                expected_revision: before.runtime_revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![ITEM_ID.to_owned()],
                },
                event_id: "upgrade-progress-rejected".to_owned(),
                occurred_at: "2026-10-16T12:00:00Z".to_owned(),
            },
            &rejection,
        )
        .unwrap();
    let refusals = store
        .list_nisar_budget_upgrade_refusals_v2("upgrade-progress", ITEM_ID)
        .unwrap();
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].previous_progress(), &before.previous_progress);
    assert_eq!(refusals[0].previous_result(), &before.previous_result);
    let snapshot = store.load_download_snapshot("upgrade-progress").unwrap();
    assert_eq!(snapshot.results, vec![rejection]);
    assert!(
        snapshot.progress.is_empty(),
        "the visible result keeps the ordinary result semantics"
    );
}

#[test]
fn budget_upgrade_rejection_never_overrides_a_pause_or_cancel() {
    for kind in [
        TaskStateCommandKind::PauseTask,
        TaskStateCommandKind::PauseItems,
        TaskStateCommandKind::CancelItems,
    ] {
        let database = TempDatabase::new("budget-rejection-stop");
        let mut store = TaskStore::open(database.path()).unwrap();
        create_active_planned_task(&mut store, database.path(), "upgrade-stopped");
        save_extended_recovery_anchor(&mut store, "upgrade-stopped", ITEM_ID, 500, 100);
        let before = store
            .load_nisar_budget_upgrade_context_v2("upgrade-stopped", ITEM_ID)
            .unwrap();
        let download_before = store.load_download_snapshot("upgrade-stopped").unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "upgrade-stopped".to_owned(),
                expected_revision: before.runtime_revision,
                command: StoreTaskCommand {
                    kind: kind.clone(),
                    item_ids: if kind == TaskStateCommandKind::PauseTask {
                        vec![]
                    } else {
                        vec![ITEM_ID.to_owned()]
                    },
                },
                event_id: "stop-first".to_owned(),
                occurred_at: "2026-10-16T12:00:00Z".to_owned(),
            })
            .unwrap();
        let request = StoreUpdateTaskRequest {
            task_id: "upgrade-stopped".to_owned(),
            expected_revision: before.runtime_revision + 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec![ITEM_ID.to_owned()],
            },
            event_id: "rejection-after-stop".to_owned(),
            occurred_at: "2026-10-16T12:00:01Z".to_owned(),
        };
        let rejection = rejection_result("upgrade-stopped", database.path(), "LOCAL_VALIDATION");
        let runtime = store
            .finish_nisar_budget_upgrade_rejection(&request, &rejection)
            .unwrap();
        assert!(!runtime.active_item_ids.contains(&ITEM_ID.to_owned()));
        assert!(!runtime.failed_item_ids.contains(&ITEM_ID.to_owned()));
        if kind == TaskStateCommandKind::CancelItems {
            assert!(runtime.cancelled_item_ids.contains(&ITEM_ID.to_owned()));
        } else {
            assert!(runtime.paused_item_ids.contains(&ITEM_ID.to_owned()));
        }
        // An exact replay of the stop-won decision remains stable.
        assert_eq!(
            store
                .finish_nisar_budget_upgrade_rejection(&request, &rejection)
                .unwrap(),
            runtime
        );
        assert!(store
            .list_nisar_budget_upgrade_refusals_v2("upgrade-stopped", ITEM_ID)
            .unwrap()
            .is_empty());
        let download_after = store.load_download_snapshot("upgrade-stopped").unwrap();
        assert_eq!(download_after.results, download_before.results);
        assert_eq!(download_after.progress, download_before.progress);
        assert!(store
            .load_nisar_budget_upgrade_proof_v2("upgrade-stopped", ITEM_ID)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .load_nisar_subset_execution_anchor_v2(
                    "upgrade-stopped",
                    ITEM_ID,
                    NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
                )
                .unwrap(),
            Some(before.science_anchor)
        );
    }
}

#[test]
fn budget_upgrade_proof_retains_exact_stale_progress_row_without_touching_it() {
    let database = TempDatabase::new("budget-proof-progress");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-stale-progress");
    save_extended_recovery_anchor(&mut store, "upgrade-stale-progress", ITEM_ID, 500, 100);
    store
        .save_scheduler_download_progress(&DownloadItemProgress {
            task_id: "upgrade-stale-progress".to_owned(),
            item_id: ITEM_ID.to_owned(),
            status: DownloadItemStatus::Active,
            bytes_completed: 400,
            total_bytes: None,
            bytes_per_second: 12.25,
            attempt: 1,
            updated_at: "2026-10-16T11:58:00Z".to_owned(),
        })
        .unwrap();
    let connection = Connection::open(database.path()).unwrap();
    let raw_progress =
        |connection: &Connection| -> (String, String, String, i64, Option<i64>, f64, Option<i64>) {
            connection.query_row("SELECT status,updated_at,progress_json,bytes_completed,total_bytes,bytes_per_second,last_known_total_bytes FROM download_progress WHERE task_id='upgrade-stale-progress'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap()
        };
    let progress_before = raw_progress(&connection);
    let proof = proof_for(&store, "upgrade-stale-progress");
    let StoreNisarBudgetUpgradeDownloadProgressV2::Present {
        status,
        updated_at,
        progress_json,
        bytes_completed,
        total_bytes,
        bytes_per_second,
        last_known_total_bytes,
    } = proof.context().previous_progress.clone()
    else {
        panic!("a stale progress row must be retained, not treated as a conflict");
    };
    assert_eq!(
        (
            status,
            updated_at,
            progress_json,
            bytes_completed as i64,
            total_bytes.map(|n| n as i64),
            bytes_per_second,
            last_known_total_bytes.map(|n| n as i64)
        ),
        progress_before
    );
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .unwrap());
    assert_eq!(raw_progress(&connection), progress_before);
    drop(store);
    let reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        reopened
            .load_nisar_budget_upgrade_proof_v2("upgrade-stale-progress", ITEM_ID)
            .unwrap(),
        Some(proof)
    );
}

#[test]
fn budget_upgrade_refusal_schema_is_strict_immutable_and_migrates_from_v29() {
    let database = TempDatabase::new("budget-refusal-schema");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-refusal-schema");
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch("DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers; DROP TABLE nisar_budget_upgrade_refusal_v2; PRAGMA user_version=29;")
        .unwrap();
    drop(connection);
    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store.health().unwrap().schema_version,
        insar_store::STORE_SCHEMA_VERSION
    );
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let table_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='nisar_budget_upgrade_refusal_v2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(table_sql.ends_with("STRICT"));
    let insert = "INSERT INTO nisar_budget_upgrade_refusal_v2(event_id,task_id,item_id,refusal_sha256,canonical_json) VALUES (?1,?2,?3,?4,?5)";
    assert!(connection
        .execute(
            insert,
            params![
                "e1",
                "upgrade-refusal-schema",
                ITEM_ID,
                "a".repeat(64),
                "not json"
            ]
        )
        .is_err());
    assert!(connection
        .execute(
            insert,
            params!["e1", "missing-task", ITEM_ID, "a".repeat(64), "{}"]
        )
        .is_err());
    assert_eq!(
        connection
            .execute(
                insert,
                params![
                    "e1",
                    "upgrade-refusal-schema",
                    ITEM_ID,
                    "a".repeat(64),
                    "{}"
                ]
            )
            .unwrap(),
        1
    );
    assert!(connection
        .execute(
            "UPDATE nisar_budget_upgrade_refusal_v2 SET canonical_json='[]'",
            []
        )
        .is_err());
    assert!(connection
        .execute("DELETE FROM nisar_budget_upgrade_refusal_v2", [])
        .is_err());
    let store = TaskStore::open_existing(database.path()).unwrap();
    assert!(
        store
            .list_nisar_budget_upgrade_refusals_v2("upgrade-refusal-schema", ITEM_ID)
            .is_err(),
        "a row that is not canonical evidence is reported, never silently skipped"
    );
}

#[test]
fn budget_upgrade_rejects_bounded_evidence_and_row_tampering() {
    let database = TempDatabase::new("budget-proof-bounds");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-bounds");
    save_extended_recovery_anchor(&mut store, "upgrade-bounds", ITEM_ID, 500, 100);
    let proof = proof_for(&store, "upgrade-bounds");
    for key in [
        "planning", "science", "anchor", "policy", "identity", "ceiling",
    ] {
        let mut evidence = proof.evidence().clone();
        let mut context = proof.context().clone();
        match key {
            "planning" => {
                evidence.planning_checkpoint_canonical_json = "x".repeat(16 * 1024 * 1024 + 1)
            }
            "science" => {
                evidence.science_checkpoint_canonical_json = "x".repeat(16 * 1024 * 1024 + 1)
            }
            "anchor" => context.science_anchor.canonical_json = "x".repeat(64 * 1024 + 1),
            "policy" => {
                evidence.planning_checkpoint_canonical_json = evidence
                    .planning_checkpoint_canonical_json
                    .replace("physical_execution_union_v1", "physical_execution_union_v2")
            }
            "identity" => context.item_id = ITEM_ID_2.to_owned(),
            "ceiling" => evidence.new_limits.max_request_count = 1,
            _ => unreachable!(),
        }
        assert!(
            StoredNisarBudgetUpgradeProofV2::new(
                context,
                "2026-10-16T12:00:00Z".to_owned(),
                evidence
            )
            .is_err(),
            "{key}"
        );
    }
    let connection = Connection::open(database.path()).unwrap();
    let mut document: Value = serde_json::from_str(&proof.canonical_json().unwrap()).unwrap();
    document["occurred_at"] = json!("2026-10-16T12:01:00Z");
    connection.execute("INSERT INTO nisar_budget_upgrade_v2(task_id,item_id,proof_sha256,canonical_json) VALUES ('upgrade-bounds',?1,?2,?3)",
        params![ITEM_ID,proof.proof_sha256(),document.to_string()]).unwrap();
    assert!(store
        .load_nisar_budget_upgrade_proof_v2("upgrade-bounds", ITEM_ID)
        .is_err());
}

#[test]
fn budget_upgrade_exact_result_match_is_not_semantic_json_equality() {
    let database = TempDatabase::new("budget-result-exact");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-exact");
    save_extended_recovery_anchor(&mut store, "upgrade-exact", ITEM_ID, 500, 100);
    let result = DownloadItemResult {
        task_id: "upgrade-exact".to_owned(),
        item_id: ITEM_ID.to_owned(),
        outcome: DownloadOutcome::Interrupted,
        destination: database.path().join("output"),
        bytes_written: 500,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-10-16T11:54:35Z".to_owned(),
    };
    store.save_scheduler_download_result(&result).unwrap();
    let proof = proof_for(&store, "upgrade-exact");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "UPDATE download_results SET result_json=?1 WHERE task_id='upgrade-exact'",
            [serde_json::to_string_pretty(&result).unwrap()],
        )
        .unwrap();
    assert!(!store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .unwrap());
    assert!(store
        .load_nisar_budget_upgrade_proof_v2("upgrade-exact", ITEM_ID)
        .unwrap()
        .is_none());
}

#[test]
fn budget_upgrade_schema_retains_immutable_json_proof_per_item() {
    let database = TempDatabase::new("budget-upgrade-schema");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-schema");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let table_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='nisar_budget_upgrade_v2'",
            [],
            |row| row.get(0),
        )
        .expect("the budget proof table must be created with the Store");
    assert!(table_sql.ends_with("STRICT"));
    let insert = "INSERT INTO nisar_budget_upgrade_v2(task_id,item_id,proof_sha256,canonical_json) VALUES (?1,?2,?3,?4)";
    assert!(connection
        .execute(
            insert,
            params!["upgrade-schema", ITEM_ID, "a".repeat(64), "not json"]
        )
        .is_err());
    assert_eq!(
        connection
            .execute(
                insert,
                params!["upgrade-schema", ITEM_ID, "a".repeat(64), "{}"]
            )
            .unwrap(),
        1
    );
    assert!(connection
        .execute(
            insert,
            params!["upgrade-schema", ITEM_ID, "b".repeat(64), "{}"]
        )
        .is_err());
    assert!(connection
        .execute("UPDATE nisar_budget_upgrade_v2 SET canonical_json='[]'", [])
        .is_err());
    assert!(connection
        .execute("DELETE FROM nisar_budget_upgrade_v2", [])
        .is_err());
    assert!(connection
        .execute(
            insert,
            params!["missing-task", ITEM_ID, "a".repeat(64), "{}"]
        )
        .is_err());
    // SQL limits bytes, not Unicode scalar values. 24 Mi characters exceed 68 MiB.
    let oversized = serde_json::to_string(&"测".repeat(24 * 1024 * 1024)).unwrap();
    assert!(connection
        .execute(
            insert,
            params!["upgrade-schema", ITEM_ID_2, "a".repeat(64), oversized]
        )
        .is_err());
}

#[test]
fn budget_upgrade_schema_migrates_v28_without_losing_old_anchor() {
    let database = TempDatabase::new("budget-upgrade-v28");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-v28");
    save_extended_recovery_anchor(&mut store, "upgrade-v28", ITEM_ID, 500, 100);
    let before = store
        .load_nisar_subset_execution_anchor_v2(
            "upgrade-v28",
            ITEM_ID,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .unwrap();
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch("DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers; DROP TABLE IF EXISTS nisar_budget_upgrade_v2; PRAGMA user_version=28;")
        .unwrap();
    drop(connection);
    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store
            .load_nisar_subset_execution_anchor_v2(
                "upgrade-v28",
                ITEM_ID,
                NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
            )
            .unwrap(),
        before
    );
    let connection = Connection::open(database.path()).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM nisar_budget_upgrade_v2", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        insar_store::STORE_SCHEMA_VERSION
    );
}

/// Two real active items of one task, both with a frozen plan and a science anchor.
fn create_two_active_planned_items(store: &mut TaskStore, database: &Path, task_id: &str) {
    let items = vec![source_item(), source_item_two()];
    let ids = vec![ITEM_ID.to_owned(), ITEM_ID_2.to_owned()];
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "nisar-source-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-09-04T00:00:00Z".to_owned(),
                aoi_name: "Batch AOI".to_owned(),
                aoi_geojson: Some(rectangle(0.0, 0.0, 10.0, 10.0)),
                query: Map::new(),
                search_stats: None,
                candidates: items.clone(),
            },
            workbench: SearchWorkbenchState {
                batch_id: "nisar-source-batch".to_owned(),
                checked_item_ids: ids.clone(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    let intents = [prepared_intent(), prepared_intent_two()];
    let task_items: Vec<_> = intents
        .iter()
        .zip(items)
        .map(|(intent, item)| task_item_for(intent, item))
        .collect();
    let mut request = task_request(database, task_id);
    request.selected_item_ids = ids.clone();
    request.checked_item_ids = ids.clone();
    store
        .create_paused_task_with_explicit_items(&request, 0, &ids, &task_items)
        .unwrap();
    for (revision, kind, selected) in [
        (0, TaskStateCommandKind::ResumeTask, vec![]),
        (1, TaskStateCommandKind::ResumeItems, ids.clone()),
        (2, TaskStateCommandKind::ActivateItems, ids.clone()),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind,
                    item_ids: selected,
                },
                event_id: format!("{task_id}-{revision}"),
                occurred_at: "2026-09-04T00:00:10Z".to_owned(),
            })
            .unwrap();
    }
    for (index, intent) in intents.into_iter().enumerate() {
        store
            .attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
                task_id: task_id.to_owned(),
                item_id: ids[index].clone(),
                prepared_intent_sha256: intent.canonical_intent_sha256.clone(),
                expected_revision: 3 + index as u64,
                occurred_at: "2026-09-04T00:00:40Z".to_owned(),
                plan: plan_with_graph(intent, graph()),
            })
            .unwrap();
        save_extended_recovery_anchor(store, task_id, &ids[index], 500, 100);
    }
}

/// Review P1 (2026-10-17): two items of one ASF task run concurrently. The
/// sibling's progress write, its terminal result and its runtime transition all
/// advance task-level revisions between this item's context capture and its
/// IMMEDIATE admission. Only this item's own rows decide admission; the captured
/// task-level revisions are retained in the proof as observations.
#[test]
fn budget_upgrade_admission_ignores_sibling_progress_result_and_runtime_writes() {
    let database = TempDatabase::new("budget-sibling-writes");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_two_active_planned_items(&mut store, database.path(), "upgrade-siblings");
    let proof = proof_for_item(&store, "upgrade-siblings", ITEM_ID);
    let captured = proof.context().clone();
    store
        .save_scheduler_download_progress(&DownloadItemProgress {
            task_id: "upgrade-siblings".to_owned(),
            item_id: ITEM_ID_2.to_owned(),
            status: DownloadItemStatus::Active,
            bytes_completed: 9,
            total_bytes: None,
            bytes_per_second: 3.0,
            attempt: 1,
            updated_at: "2026-10-16T12:00:01Z".to_owned(),
        })
        .unwrap();
    let sibling_result = DownloadItemResult {
        task_id: "upgrade-siblings".to_owned(),
        item_id: ITEM_ID_2.to_owned(),
        outcome: DownloadOutcome::Failed,
        destination: database.path().join("output"),
        bytes_written: 9,
        attempt_count: 1,
        failure: Some(insar_core::DownloadFailure {
            code: "SIBLING_PROVIDER_FAILURE".to_owned(),
            reason_zh: "Sibling failed".to_owned(),
            retryable: true,
        }),
        finished_at: "2026-10-16T12:00:02Z".to_owned(),
    };
    store
        .save_scheduler_download_result(&sibling_result)
        .unwrap();
    let runtime = store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: "upgrade-siblings".to_owned(),
            expected_revision: captured.runtime_revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec![ITEM_ID_2.to_owned()],
            },
            event_id: "sibling-failed".to_owned(),
            occurred_at: "2026-10-16T12:00:02Z".to_owned(),
        })
        .unwrap();
    assert!(runtime.active_item_ids.contains(&ITEM_ID.to_owned()));
    let current = store
        .load_nisar_budget_upgrade_context_v2("upgrade-siblings", ITEM_ID)
        .unwrap();
    assert!(current.runtime_revision > captured.runtime_revision);
    assert!(current.download_revision > captured.download_revision);
    assert!(
        store
            .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
            .unwrap(),
        "sibling writes must not refuse this item's upgrade"
    );
    let stored = store
        .load_nisar_budget_upgrade_proof_v2("upgrade-siblings", ITEM_ID)
        .unwrap()
        .unwrap();
    assert_eq!(stored.context().runtime_revision, captured.runtime_revision);
    assert_eq!(
        stored.context().download_revision,
        captured.download_revision
    );
    // The sibling's rows are untouched by this item's upgrade.
    let snapshot = store.load_download_snapshot("upgrade-siblings").unwrap();
    assert_eq!(snapshot.results, vec![sibling_result]);
    assert_eq!(upgrade_events(&store, "upgrade-siblings").len(), 1);
}

/// The former "download" refusal case: a task-level download revision advanced
/// by another writer, with this item's own rows unchanged, is admitted.
#[test]
fn budget_upgrade_admission_ignores_a_task_level_download_revision_advance() {
    let database = TempDatabase::new("budget-download-revision");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-download-revision");
    save_extended_recovery_anchor(&mut store, "upgrade-download-revision", ITEM_ID, 500, 100);
    let proof = proof_for(&store, "upgrade-download-revision");
    Connection::open(database.path()).unwrap().execute("INSERT INTO download_engine_state(task_id,revision,updated_at) VALUES ('upgrade-download-revision',1,'changed') ON CONFLICT(task_id) DO UPDATE SET revision=revision+1", []).unwrap();
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .unwrap());
}

/// Review P2: an item cancelled earlier and continued again keeps its Cancelled
/// result row until the new outcome replaces it; that is a valid predecessor.
#[test]
fn budget_upgrade_admits_a_continued_item_whose_previous_result_was_cancelled() {
    let database = TempDatabase::new("budget-cancelled-result");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-cancelled");
    save_extended_recovery_anchor(&mut store, "upgrade-cancelled", ITEM_ID, 500, 100);
    let cancelled = DownloadItemResult {
        task_id: "upgrade-cancelled".to_owned(),
        item_id: ITEM_ID.to_owned(),
        outcome: DownloadOutcome::Cancelled,
        destination: database.path().join("output"),
        bytes_written: 500,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-10-16T11:54:35Z".to_owned(),
    };
    store.save_scheduler_download_result(&cancelled).unwrap();
    let context = store.load_nisar_budget_upgrade_context_v2("upgrade-cancelled", ITEM_ID);
    assert!(
        context.is_ok(),
        "a Cancelled predecessor row must not refuse the upgrade: {context:?}"
    );
    let proof = proof_for(&store, "upgrade-cancelled");
    assert!(matches!(
        &proof.context().previous_result,
        insar_store::StoreNisarBudgetUpgradeDownloadResultV2::Present { outcome, .. }
            if outcome == "cancelled"
    ));
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .unwrap());
}

fn resealed_proof_row(
    store: &TaskStore,
    database: &Path,
    task_id: &str,
    edit: impl FnOnce(&mut Value),
) -> Result<Option<StoredNisarBudgetUpgradeProofV2>, StoreError> {
    let proof = proof_for(store, task_id);
    let mut document: Value = serde_json::from_str(&proof.canonical_json().unwrap()).unwrap();
    edit(&mut document);
    let canonical = document.to_string();
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    Connection::open(database)
        .unwrap()
        .execute(
            "INSERT INTO nisar_budget_upgrade_v2(task_id,item_id,proof_sha256,canonical_json) VALUES (?1,?2,?3,?4)",
            params![task_id, ITEM_ID, digest, canonical],
        )
        .unwrap();
    store.load_nisar_budget_upgrade_proof_v2(task_id, ITEM_ID)
}

/// Review P2: the proof shape changed (raw progress row, pre-reconciliation
/// state), so its version is 2. A digest-intact proof of this contract with any
/// other version gets an explicit local diagnostic, never a reinterpretation.
#[test]
fn budget_upgrade_reports_an_unsupported_proof_version_explicitly() {
    for (label, found) in [("v1", 1_u64), ("v1-without-progress", 1), ("v3", 3)] {
        let database = TempDatabase::new(&format!("budget-proof-version-{label}"));
        let mut store = TaskStore::open(database.path()).unwrap();
        create_active_planned_task(&mut store, database.path(), "upgrade-version");
        save_extended_recovery_anchor(&mut store, "upgrade-version", ITEM_ID, 500, 100);
        let loaded = resealed_proof_row(&store, database.path(), "upgrade-version", |document| {
            assert_eq!(document["schema_version"], json!(2));
            document["schema_version"] = json!(found);
            if label == "v1-without-progress" {
                document["context"]
                    .as_object_mut()
                    .unwrap()
                    .remove("previous_progress");
            }
        });
        match loaded {
            Err(StoreError::NisarBudgetUpgradeProofVersionUnsupported { found: actual }) => {
                assert_eq!(actual, found, "{label}")
            }
            other => panic!("{label}: expected an explicit version diagnostic, got {other:?}"),
        }
    }
}

/// Review P2: the exact non-quiescent legacy state that reconciliation settled
/// is part of the immutable proof; a quiescent copy cannot pose as one.
#[test]
fn budget_upgrade_proof_retains_exact_pre_reconciliation_state_and_rejects_quiescent_copies() {
    let database = TempDatabase::new("budget-pre-reconciliation");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-pre");
    save_extended_recovery_anchor(&mut store, "upgrade-pre", ITEM_ID, 500, 100);
    let base = proof_for(&store, "upgrade-pre");
    // A coherent pending-transition anchor row of the same lineage stands in
    // for the raw legacy row before reconciliation committed it.
    let pending = next_anchor(&base);
    let mut local: Value =
        serde_json::from_str(&base.evidence().science_checkpoint_canonical_json).unwrap();
    local["pending_attempt"] = json!({"state": "Reserved", "range_index": 1});
    let pre = StoreNisarBudgetUpgradePreReconciliationV2 {
        science_anchor_record_sha256: pending.record_sha256.clone(),
        science_anchor_canonical_json: pending.canonical_json.clone(),
        local_checkpoint_canonical_json: Some(local.to_string()),
    };
    let mut evidence = base.evidence().clone();
    evidence.pre_reconciliation = Some(pre.clone());
    let proof = StoredNisarBudgetUpgradeProofV2::new(
        base.context().clone(),
        "2026-10-16T12:00:00Z".to_owned(),
        evidence.clone(),
    )
    .unwrap();
    assert!(store
        .prepare_nisar_budget_upgrade_v2(&proof, &next_anchor(&proof))
        .unwrap());
    let stored = store
        .load_nisar_budget_upgrade_proof_v2("upgrade-pre", ITEM_ID)
        .unwrap()
        .unwrap();
    assert_eq!(stored.evidence().pre_reconciliation, Some(pre));
    for tamper in ["quiescent-anchor", "other-item-local", "digest"] {
        let mut evidence = evidence.clone();
        let pre = evidence.pre_reconciliation.as_mut().unwrap();
        match tamper {
            "quiescent-anchor" => {
                pre.science_anchor_record_sha256 =
                    base.context().science_anchor.record_sha256.clone();
                pre.science_anchor_canonical_json =
                    base.context().science_anchor.canonical_json.clone();
                pre.local_checkpoint_canonical_json =
                    Some(base.evidence().science_checkpoint_canonical_json.clone());
            }
            "other-item-local" => {
                let mut other = local.clone();
                other["item_id"] = json!(ITEM_ID_2);
                pre.local_checkpoint_canonical_json = Some(other.to_string());
            }
            "digest" => pre.science_anchor_record_sha256 = "0".repeat(64),
            _ => unreachable!(),
        }
        assert!(
            StoredNisarBudgetUpgradeProofV2::new(
                base.context().clone(),
                "2026-10-16T12:00:00Z".to_owned(),
                evidence
            )
            .is_err(),
            "{tamper}"
        );
    }
}

/// Review gap: a later write in the refusal transaction fails (here the task
/// event). The whole transaction rolls back: no evidence row, the old result,
/// progress and runtime are unchanged, and a retry records exactly once.
#[test]
fn budget_upgrade_rejection_rolls_back_evidence_when_a_later_write_fails() {
    let database = TempDatabase::new("budget-rejection-rollback");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "upgrade-rejection-rollback");
    save_extended_recovery_anchor(&mut store, "upgrade-rejection-rollback", ITEM_ID, 500, 100);
    let old = DownloadItemResult {
        task_id: "upgrade-rejection-rollback".to_owned(),
        item_id: ITEM_ID.to_owned(),
        outcome: DownloadOutcome::Failed,
        destination: database.path().join("output"),
        bytes_written: 500,
        attempt_count: 1,
        failure: Some(insar_core::DownloadFailure {
            code: "old-provider-error".to_owned(),
            reason_zh: "Original provider response".to_owned(),
            retryable: false,
        }),
        finished_at: "2026-10-16T11:54:35Z".to_owned(),
    };
    store.save_scheduler_download_result(&old).unwrap();
    let connection = Connection::open(database.path()).unwrap();
    let raw_result = |connection: &Connection| -> (String, String, i64) {
        connection
            .query_row(
                "SELECT outcome,result_json,bytes_written FROM download_results WHERE task_id='upgrade-rejection-rollback'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    };
    let result_before = raw_result(&connection);
    let before = store
        .load_nisar_budget_upgrade_context_v2("upgrade-rejection-rollback", ITEM_ID)
        .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_rejection_event BEFORE INSERT ON task_events \
             BEGIN SELECT RAISE(ABORT, 'injected task event failure'); END;",
        )
        .unwrap();
    let request = StoreUpdateTaskRequest {
        task_id: "upgrade-rejection-rollback".to_owned(),
        expected_revision: before.runtime_revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::MarkFailed,
            item_ids: vec![ITEM_ID.to_owned()],
        },
        event_id: "upgrade-rejection-rollback-event".to_owned(),
        occurred_at: "2026-10-16T12:00:00Z".to_owned(),
    };
    let rejection = rejection_result(
        "upgrade-rejection-rollback",
        database.path(),
        "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
    );
    assert!(store
        .finish_nisar_budget_upgrade_rejection(&request, &rejection)
        .is_err());
    assert!(store
        .list_nisar_budget_upgrade_refusals_v2("upgrade-rejection-rollback", ITEM_ID)
        .unwrap()
        .is_empty());
    assert_eq!(raw_result(&connection), result_before);
    let after = store
        .load_nisar_budget_upgrade_context_v2("upgrade-rejection-rollback", ITEM_ID)
        .unwrap();
    assert_eq!(
        after, before,
        "runtime, download revision and rows roll back"
    );
    connection
        .execute_batch("DROP TRIGGER fail_rejection_event;")
        .unwrap();
    store
        .finish_nisar_budget_upgrade_rejection(&request, &rejection)
        .unwrap();
    let refusals = store
        .list_nisar_budget_upgrade_refusals_v2("upgrade-rejection-rollback", ITEM_ID)
        .unwrap();
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].previous_result(), &before.previous_result);
}

/// Review 2026-10-17 round 2, P2-1: when the refusal evidence could not be
/// retained, the fallback replaces no download row (the exact result row is the
/// evidence that could not be copied), keeps the diagnostic on the task event
/// and still honours a concurrent task pause or item cancellation. Only an item
/// without any download row gets the visible failure result.
#[test]
fn budget_upgrade_unretained_rejection_replaces_no_row_and_never_overrides_a_stop() {
    let item = ITEM_ID.to_owned();
    for case in ["result-row", "paused-task", "cancelled-item", "no-row"] {
        let database = TempDatabase::new(&format!("budget-unretained-{case}"));
        let task_id = format!("upgrade-unretained-{case}");
        let mut store = TaskStore::open(database.path()).unwrap();
        create_active_planned_task(&mut store, database.path(), &task_id);
        save_extended_recovery_anchor(&mut store, &task_id, ITEM_ID, 500, 100);
        if case != "no-row" {
            store
                .save_scheduler_download_result(&DownloadItemResult {
                    task_id: task_id.clone(),
                    item_id: item.clone(),
                    outcome: DownloadOutcome::Failed,
                    destination: database.path().join("output"),
                    bytes_written: 500,
                    attempt_count: 1,
                    failure: Some(insar_core::DownloadFailure {
                        code: "old-provider-error".to_owned(),
                        reason_zh: "Original provider response".to_owned(),
                        retryable: false,
                    }),
                    finished_at: "2026-10-16T11:54:35Z".to_owned(),
                })
                .unwrap();
        }
        let mut revision = runtime_revision(&store, &task_id);
        if matches!(case, "paused-task" | "cancelled-item") {
            let (kind, item_ids) = if case == "paused-task" {
                (TaskStateCommandKind::PauseTask, vec![])
            } else {
                (TaskStateCommandKind::CancelItems, vec![item.clone()])
            };
            store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: task_id.clone(),
                    expected_revision: revision,
                    command: StoreTaskCommand { kind, item_ids },
                    event_id: format!("{case}-stop-first"),
                    occurred_at: "2026-10-16T12:00:00Z".to_owned(),
                })
                .unwrap();
            revision += 1;
        }
        let rows_before = store.load_download_snapshot(&task_id).unwrap();
        let request = StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec![item.clone()],
            },
            event_id: format!("{case}-unretained"),
            occurred_at: "2026-10-16T12:00:01Z".to_owned(),
        };
        let rejection = rejection_result(
            &task_id,
            database.path(),
            "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
        );
        let runtime = store
            .finish_nisar_budget_upgrade_rejection_unretained(&request, &rejection)
            .unwrap();
        assert_eq!(
            store
                .finish_nisar_budget_upgrade_rejection_unretained(&request, &rejection)
                .unwrap(),
            runtime,
            "{case}: an exact replay is idempotent"
        );
        assert!(
            store
                .list_nisar_budget_upgrade_refusals_v2(&task_id, ITEM_ID)
                .unwrap()
                .is_empty(),
            "{case}: no evidence is claimed"
        );
        let rows_after = store.load_download_snapshot(&task_id).unwrap();
        assert!(rows_after.progress.is_empty(), "{case}");
        match case {
            "result-row" => {
                assert!(runtime.failed_item_ids.contains(&item));
                assert!(!runtime.active_item_ids.contains(&item));
                assert_eq!(
                    rows_after.results, rows_before.results,
                    "the evidence row is not overwritten"
                );
                let events = store.list_task_events(&task_id).unwrap();
                let event = events.last().unwrap();
                assert_eq!(event.event_type, "mark_failed");
                assert_eq!(
                    event.error,
                    Some(serde_json::to_value(rejection.failure.as_ref().unwrap()).unwrap())
                );
            }
            "paused-task" => {
                assert!(runtime.paused_item_ids.contains(&item));
                assert!(!runtime.failed_item_ids.contains(&item));
                assert_eq!(rows_after.results, rows_before.results);
            }
            "cancelled-item" => {
                assert!(runtime.cancelled_item_ids.contains(&item));
                assert!(!runtime.failed_item_ids.contains(&item));
                assert_eq!(rows_after.results, rows_before.results);
            }
            _ => {
                assert!(runtime.failed_item_ids.contains(&item));
                assert_eq!(
                    rows_after.results,
                    vec![rejection.clone()],
                    "nothing is replaced, so the local failure is shown"
                );
            }
        }
    }
}

/// Review 2026-10-17 round 3, P2-3: an unretained rejection keeps the prior
/// result row (here a provider authentication failure) and records its local
/// diagnostic only on the item's failure event. The task card codes and the
/// task page read that event as the item's current failure, so the card no
/// longer asks for a login check and the detail shows the local refusal. A
/// later ordinary failure records no failure on its event, so its result row
/// is current again.
#[test]
fn budget_upgrade_unretained_rejection_is_the_current_failure_of_card_and_page() {
    const REFUSED: &str = "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED";
    let database = TempDatabase::new("budget-unretained-current");
    let task_id = "upgrade-unretained-current".to_owned();
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), &task_id);
    let result =
        |code: &str, reason_zh: &str, retryable: bool, finished_at: &str| DownloadItemResult {
            task_id: task_id.clone(),
            item_id: ITEM_ID.to_owned(),
            outcome: DownloadOutcome::Failed,
            destination: database.path().join("output"),
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(insar_core::DownloadFailure {
                code: code.to_owned(),
                reason_zh: reason_zh.to_owned(),
                retryable,
            }),
            finished_at: finished_at.to_owned(),
        };
    let provider = result(
        "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED",
        "Bhoonidhi 认证刷新失败 [stage=science_auth_refresh]",
        false,
        "2026-10-16T11:54:35Z",
    );
    store.save_scheduler_download_result(&provider).unwrap();
    let request =
        |store: &TaskStore, kind, event_id: &str, occurred_at: &str| StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: runtime_revision(store, &task_id),
            command: StoreTaskCommand {
                kind,
                item_ids: vec![ITEM_ID.to_owned()],
            },
            event_id: event_id.to_owned(),
            occurred_at: occurred_at.to_owned(),
        };
    let rejection = rejection_result(&task_id, database.path(), REFUSED);
    let unretained = request(
        &store,
        TaskStateCommandKind::MarkFailed,
        "current-unretained",
        "2026-10-16T12:00:01Z",
    );
    store
        .finish_nisar_budget_upgrade_rejection_unretained(&unretained, &rejection)
        .unwrap();
    assert_eq!(
        store.load_download_snapshot(&task_id).unwrap().results,
        vec![provider.clone()],
        "the prior result row stays in place"
    );
    let card = |store: &TaskStore| {
        store
            .core_restore(false)
            .unwrap()
            .tasks
            .into_iter()
            .find(|task| task.task_id == task_id)
            .unwrap()
            .failure_codes
    };
    let page_failure = |store: &TaskStore| {
        store
            .load_task_page(&insar_core::StoreTaskPageRequest {
                task_id: task_id.clone(),
                expected_revision: 0,
                page_index: 0,
                page_size: 10,
            })
            .unwrap()
            .items[0]
            .failure
            .clone()
    };
    assert_eq!(
        card(&store),
        vec![insar_store::StoreTaskFailureCode {
            code: REFUSED.to_owned(),
            local: false,
            tls_certificate: false,
            network: false,
        }]
    );
    assert_eq!(page_failure(&store), rejection.failure);

    let retry = request(
        &store,
        TaskStateCommandKind::RetryItems,
        "current-retry",
        "2026-10-16T12:00:02Z",
    );
    store.update_task(&retry).unwrap();
    let activate = request(
        &store,
        TaskStateCommandKind::ActivateItems,
        "current-activate",
        "2026-10-16T12:00:03Z",
    );
    store.update_scheduler_runtime(&activate).unwrap();
    let ordinary = result(
        "NISAR_SUBSET_V2_SCIENCE_INCOMPLETE",
        "第 3/9 段 连接失败或超时，已自动重试 2 次仍失败，断点保留，可稍后重试",
        true,
        "2026-10-16T12:00:04Z",
    );
    store.save_scheduler_download_result(&ordinary).unwrap();
    let failed = request(
        &store,
        TaskStateCommandKind::MarkFailed,
        "current-ordinary",
        "2026-10-16T12:00:04Z",
    );
    store.update_scheduler_runtime(&failed).unwrap();
    assert_eq!(
        card(&store),
        vec![insar_store::StoreTaskFailureCode {
            code: "NISAR_SUBSET_V2_SCIENCE_INCOMPLETE".to_owned(),
            local: false,
            tls_certificate: false,
            network: false,
        }]
    );
    assert_eq!(page_failure(&store), ordinary.failure);
}
/// The new-shell failure summary reads an item's latest `mark_failed` event
/// for every listed task but lets it win only for event-scoped (NISAR subset)
/// tasks, exactly like the legacy restore's failure codes. A refused budget
/// upgrade whose result row still holds an older provider failure proves it:
/// the scoped summary and the legacy codes both report the refusal, and the
/// unscoped summary falls back to the row.
#[test]
fn failure_summary_event_scope_matches_legacy_failure_codes_for_a_subset_task() {
    let database = TempDatabase::new("failure-summary-scope");
    let mut store = TaskStore::open(database.path()).unwrap();
    create_active_planned_task(&mut store, database.path(), "summary-scope");
    save_extended_recovery_anchor(&mut store, "summary-scope", ITEM_ID, 500, 100);
    let before = store
        .load_nisar_budget_upgrade_context_v2("summary-scope", ITEM_ID)
        .unwrap();
    let rejection = rejection_result(
        "summary-scope",
        database.path(),
        "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
    );
    store
        .finish_nisar_budget_upgrade_rejection(
            &StoreUpdateTaskRequest {
                task_id: "summary-scope".to_owned(),
                expected_revision: before.runtime_revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![ITEM_ID.to_owned()],
                },
                event_id: "summary-scope-rejected".to_owned(),
                occurred_at: "2026-10-16T12:00:00Z".to_owned(),
            },
            &rejection,
        )
        .unwrap();
    let mut older = rejection.clone();
    older.failure = Some(insar_core::DownloadFailure {
        code: "OLD_PROVIDER_ERROR".to_owned(),
        reason_zh: "Original provider response".to_owned(),
        retryable: false,
    });
    Connection::open(database.path())
        .unwrap()
        .execute(
            "UPDATE download_results SET result_json=?1 WHERE task_id='summary-scope'",
            [serde_json::to_string(&older).unwrap()],
        )
        .unwrap();

    let legacy = store
        .core_restore(false)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == "summary-scope")
        .unwrap();
    assert_eq!(legacy.task_type.as_deref(), Some("nisar_subset"));
    let legacy_codes = legacy
        .failure_codes
        .iter()
        .map(|code| code.code.clone())
        .collect::<Vec<_>>();
    assert_eq!(legacy_codes, ["NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED"]);

    let ids = ["summary-scope".to_owned()];
    let scoped = store.load_task_failure_summaries(&ids, &ids).unwrap();
    let scoped = &scoped["summary-scope"];
    assert_eq!(scoped.codes, legacy_codes);
    assert_eq!(
        scoped.message_zh.as_deref(),
        Some(rejection.failure.as_ref().unwrap().reason_zh.as_str())
    );
    let unscoped = store.load_task_failure_summaries(&ids, &[]).unwrap();
    assert_eq!(unscoped["summary-scope"].codes, ["OLD_PROVIDER_ERROR"]);
}
