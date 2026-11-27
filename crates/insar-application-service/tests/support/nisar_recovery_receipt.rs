//! Offline audit receipts preserve failed live evidence and never resume a task.
use super::*;
const TASK_ID: &str = "nisar-live-acceptance-task";

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflinePhysicalLogicalOwnerV2 {
    dataset_path: String,
    polarization: Option<String>,
    component_id: Option<u16>,
    window_sha256: Option<String>,
    logical_chunk: Option<[u64; 2]>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflinePhysicalRangeV2 {
    start: u64,
    end_exclusive: u64,
    logical_owners: Vec<OfflinePhysicalLogicalOwnerV2>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineAllocatedRangeEvidenceV2 {
    intervals: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineCompletedScienceRangeV2 {
    index: u16,
    start: u64,
    end_exclusive: u64,
    request_count: u64,
    response_sha256: String,
    allocated_range_evidence: OfflineAllocatedRangeEvidenceV2,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflinePendingAttemptV2 {
    range_index: u16,
    start: u64,
    end_exclusive: u64,
    reserved_response_body_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reserved_request_count: Option<u64>,
    state: String,
    request_issued: bool,
    request_count: u64,
    observed_response_body_bytes: u64,
}

#[test]
fn offline_pending_request_allowance_preserves_legacy_and_new_canonical() {
    for raw in [
        r#"{"range_index":0,"start":0,"end_exclusive":32,"reserved_response_body_bytes":32,"state":"ResponseRecorded","request_issued":true,"request_count":5,"observed_response_body_bytes":32}"#,
        r#"{"range_index":0,"start":0,"end_exclusive":32,"reserved_response_body_bytes":32,"reserved_request_count":22,"state":"ResponseRecorded","request_issued":true,"request_count":5,"observed_response_body_bytes":32}"#,
    ] {
        let pending: OfflinePendingAttemptV2 = serde_json::from_str(raw).unwrap();
        assert_eq!(serde_json::to_string(&pending).unwrap(), raw);
        assert_eq!(
            sha256_bytes(&serde_json::to_vec(&pending).unwrap()),
            sha256_bytes(raw.as_bytes())
        );
    }
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineBudgetUpgradeLimitsV2 {
    max_request_count: u64,
    max_science_request_count: u64,
    max_response_body_bytes: u64,
}

/// Mirrors the compact receipt a one-time NISAR budget upgrade adds to a
/// checkpoint; absence keeps the legacy canonical text and digest.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineBudgetUpgradeReceiptV2 {
    contract: String,
    proof_sha256: String,
    limits: OfflineBudgetUpgradeLimitsV2,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineCheckpointV2 {
    checkpoint_contract: String,
    schema_version: u32,
    workspace_canonical_path: String,
    workspace_identity: String,
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    anchor_generation: u64,
    previous_checkpoint_sha256: Option<String>,
    part_identity_sha256: String,
    task_id: String,
    item_id: String,
    prepared_intent_sha256: String,
    final_plan_sha256: String,
    physical_plan_sha256: String,
    inventory_sha256: String,
    inventory_evidence_sha256: String,
    source_graph_sha256: String,
    source_product_identity: insar_core::NisarSubsetSourceProductIdentityV2,
    source_asset_identity: insar_core::NisarSubsetSourceAssetIdentityV2,
    source_validator: insar_core::NisarSubsetSourceValidatorV1,
    source_validator_sha256: String,
    source_size: u64,
    planned_ranges: Vec<OfflinePhysicalRangeV2>,
    completed_ranges: Vec<OfflineCompletedScienceRangeV2>,
    pending_attempt: Option<OfflinePendingAttemptV2>,
    committed_planning_request_count: u64,
    committed_planning_response_body_bytes: u64,
    reserved_science_request_count: u64,
    reserved_science_response_body_bytes: u64,
    issued_science_request_count: u64,
    observed_science_response_body_bytes: u64,
    failed_science_request_count: u64,
    unissued_science_reservation_count: u64,
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    resumed_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_upgrade_receipt: Option<OfflineBudgetUpgradeReceiptV2>,
    state: String,
    checkpoint_sha256: String,
}

#[derive(Serialize)]
struct OfflineCheckpointDigestIdentity<'a> {
    checkpoint_contract: &'a str,
    schema_version: u32,
    workspace_canonical_path: &'a str,
    workspace_identity: &'a str,
    lineage_key_sha256: &'a str,
    execution_binding_sha256: &'a str,
    anchor_generation: u64,
    previous_checkpoint_sha256: &'a Option<String>,
    part_identity_sha256: &'a str,
    task_id: &'a str,
    item_id: &'a str,
    prepared_intent_sha256: &'a str,
    final_plan_sha256: &'a str,
    physical_plan_sha256: &'a str,
    inventory_sha256: &'a str,
    inventory_evidence_sha256: &'a str,
    source_graph_sha256: &'a str,
    source_product_identity: &'a insar_core::NisarSubsetSourceProductIdentityV2,
    source_asset_identity: &'a insar_core::NisarSubsetSourceAssetIdentityV2,
    source_validator: &'a insar_core::NisarSubsetSourceValidatorV1,
    source_validator_sha256: &'a str,
    source_size: u64,
    planned_ranges: &'a [OfflinePhysicalRangeV2],
    completed_ranges: &'a [OfflineCompletedScienceRangeV2],
    pending_attempt: &'a Option<OfflinePendingAttemptV2>,
    committed_planning_request_count: u64,
    committed_planning_response_body_bytes: u64,
    reserved_science_request_count: u64,
    reserved_science_response_body_bytes: u64,
    issued_science_request_count: u64,
    observed_science_response_body_bytes: u64,
    failed_science_request_count: u64,
    unissued_science_reservation_count: u64,
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    resumed_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_upgrade_receipt: &'a Option<OfflineBudgetUpgradeReceiptV2>,
    state: &'a str,
}

fn offline_sha256_serialized<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("offline digest serialization");
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn offline_checkpoint_digest(checkpoint: &OfflineCheckpointV2) -> String {
    offline_sha256_serialized(&OfflineCheckpointDigestIdentity {
        checkpoint_contract: &checkpoint.checkpoint_contract,
        schema_version: checkpoint.schema_version,
        workspace_canonical_path: &checkpoint.workspace_canonical_path,
        workspace_identity: &checkpoint.workspace_identity,
        lineage_key_sha256: &checkpoint.lineage_key_sha256,
        execution_binding_sha256: &checkpoint.execution_binding_sha256,
        anchor_generation: checkpoint.anchor_generation,
        previous_checkpoint_sha256: &checkpoint.previous_checkpoint_sha256,
        part_identity_sha256: &checkpoint.part_identity_sha256,
        task_id: &checkpoint.task_id,
        item_id: &checkpoint.item_id,
        prepared_intent_sha256: &checkpoint.prepared_intent_sha256,
        final_plan_sha256: &checkpoint.final_plan_sha256,
        physical_plan_sha256: &checkpoint.physical_plan_sha256,
        inventory_sha256: &checkpoint.inventory_sha256,
        inventory_evidence_sha256: &checkpoint.inventory_evidence_sha256,
        source_graph_sha256: &checkpoint.source_graph_sha256,
        source_product_identity: &checkpoint.source_product_identity,
        source_asset_identity: &checkpoint.source_asset_identity,
        source_validator: &checkpoint.source_validator,
        source_validator_sha256: &checkpoint.source_validator_sha256,
        source_size: checkpoint.source_size,
        planned_ranges: &checkpoint.planned_ranges,
        completed_ranges: &checkpoint.completed_ranges,
        pending_attempt: &checkpoint.pending_attempt,
        committed_planning_request_count: checkpoint.committed_planning_request_count,
        committed_planning_response_body_bytes: checkpoint.committed_planning_response_body_bytes,
        reserved_science_request_count: checkpoint.reserved_science_request_count,
        reserved_science_response_body_bytes: checkpoint.reserved_science_response_body_bytes,
        issued_science_request_count: checkpoint.issued_science_request_count,
        observed_science_response_body_bytes: checkpoint.observed_science_response_body_bytes,
        failed_science_request_count: checkpoint.failed_science_request_count,
        unissued_science_reservation_count: checkpoint.unissued_science_reservation_count,
        indeterminate_science_request_count: checkpoint.indeterminate_science_request_count,
        indeterminate_science_response_body_bytes: checkpoint
            .indeterminate_science_response_body_bytes,
        committed_science_request_count: checkpoint.committed_science_request_count,
        committed_science_response_body_bytes: checkpoint.committed_science_response_body_bytes,
        resumed_bytes: checkpoint.resumed_bytes,
        budget_upgrade_receipt: &checkpoint.budget_upgrade_receipt,
        state: &checkpoint.state,
    })
}

/// Shared verbatim with the production checkpoint test in
/// crates/insar-download/src/nisar_science_transfer_v2.rs.
const GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2: &str = concat!(
    r#"{"checkpoint_contract":"NisarSubsetScienceCheckpointV2","schema_version":1,"#,
    r#""workspace_canonical_path":"recovery-workspace","#,
    r#""workspace_identity":"workspace-identity","#,
    r#""lineage_key_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","#,
    r#""execution_binding_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","#,
    r#""anchor_generation":7,"#,
    r#""previous_checkpoint_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","#,
    r#""part_identity_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","#,
    r#""task_id":"task","item_id":"item","#,
    r#""prepared_intent_sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","#,
    r#""final_plan_sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","#,
    r#""physical_plan_sha256":"1111111111111111111111111111111111111111111111111111111111111111","#,
    r#""inventory_sha256":"2222222222222222222222222222222222222222222222222222222222222222","#,
    r#""inventory_evidence_sha256":"3333333333333333333333333333333333333333333333333333333333333333","#,
    r#""source_graph_sha256":"4444444444444444444444444444444444444444444444444444444444444444","#,
    r#""source_product_identity":{"provider_id":"asf","collection_id":"C1-ASF","#,
    r#""collection_profile_id":"nisar-l1-rslc","provider_product_id":"G1-ASF","#,
    r#""provider_product_key":"PROVIDER::asf::ITEM::G1-ASF","#,
    r#""product_specification_version":"1.0","crid":"A","quality_status":"PROVISIONAL"},"#,
    r#""source_asset_identity":{"asset_key":"science-rslc-hdf5","#,
    r#""stable_asset_key":"stable-asset","file_name":"source.h5","#,
    r#""checksum":{"algorithm":"sha256","#,
    r#""value":"5555555555555555555555555555555555555555555555555555555555555555"}},"#,
    r#""source_validator":{"kind":"strong_etag","value":"\"etag\"","content_length":4096},"#,
    r#""source_validator_sha256":"6666666666666666666666666666666666666666666666666666666666666666","#,
    r#""source_size":4096,"planned_ranges":[{"start":0,"end_exclusive":32,"#,
    r#""logical_owners":[{"dataset_path":"/science/LSAR/RSLC/swaths/frequencyA/HH","#,
    r#""polarization":"HH","component_id":null,"window_sha256":null,"#,
    r#""logical_chunk":[0,0]}]},{"start":64,"end_exclusive":96,"logical_owners":[]}],"#,
    r#""completed_ranges":[{"index":0,"start":0,"end_exclusive":32,"request_count":2,"#,
    r#""response_sha256":"7777777777777777777777777777777777777777777777777777777777777777","#,
    r#""allocated_range_evidence":{"intervals":[[0,32]]}}],"pending_attempt":null,"#,
    r#""committed_planning_request_count":31,"#,
    r#""committed_planning_response_body_bytes":195035138,"#,
    r#""reserved_science_request_count":935,"#,
    r#""reserved_science_response_body_bytes":493865474,"issued_science_request_count":935,"#,
    r#""observed_science_response_body_bytes":493865474,"failed_science_request_count":933,"#,
    r#""unissued_science_reservation_count":0,"indeterminate_science_request_count":0,"#,
    r#""indeterminate_science_response_body_bytes":0,"committed_science_request_count":2,"#,
    r#""committed_science_response_body_bytes":32,"resumed_bytes":0,"state":"Interrupted"}"#,
);
const GOLDEN_BUDGET_UPGRADE_RECEIPT_V2: &str = concat!(
    r#"{"contract":"nisar_science_budget_upgrade_v1","#,
    r#""proof_sha256":"9999999999999999999999999999999999999999999999999999999999999999","#,
    r#""limits":{"max_request_count":1054,"max_science_request_count":1023,"#,
    r#""max_response_body_bytes":1186766086}}"#,
);

fn sealed_golden_checkpoint(identity: &str) -> String {
    format!(
        "{},\"checkpoint_sha256\":\"{}\"}}",
        &identity[..identity.len() - 1],
        sha256_bytes(identity.as_bytes())
    )
}

#[test]
fn offline_checkpoint_budget_receipt_preserves_legacy_and_upgraded_canonical_digest() {
    let upgraded = GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2.replacen(
        ",\"state\":",
        &format!(",\"budget_upgrade_receipt\":{GOLDEN_BUDGET_UPGRADE_RECEIPT_V2},\"state\":"),
        1,
    );
    for (identity, has_receipt) in [
        (GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2.to_owned(), false),
        (upgraded.clone(), true),
    ] {
        let canonical = sealed_golden_checkpoint(&identity);
        let checkpoint: OfflineCheckpointV2 = serde_json::from_str(&canonical).unwrap();
        assert_eq!(serde_json::to_string(&checkpoint).unwrap(), canonical);
        assert_eq!(
            offline_checkpoint_digest(&checkpoint),
            checkpoint.checkpoint_sha256
        );
        assert_eq!(
            checkpoint.checkpoint_sha256,
            sha256_bytes(identity.as_bytes())
        );
        assert_eq!(checkpoint.budget_upgrade_receipt.is_some(), has_receipt);
    }
    for tampered in [
        upgraded.replacen("\"limits\":{", "\"limits\":{\"extra\":1,", 1),
        upgraded.replacen("\"contract\":", "\"extra\":1,\"contract\":", 1),
    ] {
        assert!(
            serde_json::from_str::<OfflineCheckpointV2>(&sealed_golden_checkpoint(&tampered))
                .is_err()
        );
    }
}

fn load_and_verify_offline_checkpoint(
    output: &Path,
    observed: &LiveRecoveryCheckpointObservation,
) -> OfflineCheckpointV2 {
    let path = find_live_recovery_checkpoint(output).expect("offline checkpoint path");
    let bytes = fs::read(path).expect("offline checkpoint bytes");
    let raw: Value = serde_json::from_slice(&bytes).expect("offline checkpoint JSON");
    let checkpoint: OfflineCheckpointV2 =
        serde_json::from_slice(&bytes).expect("offline checkpoint schema");
    assert_eq!(checkpoint.task_id, observed.task_id);
    assert_eq!(checkpoint.item_id, observed.item_id);
    assert_eq!(checkpoint.anchor_generation, observed.anchor_generation);
    assert_eq!(checkpoint.checkpoint_sha256, observed.checkpoint_sha256);
    assert_eq!(
        offline_checkpoint_digest(&checkpoint),
        checkpoint.checkpoint_sha256
    );
    // `checkpoint_sha256` is sealed over the typed checkpoint identity, whose
    // nested range structs use production field order.  The live observation
    // digest is deliberately computed from a `serde_json::Value`; serde_json
    // canonicalizes object keys for that map representation, so it has a
    // different (but deterministic) byte stream.  Verify both algorithms
    // instead of treating the map digest as the production typed digest.
    let raw_completed_ranges = raw
        .get("completed_ranges")
        .expect("offline completed ranges JSON");
    assert_eq!(
        offline_sha256_serialized(raw_completed_ranges),
        observed.completed_ranges_digest
    );
    assert_eq!(
        checkpoint.planned_ranges.len(),
        observed.planned_ranges.len()
    );
    for (index, (actual, observed)) in checkpoint
        .planned_ranges
        .iter()
        .zip(&observed.planned_ranges)
        .enumerate()
    {
        assert_eq!(actual.start, observed.start);
        assert_eq!(actual.end_exclusive, observed.end_exclusive);
        assert_eq!(index as u64, observed.index);
    }
    assert_eq!(
        checkpoint.completed_ranges.len(),
        observed.completed_ranges.len()
    );
    assert_eq!(
        checkpoint.completed_ranges.len(),
        observed.completed_range_request_counts.len()
    );
    for (actual, (observed, count)) in checkpoint.completed_ranges.iter().zip(
        observed
            .completed_ranges
            .iter()
            .zip(&observed.completed_range_request_counts),
    ) {
        assert_eq!(actual.index as u64, observed.index);
        assert_eq!(actual.start, observed.start);
        assert_eq!(actual.end_exclusive, observed.end_exclusive);
        assert_eq!(actual.request_count, *count);
    }
    checkpoint
}

fn assert_offline_science_range_bindings(
    evidence: &NisarLiveAcceptanceEvidenceV2,
    checkpoint: &OfflineCheckpointV2,
) {
    let science = evidence
        .range_observations
        .iter()
        .filter(|observation| observation.stage == "science")
        .collect::<Vec<_>>();
    assert_eq!(science.len(), checkpoint.completed_ranges.len());
    let mut request_count = 0_u64;
    let mut response_bytes = 0_u64;
    for completed in &checkpoint.completed_ranges {
        let matches = science
            .iter()
            .filter(|observation| {
                observation.start == completed.start
                    && observation.end_exclusive == completed.end_exclusive
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "each checkpoint range needs one science observation"
        );
        let observation = matches[0];
        let range_bytes = completed
            .end_exclusive
            .checked_sub(completed.start)
            .expect("positive checkpoint range");
        assert_eq!(observation.source_size_bytes, evidence.source_size_bytes);
        assert_eq!(observation.request_count, completed.request_count);
        assert!(observation.request_count > 0);
        assert_eq!(observation.http_status, 206);
        assert_eq!(observation.content_range_start, completed.start);
        assert_eq!(
            observation.content_range_end_exclusive,
            completed.end_exclusive
        );
        assert_eq!(observation.content_range_total, evidence.source_size_bytes);
        assert_eq!(observation.response_body_bytes, range_bytes);
        assert_eq!(
            observation.validator_sha256,
            evidence.source_validator_sha256
        );
        request_count = request_count
            .checked_add(observation.request_count)
            .expect("science request count");
        response_bytes = response_bytes
            .checked_add(observation.response_body_bytes)
            .expect("science response bytes");
    }
    assert_eq!(request_count, checkpoint.committed_science_request_count);
    assert_eq!(
        response_bytes,
        checkpoint.committed_science_response_body_bytes
    );
    assert_eq!(request_count, evidence.science_range.requests);
    assert_eq!(response_bytes, evidence.science_range.response_bytes);
    assert_eq!(request_count, evidence.committed_science_requests);
    assert_eq!(response_bytes, evidence.committed_science_response_bytes);
}

fn assert_offline_anchor_digest(
    database: &Path,
    item_id: &str,
    checkpoint: &OfflineCheckpointV2,
    observed: &LiveRecoveryAnchorObservation,
) {
    let store = TaskStore::open(database).expect("offline anchor store");
    let anchor = store
        .load_nisar_subset_execution_anchor_v2(
            TASK_ID,
            item_id,
            NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
        )
        .expect("offline anchor load")
        .expect("offline anchor row");
    let canonical: Value = serde_json::from_str(&anchor.canonical_json).expect("anchor JSON");
    let lineage = canonical["lineage_key_sha256"]
        .as_str()
        .expect("anchor lineage")
        .to_owned();
    let execution_binding = canonical["execution_binding_sha256"]
        .as_str()
        .expect("anchor execution binding")
        .to_owned();
    let generation = canonical["generation"].as_u64().expect("anchor generation");
    let committed = canonical["committed_checkpoint_sha256"]
        .as_str()
        .map(ToOwned::to_owned);
    let pending_transition: Option<Value> = None;
    let recomputed = offline_sha256_serialized(&(
        &lineage,
        &execution_binding,
        generation,
        &committed,
        &pending_transition,
    ));
    assert_eq!(anchor.task_id, TASK_ID);
    assert_eq!(anchor.item_id, item_id);
    assert_eq!(
        anchor.anchor_kind,
        NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
    );
    assert_eq!(anchor.lineage_key_sha256, checkpoint.lineage_key_sha256);
    assert_eq!(lineage, checkpoint.lineage_key_sha256);
    assert_eq!(generation, checkpoint.anchor_generation);
    assert_eq!(
        committed.as_deref(),
        Some(checkpoint.checkpoint_sha256.as_str())
    );
    assert!(canonical["pending_transition"].is_null());
    assert_eq!(
        canonical["record_sha256"].as_str(),
        Some(anchor.record_sha256.as_str())
    );
    assert_eq!(anchor.record_sha256, observed.record_sha256);
    assert_eq!(recomputed, anchor.record_sha256);
    assert_eq!(observed.lineage_key_sha256, checkpoint.lineage_key_sha256);
    assert_eq!(observed.generation, checkpoint.anchor_generation);
    assert_eq!(
        observed.committed_checkpoint_sha256.as_deref(),
        Some(checkpoint.checkpoint_sha256.as_str())
    );
    assert!(!observed.pending_transition);
}

fn assert_offline_terminal_store(
    store: &TaskStore,
    item_id: &str,
    expected_bytes: u64,
    mode: &str,
) -> Vec<insar_store::StoredTaskEvent> {
    let terminal = store
        .load_task_record(TASK_ID)
        .expect("terminal task load")
        .expect("terminal task");
    assert_eq!(terminal.runtime.task_id, TASK_ID);
    assert_eq!(terminal.runtime.status, TaskLifecycleStatus::Completed);
    assert!(!terminal.runtime.task_paused);
    assert!(terminal.runtime.active_item_ids.is_empty());
    assert!(terminal.runtime.paused_item_ids.is_empty());
    assert!(terminal.runtime.queued_item_ids.is_empty());
    assert!(terminal.runtime.failed_item_ids.is_empty());
    assert!(terminal.runtime.skipped_item_ids.is_empty());
    assert!(terminal.runtime.cancelled_item_ids.is_empty());
    assert_eq!(
        terminal.runtime.completed_item_ids,
        vec![item_id.to_owned()]
    );

    let download = store
        .load_download_snapshot(TASK_ID)
        .expect("terminal download load");
    assert_eq!(download.task_id, TASK_ID);
    assert_eq!(download.results.len(), 1);
    let result = &download.results[0];
    assert_eq!(result.task_id, TASK_ID);
    assert_eq!(result.item_id, item_id);
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(result.bytes_written, expected_bytes);
    assert!(result.failure.is_none());

    let events = store.list_task_events(TASK_ID).expect("task events");
    assert!(!events.is_empty());
    for (sequence, event) in events.iter().enumerate() {
        assert_eq!(event.task_id, TASK_ID);
        assert_eq!(event.sequence, sequence as u64);
        assert!(event.error.is_none());
    }
    assert_eq!(terminal.runtime.revision, events.last().unwrap().sequence);
    let completed = events
        .iter()
        .rposition(|event| event.event_type == "mark_completed")
        .expect("mark_completed event");
    assert_eq!(completed, events.len() - 1);
    if mode == "body_interruption" {
        let failed = events
            .iter()
            .position(|event| event.event_type == "mark_failed")
            .expect("body mark_failed event");
        let retry = events
            .iter()
            .enumerate()
            .find(|(index, event)| *index > failed && event.event_type == "retry_items")
            .map(|(index, _)| index)
            .expect("body retry_items event");
        let activate = events
            .iter()
            .enumerate()
            .find(|(index, event)| *index > retry && event.event_type == "activate_items")
            .map(|(index, _)| index)
            .expect("body retry activate event");
        assert!(failed < retry && retry < activate && activate < completed);
        for index in [failed, retry, activate, completed] {
            assert_eq!(events[index].item_id.as_deref(), Some(item_id));
        }
    } else {
        assert_eq!(mode, "pause_resume");
        let pause = events
            .iter()
            .rposition(|event| event.event_type == "pause_task")
            .expect("pause_task event");
        let resume = events
            .iter()
            .enumerate()
            .find(|(index, event)| *index > pause && event.event_type == "resume_task")
            .map(|(index, _)| index)
            .expect("resume_task event");
        assert!(pause < resume && resume < completed);
    }
    events
}

fn normalize_cached_planning_records(
    evidence: &mut NisarLiveAcceptanceEvidenceV2,
) -> Result<usize, &'static str> {
    let original = evidence.range_observations.clone();
    let mut unique = Vec::new();
    let mut removed = 0;
    for observation in original {
        if let Some(previous) =
            unique
                .iter()
                .find(|previous: &&insar_download::NisarLiveRangeObservationV2| {
                    previous.stage == observation.stage
                        && previous.start == observation.start
                        && previous.end_exclusive == observation.end_exclusive
                })
        {
            if observation.stage == "science" || previous != &observation {
                return Err("NON_IDENTICAL_OR_SCIENCE_DUPLICATE");
            }
            if !matches!(
                observation.stage.as_str(),
                "preflight"
                    | "metadata_prefix"
                    | "metadata_discovery"
                    | "additional_geolocation"
                    | "planning_revalidation"
            ) {
                return Err("UNKNOWN_STAGE");
            }
            removed += 1;
        } else {
            unique.push(observation);
        }
    }
    evidence.range_observations = unique;
    Ok(removed)
}

#[test]
fn offline_normalization_rejects_changed_or_repeated_science_observations() {
    let mut evidence = passing_m2d_b_evidence();
    let science = evidence
        .range_observations
        .iter()
        .find(|entry| entry.stage == "science")
        .unwrap()
        .clone();
    evidence.range_observations.push(science);
    assert!(normalize_cached_planning_records(&mut evidence).is_err());
}

#[test]
#[ignore = "Requires explicitly supplied preserved live evidence and output; reads no credentials and sends no requests"]
fn nisar_completed_recovery_offline_receipt() {
    let original_root =
        PathBuf::from(std::env::var("INSAR_NISAR_RECOVERY_OFFLINE_STORE_ROOT").unwrap());
    let evidence_path =
        PathBuf::from(std::env::var("INSAR_NISAR_RECOVERY_OFFLINE_EVIDENCE").unwrap());
    let trace_path = PathBuf::from(std::env::var("INSAR_NISAR_RECOVERY_OFFLINE_TRACE").unwrap());
    let receipt_path =
        PathBuf::from(std::env::var("INSAR_NISAR_RECOVERY_OFFLINE_RECEIPT").unwrap());
    assert!(
        !receipt_path.exists(),
        "Receipts must not overwrite existing evidence"
    );
    let original_bytes = fs::read(&evidence_path).unwrap();
    let trace_bytes = fs::read(&trace_path).unwrap();
    let mut evidence: NisarLiveAcceptanceEvidenceV2 =
        serde_json::from_slice(&original_bytes).unwrap();
    let trace: LiveRecoveryTrace = serde_json::from_slice(&trace_bytes).unwrap();
    assert_eq!(evidence.status, "fail");
    assert!(matches!(
        evidence.typed_stop_code.as_str(),
        "NISAR_LIVE_RANGE_POSTCHECK_FAILED" | "NISAR_LIVE_EVIDENCE_INCOMPLETE"
    ));
    let original_stop = evidence.typed_stop_code.clone();
    let output = original_root.join("output");
    let database = original_root.join("tasks.sqlite3");
    let checkpoint = read_live_recovery_checkpoint(&output).unwrap().unwrap();
    let canonical_checkpoint = load_and_verify_offline_checkpoint(&output, &checkpoint);
    let anchor = read_live_recovery_anchor(&database, TASK_ID, &checkpoint.item_id)
        .unwrap()
        .unwrap();
    assert!(live_recovery_anchor_matches_checkpoint(
        &anchor,
        &checkpoint
    ));
    assert_offline_anchor_digest(
        &database,
        &checkpoint.item_id,
        &canonical_checkpoint,
        &anchor,
    );
    assert_eq!(checkpoint.state, "Complete");
    assert!(!checkpoint.pending_attempt);
    assert_eq!(checkpoint.completed_ranges, checkpoint.planned_ranges);
    assert_eq!(
        checkpoint.completed_ranges.len(),
        checkpoint.planned_range_count
    );
    assert_eq!(checkpoint.indeterminate_science_request_count, 0);
    assert_eq!(checkpoint.indeterminate_science_response_body_bytes, 0);
    assert_eq!(
        checkpoint.prepared_intent_sha256,
        evidence.prepared_intent_sha256
    );
    assert_eq!(checkpoint.final_plan_sha256, evidence.final_plan_sha256);
    assert_eq!(
        checkpoint.physical_plan_sha256,
        evidence.physical_plan_sha256
    );
    assert_eq!(
        checkpoint.source_validator_sha256,
        evidence.source_validator_sha256
    );
    assert_eq!(
        checkpoint.committed_science_response_body_bytes,
        evidence.science_response_bytes
    );
    assert_eq!(
        checkpoint.committed_science_response_body_bytes,
        checkpoint.planned_science_response_body_bytes
    );
    assert!(matches!(
        trace.mode.as_str(),
        "body_interruption" | "pause_resume"
    ));
    if trace.mode == "body_interruption" {
        assert!(trace.failure.is_some());
        assert!(trace.pause.is_none());
    } else {
        assert!(trace.pause.is_some());
        assert!(trace.failure.is_none());
    }
    let previous = trace.failure.as_ref().or(trace.pause.as_ref()).unwrap();
    let completion_trace = trace.completion.as_ref().expect("completion trace");
    assert_eq!(
        completion_trace.checkpoint_sha256,
        checkpoint.checkpoint_sha256
    );
    assert_eq!(previous.state, "Interrupted");
    assert!(!previous.pending_attempt);
    assert!(trace.worker_quiescent);
    assert!(
        previous.committed_science_response_body_bytes
            >= checkpoint.planned_science_response_body_bytes / 10
    );
    assert_eq!(
        previous.prepared_intent_sha256,
        checkpoint.prepared_intent_sha256
    );
    assert_eq!(previous.final_plan_sha256, checkpoint.final_plan_sha256);
    assert_eq!(
        previous.physical_plan_sha256,
        checkpoint.physical_plan_sha256
    );
    assert_eq!(
        previous.source_validator_sha256,
        checkpoint.source_validator_sha256
    );
    assert_eq!(
        previous.part_identity_sha256,
        checkpoint.part_identity_sha256
    );
    assert_eq!(previous.lineage_key_sha256, checkpoint.lineage_key_sha256);
    assert_eq!(
        checkpoint.resumed_bytes,
        previous.committed_science_response_body_bytes
    );
    for (range, count) in previous
        .completed_ranges
        .iter()
        .zip(&previous.completed_range_request_counts)
    {
        let position = checkpoint
            .completed_ranges
            .iter()
            .position(|current| current == range)
            .unwrap();
        assert_eq!(*count, checkpoint.completed_range_request_counts[position]);
    }
    let store = TaskStore::open(&database).unwrap();
    let events = assert_offline_terminal_store(
        &store,
        &checkpoint.item_id,
        evidence.output_size_bytes,
        trace.mode.as_str(),
    );
    let terminal = store.load_task_record(TASK_ID).unwrap().unwrap();
    let download = store.load_download_snapshot(TASK_ID).unwrap();
    drop(store);
    let reopened = TaskStore::open(&database).unwrap();
    assert_eq!(
        reopened.load_task_record(TASK_ID).unwrap().unwrap(),
        terminal
    );
    assert_eq!(reopened.load_download_snapshot(TASK_ID).unwrap(), download);
    assert_eq!(reopened.list_task_events(TASK_ID).unwrap(), events);
    drop(reopened);
    let artifacts = fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("h5"))
        .collect::<Vec<_>>();
    assert_eq!(artifacts.len(), 1);
    let artifact = &artifacts[0];
    assert_eq!(
        fs::metadata(artifact).unwrap().len(),
        evidence.output_size_bytes
    );
    let mut reader = fs::File::open(artifact).unwrap();
    let mut hash = sha2::Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    let artifact_hash = format!("{:x}", hash.finalize());
    assert_eq!(artifact_hash, evidence.output_sha256);
    let manifest_path = artifact.with_file_name(format!(
        "{}.artifact-manifest-v2.json",
        artifact.file_name().unwrap().to_str().unwrap()
    ));
    let manifest_bytes = fs::read(manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["artifact_sha256"], artifact_hash);
    assert_eq!(
        manifest["artifact_size_bytes"].as_u64(),
        Some(evidence.output_size_bytes)
    );
    assert_eq!(manifest["task_id"].as_str(), Some(TASK_ID));
    assert_eq!(
        manifest["item_id"].as_str(),
        Some(checkpoint.item_id.as_str())
    );
    assert_eq!(
        manifest["artifact_file_name"].as_str(),
        artifact.file_name().and_then(|name| name.to_str())
    );
    assert_eq!(
        manifest["science_checkpoint_sha256"],
        checkpoint.checkpoint_sha256
    );
    assert_eq!(
        manifest["completed_range_evidence_sha256"].as_str(),
        Some(offline_sha256_serialized(&canonical_checkpoint.completed_ranges).as_str())
    );
    assert_eq!(
        manifest["prepared_intent_sha256"],
        checkpoint.prepared_intent_sha256
    );
    assert_eq!(
        manifest["final_semantic_plan_sha256"],
        checkpoint.final_plan_sha256
    );
    assert_eq!(
        manifest["physical_plan_sha256"],
        checkpoint.physical_plan_sha256
    );
    assert_eq!(
        manifest["source_product_id"],
        canonical_checkpoint
            .source_product_identity
            .provider_product_id
    );
    assert_eq!(
        manifest["source_asset_key"],
        canonical_checkpoint.source_asset_identity.stable_asset_key
    );
    assert_eq!(
        manifest["source_validator_sha256"],
        checkpoint.source_validator_sha256
    );
    assert_eq!(
        manifest["source_size"].as_u64(),
        Some(canonical_checkpoint.source_size)
    );
    assert_eq!(manifest["inventory_sha256"], checkpoint.inventory_sha256);
    assert_eq!(
        manifest["inventory_evidence_sha256"],
        checkpoint.inventory_evidence_sha256
    );
    assert_eq!(
        manifest["dependency_graph_sha256"],
        checkpoint.source_graph_sha256
    );
    validate_output_science_selection(
        &output,
        NisarSubsetFrequency::A,
        &["HH".into(), "HV".into()],
    )
    .unwrap();
    let removed = normalize_cached_planning_records(&mut evidence).unwrap();
    assert_offline_science_range_bindings(&evidence, &canonical_checkpoint);
    let mut retry_request_delta = None;
    let mut retry_committed_bytes = None;
    let mut failed_range_for_receipt = Value::Null;
    if trace.mode == "body_interruption" {
        assert_eq!(removed, 0);
        assert_eq!(
            trace.body_interruption_failed_request_count,
            Some(checkpoint.failed_science_request_count)
        );
        assert!(checkpoint.failed_science_request_count > 0);
        assert_eq!(
            previous.failed_science_request_count,
            checkpoint.failed_science_request_count
        );
        assert!(trace.failure_runtime_revision.is_some());
        let partial_bytes = previous
            .observed_science_response_body_bytes
            .checked_sub(previous.committed_science_response_body_bytes)
            .expect("body failure observed delta");
        assert_eq!(trace.body_interruption_observed_bytes, Some(partial_bytes));
        assert!(partial_bytes > 0);
        assert!(trace.body_interruption_range.is_some());
        let failed_range = trace.body_interruption_range.as_ref().unwrap();
        let planned_position = canonical_checkpoint
            .planned_ranges
            .iter()
            .enumerate()
            .find(|(_, range)| {
                range.start == failed_range.start
                    && range.end_exclusive == failed_range.end_exclusive
            })
            .map(|(index, _)| index)
            .expect("body failure range planned");
        assert_eq!(failed_range.index, planned_position as u64);
        let failed_range_bytes = failed_range
            .end_exclusive
            .checked_sub(failed_range.start)
            .expect("body failure range length");
        assert_eq!(
            failed_range_bytes,
            previous
                .reserved_science_response_body_bytes
                .checked_sub(previous.committed_science_response_body_bytes)
                .expect("body failure reservation delta")
        );
        assert!(partial_bytes < failed_range_bytes);
        assert_eq!(
            trace.body_interruption_result_code.as_deref(),
            Some("NISAR_SUBSET_V2_SCIENCE_INCOMPLETE")
        );
        assert!(!previous.completed_ranges.iter().any(|range| {
            range.start == failed_range.start && range.end_exclusive == failed_range.end_exclusive
        }));
        let final_failed_range = canonical_checkpoint
            .completed_ranges
            .iter()
            .find(|range| {
                range.start == failed_range.start
                    && range.end_exclusive == failed_range.end_exclusive
            })
            .expect("body failure range retried");
        assert!(final_failed_range.request_count > 0);

        let committed_delta = checkpoint
            .committed_science_response_body_bytes
            .checked_sub(previous.committed_science_response_body_bytes)
            .expect("body retry committed bytes");
        let committed_request_delta = checkpoint
            .committed_science_request_count
            .checked_sub(previous.committed_science_request_count)
            .expect("body retry committed requests");
        let issued_request_delta = checkpoint
            .issued_science_request_count
            .checked_sub(previous.issued_science_request_count)
            .expect("body retry issued requests");
        let reserved_request_delta = checkpoint
            .reserved_science_request_count
            .checked_sub(previous.reserved_science_request_count)
            .expect("body retry reserved requests");
        let mut remaining_range_bytes = 0_u64;
        let mut remaining_range_requests = 0_u64;
        for completed in &canonical_checkpoint.completed_ranges {
            if let Some(position) = previous.completed_ranges.iter().position(|range| {
                range.start == completed.start && range.end_exclusive == completed.end_exclusive
            }) {
                assert_eq!(
                    previous.completed_range_request_counts[position],
                    completed.request_count
                );
            } else {
                remaining_range_bytes = remaining_range_bytes
                    .checked_add(
                        completed
                            .end_exclusive
                            .checked_sub(completed.start)
                            .expect("positive retry range"),
                    )
                    .expect("retry range bytes");
                remaining_range_requests = remaining_range_requests
                    .checked_add(completed.request_count)
                    .expect("retry range requests");
            }
        }
        assert_eq!(remaining_range_bytes, committed_delta);
        assert_eq!(remaining_range_requests, committed_request_delta);
        assert_eq!(committed_request_delta, issued_request_delta);
        assert_eq!(committed_request_delta, reserved_request_delta);
        assert_eq!(
            trace.body_interruption_retry_request_count,
            Some(committed_request_delta)
        );
        assert_eq!(
            checkpoint.observed_science_response_body_bytes,
            previous
                .observed_science_response_body_bytes
                .checked_add(committed_delta)
                .expect("body cumulative observed bytes")
        );
        assert_eq!(
            checkpoint
                .observed_science_response_body_bytes
                .checked_sub(checkpoint.committed_science_response_body_bytes)
                .expect("body final observed delta"),
            partial_bytes
        );
        assert_eq!(
            checkpoint.failed_science_request_count,
            previous.failed_science_request_count
        );
        assert_eq!(checkpoint.indeterminate_science_request_count, 0);
        assert_eq!(checkpoint.indeterminate_science_response_body_bytes, 0);
        assert_eq!(
            checkpoint.resumed_bytes,
            previous.committed_science_response_body_bytes
        );
        evidence.failed_science_requests = checkpoint.failed_science_request_count;
        evidence.failed_science_response_bytes = partial_bytes;
        evidence.total_requests = evidence
            .total_requests
            .checked_add(evidence.failed_science_requests)
            .expect("corrected total requests");
        evidence.total_response_bytes = evidence
            .total_response_bytes
            .checked_add(partial_bytes)
            .expect("corrected total response bytes");
        evidence.remaining_requests = evidence
            .remaining_requests
            .checked_sub(evidence.failed_science_requests)
            .expect("corrected remaining requests");
        evidence.remaining_response_bytes = evidence
            .remaining_response_bytes
            .checked_sub(partial_bytes)
            .expect("corrected remaining response bytes");
        evidence.source_byte_ratio =
            evidence.total_response_bytes as f64 / evidence.source_size_bytes as f64;
        retry_request_delta = Some(committed_request_delta);
        retry_committed_bytes = Some(committed_delta);
        failed_range_for_receipt = serde_json::json!({
            "index": failed_range.index,
            "start": failed_range.start,
            "end_exclusive": failed_range.end_exclusive,
            "observed_response_body_bytes": partial_bytes,
            "request_count": final_failed_range.request_count,
        });
    } else {
        assert!(removed > 0);
        assert_eq!(checkpoint.failed_science_request_count, 0);
        assert!(trace.paused_runtime_revision.unwrap() < trace.resume_runtime_revision.unwrap());
        assert_eq!(
            checkpoint.resumed_bytes,
            previous.committed_science_response_body_bytes
        );
    }
    // These corrections exist only in this separate audit result. The original
    // run remains failed, and reopening the Store was independently observed above.
    evidence.store_reopen = "observed".into();
    evidence.status = "pass".into();
    evidence.typed_stop_code = "PASS".into();
    validate_m2d_b_final_evidence_for_selection(
        &evidence,
        &evidence.aoi_digest,
        &output,
        "A",
        "HH+HV",
        &["HH".into(), "HV".into()],
        &["HH".into(), "HV".into()],
        Some((
            checkpoint.failed_science_request_count,
            checkpoint
                .reserved_science_response_body_bytes
                .checked_sub(checkpoint.committed_science_response_body_bytes)
                .unwrap(),
        )),
    )
    .unwrap();
    assert_eq!(fs::read(&evidence_path).unwrap(), original_bytes);
    assert_eq!(fs::read(&trace_path).unwrap(), trace_bytes);
    let receipt = serde_json::json!({
        "schema": "nisar_recovery_offline_audit_v1", "audit_status": "pass", "original_run_status": "fail",
        "original_stop_code": original_stop, "original_evidence_sha256": sha256_bytes(&original_bytes),
        "original_trace_sha256": sha256_bytes(&trace_bytes), "manifest_sha256": sha256_bytes(&manifest_bytes),
        "removed_identical_cached_planning_records": removed, "network_requests_during_audit": 0,
        "credential_reads_during_audit": 0, "store_reopened_completed": true,
        "artifact_sha256": artifact_hash, "artifact_size_bytes": evidence.output_size_bytes,
        "native_hdf5_selected_science_reopened": true, "checkpoint_anchor_verified": true,
        "reused_verified_bytes": checkpoint.resumed_bytes, "failed_science_requests": checkpoint.failed_science_request_count,
        "failed_science_response_bytes": evidence.failed_science_response_bytes,
        "failed_science_range": failed_range_for_receipt,
        "retry_science_request_delta": retry_request_delta,
        "retry_science_committed_response_body_bytes": retry_committed_bytes,
        "final_observed_science_response_body_bytes": checkpoint.observed_science_response_body_bytes,
        "corrected_total_requests": evidence.total_requests, "corrected_total_response_bytes": evidence.total_response_bytes,
        "raw_evidence_preserved": true, "scientific_checks": "original worker validation bound by unchanged artifact hash; native selection rechecked"
    });
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt_path)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&receipt).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    println!("OFFLINE_RECOVERY_RECEIPT={}", receipt_path.display());
}
