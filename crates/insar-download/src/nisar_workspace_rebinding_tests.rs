//! Identity-verified workspace rebinding (design 2026-10-17 §5) at the
//! planning boundary. Windows only: the rename leaves a directory junction at
//! the old path, as the 2026-10-16 workspace rename did. All fixtures live in
//! the test's own temporary root.
use std::process::{Command, Stdio};

use super::*;

/// Renames a directory and leaves a junction at its old path; restores both on
/// drop. Every handle below the directory must be released first: Windows
/// refuses to rename a directory with open descendants.
pub(super) struct RenamedBehindJunction {
    pub(super) old: PathBuf,
    pub(super) new: PathBuf,
    junction: bool,
}

impl RenamedBehindJunction {
    pub(super) fn new(old: &Path, suffix: &str) -> Self {
        let mut name = old.file_name().unwrap().to_os_string();
        name.push(suffix);
        let new = old.with_file_name(name);
        fs::rename(old, &new).expect("rename with every descendant handle released");
        let status = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(old)
            .arg(&new)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
        Self {
            old: old.to_path_buf(),
            new,
            junction: true,
        }
    }

    /// Removes only the junction, never its target.
    pub(super) fn remove_junction(&mut self) {
        if self.junction {
            fs::remove_dir(&self.old).unwrap();
            self.junction = false;
        }
    }
}

impl Drop for RenamedBehindJunction {
    fn drop(&mut self) {
        if self.junction {
            let _ = fs::remove_dir(&self.old);
        }
        let _ = fs::rename(&self.new, &self.old);
    }
}

fn local_bootstrap(case: &ReadyCase, root: &Path) -> NisarSubsetLocalHdf5MetadataBootstrapV2 {
    NisarSubsetLocalHdf5MetadataBootstrapV2::new_for_test(
        case.source_path.clone(),
        fs::metadata(&case.source_path).unwrap().len(),
        Hdf5RangeWorkspaceV1::new(root.to_path_buf()).unwrap(),
    )
    .unwrap()
}

fn plan_new(
    case: &ReadyCase,
    root: &Path,
    work: &DownloadWork,
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    plan_and_freeze_nisar_subset_v2_with(
        work,
        &WorkerControl::default(),
        &local_bootstrap(case, root),
        |ready, _| Ok(ready.proposed_plan),
    )
}

fn reopen(
    case: &ReadyCase,
    root: &Path,
    work: &DownloadWork,
) -> Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2> {
    plan_and_freeze_nisar_subset_v2_with(
        work,
        &WorkerControl::default(),
        &local_bootstrap(case, root),
        |_, _| panic!("a frozen plan is reopened, never frozen again"),
    )
}

fn export(root: &Path, work: &DownloadWork) -> Result<(), NisarSubsetPlanningErrorV2> {
    export_archived_planning_v2(
        work,
        &Hdf5RangeWorkspaceV1::open_existing(root.to_path_buf()).unwrap(),
        &WorkerControl::default(),
    )
    .map(|_| ())
}

fn planning_checkpoint_text(root: &Path) -> String {
    Hdf5RangeWorkspaceV1::open_existing(root.to_path_buf())
        .unwrap()
        .load_planning_checkpoint_v2()
        .unwrap()
        .unwrap()
}

fn planned_work(
    case: &ReadyCase,
    intent: &NisarSubsetPreparedIntentV2,
    root: &Path,
) -> DownloadWork {
    let mut work = work(case, intent);
    let completed = plan_new(case, root, &work).unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    work
}

fn copy_directory(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

/// Rewrites the planning checkpoint with another recorded identity and a
/// matching digest, so only the identity rule can refuse it.
fn reseal_identity(root: &Path, original: &str, identity: &str) {
    let mut checkpoint = parse_planning_workspace_checkpoint_v2(original).unwrap();
    checkpoint.workspace_identity = identity.to_owned();
    checkpoint.checkpoint_sha256 = checkpoint.recompute_sha256().unwrap();
    write_planning_checkpoint(root, &serde_json::to_string(&checkpoint).unwrap());
}

fn write_planning_checkpoint(root: &Path, text: &str) {
    let path = Hdf5RangeWorkspaceV1::open_existing(root.to_path_buf())
        .unwrap()
        .planning_checkpoint_path_v2();
    fs::write(path, text).unwrap();
}

/// A planning workspace reached after its ancestor was renamed reopens and
/// exports with its recorded path: through a junction at the old path, and
/// through the new path after the junction is removed. Nothing recorded is
/// rewritten.
#[cfg(windows)]
#[test]
fn planning_workspace_reopens_and_exports_after_an_ancestor_rename() {
    let (case, intent, _fixture_bootstrap, _source) =
        concrete_fixture("rebind-planning", NisarSubsetFrequency::A, &["HH"]);
    let parent = case._root.0.join("rebind-old");
    let root = parent.join("ws");
    let work = planned_work(&case, &intent, &root);
    let recorded = planning_checkpoint_text(&root);
    let mut renamed = RenamedBehindJunction::new(&parent, "-moved");
    let reopened = reopen(&case, &root, &work).expect("through the junction at the old path");
    export(&root, &work).unwrap();
    assert_eq!(planning_checkpoint_text(&root), recorded);
    renamed.remove_junction();
    let moved = renamed.new.join("ws");
    let again = reopen(&case, &moved, &work).expect("through the new path");
    export(&moved, &work).unwrap();
    assert_eq!(
        again.physical_execution_plan(),
        reopened.physical_execution_plan()
    );
    assert_eq!(planning_checkpoint_text(&moved), recorded);
}

/// The crash window between the planning checkpoint and the Store freeze: the
/// first run stopped before the plan was frozen; after the rename, the
/// prepared plan is recovered, frozen and reopened with the recorded path.
#[cfg(windows)]
#[test]
fn planning_written_before_the_store_freeze_recovers_after_an_ancestor_rename() {
    let (case, intent, _fixture_bootstrap, _source) =
        concrete_fixture("rebind-crash-window", NisarSubsetFrequency::A, &["HH"]);
    let parent = case._root.0.join("crash-old");
    let root = parent.join("ws");
    let work = work(&case, &intent);
    assert!(plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        &local_bootstrap(&case, &root),
        |_, _| Err(NisarSubsetPlanningErrorV2::ExecutionStopped),
    )
    .is_err());
    let recorded = planning_checkpoint_text(&root);
    let _renamed = RenamedBehindJunction::new(&parent, "-moved");
    plan_new(&case, &root, &work).expect("the prepared plan is recovered and frozen");
    assert_eq!(planning_checkpoint_text(&root), recorded);
}

/// Rebinding never bypasses identity: a same-volume copy, a renamed last
/// directory and a changed or empty recorded identity are still refused.
#[cfg(windows)]
#[test]
fn planning_workspace_rebinding_refuses_copies_renamed_leaves_and_changed_identities() {
    let (case, intent, _fixture_bootstrap, _source) =
        concrete_fixture("rebind-refusals", NisarSubsetFrequency::A, &["HH"]);
    let parent = case._root.0.join("refusal-old");
    let root = parent.join("ws");
    let work = planned_work(&case, &intent, &root);
    let mismatch = |result: Result<NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2>| {
        matches!(
            result,
            Err(NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch)
        )
    };
    let copy = case._root.0.join("refusal-copy").join("ws");
    copy_directory(&root, &copy);
    assert!(mismatch(reopen(&case, &copy, &work)), "same-volume copy");
    let renamed_leaf = parent.join("ws-renamed");
    fs::rename(&root, &renamed_leaf).unwrap();
    assert!(
        mismatch(reopen(&case, &renamed_leaf, &work)),
        "renamed last directory"
    );
    fs::rename(&renamed_leaf, &root).unwrap();
    let original = planning_checkpoint_text(&root);
    let _renamed = RenamedBehindJunction::new(&parent, "-moved");
    for identity in ["windows:1:2", ""] {
        reseal_identity(&root, &original, identity);
        assert!(mismatch(reopen(&case, &root, &work)), "{identity:?}");
    }
    write_planning_checkpoint(&root, &original);
    reopen(&case, &root, &work).expect("the unmodified record rebinds");
}

/// Review 2026-10-17 round 3, P2-5: the identity is refused by the rebinding
/// rule itself, not only by the later planning identity comparison (which
/// would also reject a rebound workspace whose recorded identity differs, so
/// the planning-level refusal above cannot prove the rule). With the ancestor
/// renamed behind a junction and every other condition of the rule met, a
/// recorded identity that differs in its volume serial or file ID, or is
/// malformed, is refused by the rule, the binding keeps the current path and
/// no rebinding evidence is written; the unmodified record rebinds.
#[cfg(windows)]
#[test]
fn rebinding_rule_itself_refuses_a_changed_or_malformed_recorded_identity() {
    use crate::nisar_workspace_rebinding_v2::{
        bind_nisar_workspace_v2, nisar_workspace_rebinding_permitted_v2,
        record_nisar_workspace_rebinding_v2, NisarWorkspaceRebindingRefusalV2 as Refusal,
    };
    let (case, intent, _fixture_bootstrap, _source) =
        concrete_fixture("rebind-rule-identity", NisarSubsetFrequency::A, &["HH"]);
    let parent = case._root.0.join("rule-old");
    let root = parent.join("ws");
    let work = planned_work(&case, &intent, &root);
    let original = planning_checkpoint_text(&root);
    let recorded: Value = serde_json::from_str(&original).unwrap();
    let recorded_path = recorded["workspace_canonical_path"]
        .as_str()
        .unwrap()
        .to_owned();
    let identity = recorded["workspace_identity"].as_str().unwrap().to_owned();
    let (serial, file_id) = identity
        .strip_prefix("windows:")
        .and_then(|rest| rest.split_once(':'))
        .unwrap();
    let other_serial = format!("windows:{}:{file_id}", serial.parse::<u64>().unwrap() ^ 1);
    let other_file_id = format!("windows:{serial}:{}", file_id.parse::<u64>().unwrap() ^ 1);
    let _renamed = RenamedBehindJunction::new(&parent, "-moved");
    for (tampered, refusal) in [
        (other_serial.as_str(), Refusal::IdentityChanged),
        (other_file_id.as_str(), Refusal::IdentityChanged),
        ("windows:1:0", Refusal::IdentityMalformed),
        ("", Refusal::IdentityMalformed),
    ] {
        reseal_identity(&root, &original, tampered);
        let workspace = Hdf5RangeWorkspaceV1::open_existing(root.clone()).unwrap();
        let (current_path, current_identity) = workspace.planning_workspace_identity_v2().unwrap();
        assert_ne!(current_path, recorded_path, "the ancestor rename moved it");
        assert_eq!(
            current_identity, identity,
            "the directory itself is unchanged"
        );
        let volume = workspace.root_volume_v2().unwrap();
        // Every other condition holds: the recorded identity alone decides.
        assert_eq!(
            nisar_workspace_rebinding_permitted_v2(
                true,
                &recorded_path,
                &identity,
                &current_path,
                &current_identity,
                Some(&volume),
            ),
            Ok(())
        );
        assert_eq!(
            nisar_workspace_rebinding_permitted_v2(
                true,
                &recorded_path,
                tampered,
                &current_path,
                &current_identity,
                Some(&volume),
            ),
            Err(refusal),
            "{tampered:?}"
        );
        let bound = bind_nisar_workspace_v2(&workspace).unwrap();
        assert!(!bound.is_rebound(), "{tampered:?}");
        assert_eq!(bound.path(), current_path);
        assert_eq!(
            record_nisar_workspace_rebinding_v2(&workspace, &work, None).unwrap(),
            None
        );
        assert_eq!(workspace.load_rebinding_evidence_v2().unwrap(), None);
    }
    write_planning_checkpoint(&root, &original);
    let workspace = Hdf5RangeWorkspaceV1::open_existing(root.clone()).unwrap();
    let bound = bind_nisar_workspace_v2(&workspace).unwrap();
    assert!(bound.is_rebound());
    assert_eq!(bound.path(), recorded_path);
}

/// Archived recovery from a source workspace whose ancestor was renamed: every
/// source check keeps the recorded path and the import proceeds; the source
/// itself (same directory identity) is still refused as a destination.
#[cfg(windows)]
#[test]
fn archived_recovery_imports_from_a_source_workspace_behind_a_renamed_ancestor() {
    use crate::nisar_science_transfer_v2::{
        prepare_archived_nisar_recovery_v2, NisarSubsetScienceRangeExecutorV2,
    };
    let (case, intent, _fixture_bootstrap, source_path) =
        concrete_fixture("rebind-archived", NisarSubsetFrequency::A, &["HH"]);
    let parent = case._root.0.join("archived-old");
    let root = parent.join("ws");
    let source_work = planned_work(&case, &intent, &root);
    let (anchor_json, record_sha256) = {
        let bootstrap = local_bootstrap(&case, &root);
        let completed = plan_and_freeze_nisar_subset_v2_with(
            &source_work,
            &WorkerControl::default(),
            &bootstrap,
            |_, _| panic!("a frozen plan is reopened, never frozen again"),
        )
        .unwrap();
        let physical = completed.physical_execution_plan();
        let ranges = physical.execution_transfer_ranges().unwrap();
        assert!(ranges.len() > 1);
        let server = ScienceServer::new(vec![
            exact_science_response(
                &ranges[0],
                physical.source_size(),
                &physical.source_validator().value,
                &source_range_bytes(&source_path, &ranges[0]),
            ),
            science_response(
                "401 Unauthorized",
                &[("Content-Length", "0".to_owned())],
                &[],
            ),
        ]);
        let asset = science_asset(
            &server.url_root,
            physical,
            Some(&physical.source_validator().value),
            None,
            None,
        );
        let anchor = ScienceCheckpointAnchorFixture::default();
        let authority = science_authority(&bootstrap, &asset, &source_work, &completed, &anchor);
        assert!(
            NisarSubsetScienceRangeExecutorV2::new(science_http(), asset)
                .unwrap()
                .execute(
                    &source_work,
                    &completed,
                    &authority,
                    &WorkerControl::default()
                )
                .is_err()
        );
        server.finish();
        let record = anchor.state.lock().unwrap().as_ref().unwrap().1.clone();
        (
            record.canonical_json().unwrap(),
            record.record_sha256().to_owned(),
        )
    };
    let _renamed = RenamedBehindJunction::new(&parent, "-moved");
    let source_workspace = Hdf5RangeWorkspaceV1::open_existing(root.clone()).unwrap();
    let mut new_work = source_work.clone();
    new_work.task.task_id = "explicit-archived-recovery".to_owned();
    new_work.task.output_root = case._root.0.join("recovered-output");
    let destination =
        Hdf5RangeWorkspaceV1::new(case._root.0.join("rebind-recovery-workspace")).unwrap();
    let token = prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        &record_sha256,
        &source_workspace,
        &destination,
        &WorkerControl::default(),
    )
    .expect("the renamed source keeps its recorded path");
    drop(token);
    assert!(prepare_archived_nisar_recovery_v2(
        &source_work,
        &new_work,
        &anchor_json,
        &record_sha256,
        &source_workspace,
        &source_workspace,
        &WorkerControl::default(),
    )
    .is_err());
}
