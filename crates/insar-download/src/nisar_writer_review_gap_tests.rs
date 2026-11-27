//! Writer-path test gaps of the fourth review (2026-10-17), closed in Issue #8
//! slice 6: coherent-accounting refusals of the writer binding, a Reserved
//! attempt with a multi-request reservation whose pending anchor transition
//! is still unreconciled when the writer runs, and the recovery replay of a
//! manifest that records an indeterminate science charge.
use super::*;
use crate::nisar_science_transfer_v2::{
    NisarSubsetScienceRangeExecutorV2, NisarSubsetScienceTransferErrorV2 as ScienceError,
};
use crate::nisar_subset_writer::{
    validate_writer_binding_for_test, NisarSubsetWriteError as WriteError,
    WriterBindingTamperForTest as Tamper, WriterFailpointV2,
};

struct PlannedScience {
    case: ReadyCase,
    work: DownloadWork,
    completed: NisarSubsetPlanningCompleteV2,
    bootstrap: Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    url_root: String,
    server: ScienceServer,
}

fn planned_science(label: &str) -> PlannedScience {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let server = ScienceServer::new(exact_science_responses(
        &source_path,
        completed.physical_execution_plan(),
        0,
    ));
    let url_root = server.url_root.clone();
    PlannedScience {
        case,
        work,
        completed,
        bootstrap,
        url_root,
        server,
    }
}

impl PlannedScience {
    fn asset(&self) -> ResolvedAsset {
        same_source_asset(&self.url_root, &self.completed)
    }
}

#[test]
fn writer_binding_refuses_each_incoherent_accounting_fact_after_resealing() {
    let planned = planned_science("writer-binding-negatives");
    let anchor = ScienceCheckpointAnchorFixture::default();
    let asset = planned.asset();
    let authority = science_authority(
        planned.bootstrap.as_ref(),
        &asset,
        &planned.work,
        &planned.completed,
        &anchor,
    );
    let science = NisarSubsetScienceRangeExecutorV2::new(science_http(), planned.asset())
        .unwrap()
        .execute(
            &planned.work,
            &planned.completed,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    assert!(science.complete());
    assert_eq!(
        validate_writer_binding_for_test(
            &planned.work,
            &planned.completed,
            &science,
            &authority,
            None
        ),
        Ok(())
    );
    for tamper in [
        Tamper::IndeterminateRequestsWithoutBytes,
        Tamper::IndeterminateBytesWithoutRequests,
        Tamper::IssuedIsNotCommittedPlusFailed,
        Tamper::ObservedBelowCommittedBytes,
        Tamper::ReservedBelowObservedBytes,
        Tamper::ZeroCompletedRangeRequestCount,
    ] {
        assert_eq!(
            validate_writer_binding_for_test(
                &planned.work,
                &planned.completed,
                &science,
                &authority,
                Some(tamper)
            ),
            Err(WriteError::WriterBinding),
            "{tamper:?}"
        );
    }
    drop(authority);
    drop(planned.server);
}

/// A crash right after the anchor recorded a Reserved attempt with a
/// multi-request reservation (Reserved(Some(n))) leaves that transition
/// pending. Until the science executor reconciles it, the writer refuses the
/// science authority and writes nothing; after reconciliation the charge is
/// the whole reservation of n requests, recorded as the manifest bounds, and
/// a crash after promotion replays that manifest on recovery unchanged.
#[test]
fn writer_v2_refuses_an_unreconciled_reserved_transition_then_records_and_replays_its_bounds() {
    let planned = planned_science("writer-reserved-pending");
    let anchor = ScienceCheckpointAnchorFixture::default();
    let asset = planned.asset();
    let authority = science_authority(
        planned.bootstrap.as_ref(),
        &asset,
        &planned.work,
        &planned.completed,
        &anchor,
    );
    anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), planned.asset()).unwrap();
    assert_eq!(
        executor.execute(
            &planned.work,
            &planned.completed,
            &authority,
            &WorkerControl::default()
        ),
        Err(ScienceError::Persistence)
    );
    assert!(anchor.has_pending());
    let pending_state = anchor.state.lock().unwrap().clone();
    let reserved_request_count = {
        let (_, record) = pending_state.as_ref().unwrap();
        let pending: Value =
            serde_json::from_str(record.pending_checkpoint_canonical_json().unwrap()).unwrap();
        assert_eq!(pending["pending_attempt"]["state"], json!("Reserved"));
        pending["pending_attempt"]["reserved_request_count"]
            .as_u64()
            .expect("a reservation records its request count: Reserved(Some(n))")
    };
    assert!(reserved_request_count >= 1);
    let science = executor
        .execute(
            &planned.work,
            &planned.completed,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    assert!(science.complete());
    assert_eq!(
        science.accounting().indeterminate_science_request_count(),
        reserved_request_count,
        "the charge is the whole reservation"
    );
    let reconciled_state = anchor.state.lock().unwrap().clone();
    assert!(!anchor.has_pending());

    // The anchor still holds the pending Reserved transition: refused.
    *anchor.state.lock().unwrap() = pending_state;
    let output_root = planned.case._root.0.join("writer-reserved-pending-output");
    let writer = crate::nisar_subset_writer::tests::writer(&output_root);
    assert_eq!(
        writer
            .write_and_validate_v2(
                &planned.work,
                &planned.completed,
                &science,
                &authority,
                &WorkerControl::default(),
            )
            .unwrap_err(),
        WriteError::IncompleteScienceAuthority
    );
    assert!(
        fs::read_dir(&output_root).map_or(true, |mut entries| entries.all(|entry| entry
            .unwrap()
            .path()
            .extension()
            .and_then(|value| value.to_str())
            != Some("h5")))
    );

    // Reconciled: written with the bounds, then replayed after a crash.
    *anchor.state.lock().unwrap() = reconciled_state;
    assert_eq!(
        writer
            .write_and_validate_v2_with_failpoint_for_test(
                &planned.work,
                &planned.completed,
                &science,
                &authority,
                &WorkerControl::default(),
                WriterFailpointV2::AfterArtifactPromotion,
            )
            .unwrap_err(),
        WriteError::InjectedFailure
    );
    let recovered = writer
        .write_and_validate_v2(
            &planned.work,
            &planned.completed,
            &science,
            &authority,
            &WorkerControl::default(),
        )
        .unwrap();
    let manifest = serde_json::to_value(recovered.manifest()).unwrap();
    let charge = &manifest["indeterminate_science_charge"];
    let lower = charge["science_request_count_lower_bound"]
        .as_u64()
        .unwrap();
    assert_eq!(
        charge["indeterminate_science_request_count"],
        json!(reserved_request_count)
    );
    assert_eq!(
        charge["science_request_count_upper_bound"],
        json!(lower + reserved_request_count)
    );
    let sidecar: Value =
        serde_json::from_str(&fs::read_to_string(recovered.manifest_path()).unwrap()).unwrap();
    assert_eq!(sidecar, manifest, "recovery replays the sidecar unchanged");
    drop(authority);
    drop(planned.server);
}
