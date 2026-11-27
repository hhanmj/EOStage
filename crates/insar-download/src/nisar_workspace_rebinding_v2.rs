//! Identity-verified rebinding of a renamed NISAR recovery workspace.
//!
//! A recovery workspace records its canonical path and directory identity in
//! its planning checkpoint; the science checkpoint, the Store anchor binding
//! and budget proofs embed the same path. After an ancestor directory was
//! renamed (2026-10-16: the workspace root moved and the old path stayed as a
//! junction), the canonical path of an old workspace changes while its
//! directory identity (volume serial and file ID) does not. Such a workspace
//! is bound to its recorded path only when the rules of design §3 hold. Every
//! comparison and digest then keeps using the recorded path and the current
//! path serves I/O only; nothing recorded is rewritten, and later checkpoint
//! generations keep writing the recorded path. When the rules do not hold,
//! the current path is used and every existing check fails exactly as before.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::hdf5_range_transfer::{Hdf5RangeTransferError, Hdf5RangeWorkspaceV1};
use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2;
use crate::DownloadWork;

const REBINDING_EVIDENCE_CONTRACT_V2: &str = "nisar_workspace_rebinding_evidence_v2";

/// Why a moved workspace is not bound to its recorded path (design §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarWorkspaceRebindingRefusalV2 {
    UnsupportedPlatform,
    IdentityMalformed,
    IdentityChanged,
    PathFormUnsupported,
    LeafRenamed,
    VolumeNotLocalNtfs,
}

/// Observed facts about the volume that holds the current workspace root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarWorkspaceVolumeV2 {
    pub(crate) file_system: String,
    pub(crate) local: bool,
}

/// Design §3: a workspace reached through another canonical path may keep its
/// recorded path only on Windows, when the recorded identity is well formed
/// (non-zero file ID) and equal to the current one, both paths are verbatim
/// drive paths (`\\?\X:\`, never UNC), the last directory name is unchanged and
/// the current volume is a local NTFS volume. Pure: the caller observes.
pub(crate) fn nisar_workspace_rebinding_permitted_v2(
    windows: bool,
    recorded_path: &str,
    recorded_identity: &str,
    current_path: &str,
    current_identity: &str,
    volume: Option<&NisarWorkspaceVolumeV2>,
) -> Result<(), NisarWorkspaceRebindingRefusalV2> {
    use NisarWorkspaceRebindingRefusalV2 as Refusal;
    if !windows {
        return Err(Refusal::UnsupportedPlatform);
    }
    if !well_formed_windows_identity(recorded_identity) {
        return Err(Refusal::IdentityMalformed);
    }
    if recorded_identity != current_identity {
        return Err(Refusal::IdentityChanged);
    }
    if !verbatim_drive_path(recorded_path) || !verbatim_drive_path(current_path) {
        return Err(Refusal::PathFormUnsupported);
    }
    match (last_component(recorded_path), last_component(current_path)) {
        (Some(recorded), Some(current)) if recorded == current => {}
        _ => return Err(Refusal::LeafRenamed),
    }
    match volume {
        Some(volume) if volume.local && volume.file_system == "NTFS" => Ok(()),
        _ => Err(Refusal::VolumeNotLocalNtfs),
    }
}

fn well_formed_windows_identity(identity: &str) -> bool {
    let mut parts = identity.split(':');
    matches!(
        (
            parts.next(),
            parts.next().map(str::parse::<u64>),
            parts.next().map(str::parse::<u64>),
            parts.next(),
        ),
        (Some("windows"), Some(Ok(_)), Some(Ok(file_id)), None) if file_id != 0
    )
}

/// `\\?\X:\...`: a verbatim drive path, not `\\?\UNC\...` or a device path.
fn verbatim_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() > 7
        && bytes.starts_with(br"\\?\")
        && bytes[4].is_ascii_alphabetic()
        && bytes[5] == b':'
        && bytes[6] == b'\\'
}

/// The last component of a verbatim drive path, compared exactly.
fn last_component(path: &str) -> Option<&str> {
    let component = path.trim_end_matches('\\').rsplit('\\').next()?;
    (!component.is_empty() && !component.ends_with(':')).then_some(component)
}

/// The path and identity a workspace's comparisons and digests use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarBoundWorkspaceV2 {
    path: String,
    identity: String,
    current_path: String,
}

impl NisarBoundWorkspaceV2 {
    /// The recorded path when rebound, otherwise the current canonical path.
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) fn current_path(&self) -> &str {
        &self.current_path
    }

    /// The recorded path differs from the current one and was accepted.
    pub(crate) fn is_rebound(&self) -> bool {
        self.path != self.current_path
    }
}

#[derive(Deserialize)]
struct RecordedWorkspaceV2 {
    workspace_canonical_path: String,
    workspace_identity: String,
}

/// Binds a workspace to the path its planning checkpoint recorded when design
/// §3 accepts the current location; without a readable planning checkpoint,
/// or with an equal path, the current canonical path is used.
pub(crate) fn bind_nisar_workspace_v2(
    workspace: &Hdf5RangeWorkspaceV1,
) -> Result<NisarBoundWorkspaceV2, Hdf5RangeTransferError> {
    let (current_path, current_identity) = workspace.planning_workspace_identity_v2()?;
    let recorded = workspace
        .load_planning_checkpoint_v2()
        .ok()
        .flatten()
        .and_then(|canonical| serde_json::from_str::<RecordedWorkspaceV2>(&canonical).ok())
        .filter(|recorded| recorded.workspace_canonical_path != current_path);
    let Some(recorded) = recorded else {
        return Ok(NisarBoundWorkspaceV2 {
            path: current_path.clone(),
            identity: current_identity,
            current_path,
        });
    };
    let volume = workspace.root_volume_v2().ok();
    let decision = nisar_workspace_rebinding_permitted_v2(
        cfg!(windows),
        &recorded.workspace_canonical_path,
        &recorded.workspace_identity,
        &current_path,
        &current_identity,
        volume.as_ref(),
    );
    // A refused move keeps the current path, so every existing check fails
    // exactly as before with its existing error code.
    let path = match decision {
        Ok(()) => recorded.workspace_canonical_path,
        Err(_) => current_path.clone(),
    };
    Ok(NisarBoundWorkspaceV2 {
        path,
        identity: current_identity,
        current_path,
    })
}

impl Hdf5RangeWorkspaceV1 {
    /// Drop-in for `planning_workspace_identity_v2` wherever the path is
    /// compared or digested: the bound (recorded or current) path and the
    /// current identity.
    pub(crate) fn bound_workspace_identity_v2(
        &self,
    ) -> Result<(String, String), Hdf5RangeTransferError> {
        let bound = bind_nisar_workspace_v2(self)?;
        Ok((bound.path, bound.identity))
    }
}

/// What an accepted rebinding observed (design §4). Written once as
/// non-authoritative local evidence and recorded as one Store task event; it
/// is never read back as execution authority.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct NisarWorkspaceRebindingObservationV2 {
    pub(crate) contract: &'static str,
    pub(crate) authority: &'static str,
    pub(crate) task_id: String,
    pub(crate) item_id: String,
    pub(crate) recorded_path: String,
    pub(crate) current_path: String,
    pub(crate) identity: String,
    pub(crate) volume_file_system: Option<String>,
    pub(crate) planning_checkpoint_sha256: Option<String>,
    pub(crate) science_checkpoint_sha256: Option<String>,
    pub(crate) anchor_record_sha256: Option<String>,
    pub(crate) anchor_generation: Option<u64>,
    pub(crate) observed_at: String,
    /// Whether the recorded path still reaches this directory: an observation
    /// only, never a condition (a removed junction is allowed).
    pub(crate) recorded_path_resolution: &'static str,
}

/// The Store science anchor facts the evidence names.
pub(crate) struct NisarRebindingAnchorFactsV2<'a> {
    pub(crate) record_sha256: &'a str,
    pub(crate) generation: Option<u64>,
}

fn checkpoint_sha256(canonical: Option<String>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(&canonical?).ok()?;
    value["checkpoint_sha256"].as_str().map(str::to_owned)
}

fn recorded_path_resolution(recorded_path: &str, identity: &str) -> &'static str {
    match Hdf5RangeWorkspaceV1::open_existing(Path::new(recorded_path).to_path_buf())
        .and_then(|workspace| workspace.planning_workspace_identity_v2())
    {
        Ok((_, resolved)) if resolved == identity => "same_identity",
        Ok(_) => "different_identity",
        Err(_) => "unreachable",
    }
}

/// For an accepted rebinding, writes `workspace-rebinding-v2.json`
/// atomically the first time (an existing file is kept) and returns the
/// observation, so the caller can record the Store task event; `None` when
/// the workspace is at its recorded path or the move was refused.
pub(crate) fn record_nisar_workspace_rebinding_v2(
    workspace: &Hdf5RangeWorkspaceV1,
    work: &DownloadWork,
    anchor: Option<NisarRebindingAnchorFactsV2<'_>>,
) -> Result<Option<NisarWorkspaceRebindingObservationV2>, Hdf5RangeTransferError> {
    let bound = bind_nisar_workspace_v2(workspace)?;
    if !bound.is_rebound() {
        return Ok(None);
    }
    let observation = NisarWorkspaceRebindingObservationV2 {
        contract: REBINDING_EVIDENCE_CONTRACT_V2,
        authority: "non_authoritative",
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        recorded_path: bound.path().to_owned(),
        current_path: bound.current_path().to_owned(),
        identity: bound.identity().to_owned(),
        volume_file_system: workspace
            .root_volume_v2()
            .ok()
            .map(|volume| volume.file_system),
        planning_checkpoint_sha256: checkpoint_sha256(
            workspace.load_planning_checkpoint_v2().ok().flatten(),
        ),
        science_checkpoint_sha256: checkpoint_sha256(
            workspace.load_science_checkpoint_v2().ok().flatten(),
        ),
        anchor_record_sha256: anchor
            .as_ref()
            .map(|anchor| anchor.record_sha256.to_owned()),
        anchor_generation: anchor.as_ref().and_then(|anchor| anchor.generation),
        observed_at: crate::scheduler::timestamp_now(),
        recorded_path_resolution: recorded_path_resolution(bound.path(), bound.identity()),
    };
    if workspace.load_rebinding_evidence_v2()?.is_none() {
        let canonical =
            serde_json::to_string(&observation).map_err(|_| Hdf5RangeTransferError::Persistence)?;
        workspace.persist_rebinding_evidence_v2(&canonical)?;
    }
    Ok(Some(observation))
}

/// A local failure of the steps that bind a recovery workspace before the
/// one-time budget-migration decision (review 2026-10-17 round 3, P2-1):
/// opening or creating it, writing its rebinding evidence and recording the
/// Store rebinding event. The worker shows it inside the migration guard, so
/// the prior result row is retained immutably before it is replaced. Only a
/// failed local write, a workspace held by another process and a transient
/// Store state are retryable; the diagnostic tail names the local stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarWorkspaceBindingFailureV2 {
    pub(crate) code: &'static str,
    pub(crate) reason_zh: String,
    pub(crate) retryable: bool,
}

/// The recovery workspace could not be opened (with a Store anchor it must
/// already exist and is never recreated) or created.
pub(crate) fn recovery_workspace_failure_v2(anchored: bool) -> NisarWorkspaceBindingFailureV2 {
    let reason_zh = if anchored {
        "无法打开该影像已有的恢复工作区：保存已下载片段与断点记录的目录可能已被移动、改名或删除。\
         本次没有发起任何网络请求，也没有在原位置重新创建目录。请把输出目录恢复到原来的位置后再继续；\
         若无法恢复，请保留此任务并重新创建任务。[stage=recovery_workspace_open source=local]"
    } else {
        "无法创建本地恢复工作区：输出目录可能不可写、路径不安全或磁盘不可用。\
         本次没有发起任何网络请求。请检查输出目录与磁盘后再继续。\
         [stage=recovery_workspace_create source=local]"
    };
    NisarWorkspaceBindingFailureV2 {
        code: NisarSubsetScienceTransferErrorV2::RecoveryInvalid.code(),
        reason_zh: reason_zh.to_owned(),
        retryable: false,
    }
}

/// Writing the non-authoritative rebinding evidence failed. A failed local
/// write or a workspace held by another process is transient; a replaced
/// directory or an unreadable existing record is not.
pub(crate) fn rebinding_evidence_failure_v2(
    error: Hdf5RangeTransferError,
) -> NisarWorkspaceBindingFailureV2 {
    let cause = match error {
        Hdf5RangeTransferError::Persistence => "persistence",
        Hdf5RangeTransferError::WorkspaceBusy => "workspace_busy",
        Hdf5RangeTransferError::WorkspaceInvalid => "workspace_invalid",
        Hdf5RangeTransferError::RecoveryInvalid => "recovery_invalid",
        _ => "other",
    };
    if matches!(
        error,
        Hdf5RangeTransferError::Persistence | Hdf5RangeTransferError::WorkspaceBusy
    ) {
        return NisarWorkspaceBindingFailureV2 {
            code: NisarSubsetScienceTransferErrorV2::Persistence.code(),
            reason_zh: format!(
                "暂时无法写入输出目录位置变更的本地核对记录（本地文件读写失败，或恢复工作区正被其他程序占用）。\
                 本次没有发起任何网络请求，已下载的片段与断点记录保持不变，可稍后继续。\
                 [stage=workspace_rebinding_evidence source=local cause={cause}]"
            ),
            retryable: true,
        };
    }
    NisarWorkspaceBindingFailureV2 {
        code: NisarSubsetScienceTransferErrorV2::RecoveryInvalid.code(),
        reason_zh: format!(
            "核对输出目录位置变更时，恢复工作区校验失败：目录可能已被替换，或已有的位置变更记录无法读取。\
             本次没有发起任何网络请求，已下载的片段与断点记录没有被修改。\
             请保留此任务供显式恢复，或重新创建任务。\
             [stage=workspace_rebinding_evidence source=local cause={cause}]"
        ),
        retryable: false,
    }
}

/// Recording the Store rebinding event failed with this classified Store
/// error; `None` is a lifecycle stop (pause or cancel).
pub(crate) fn rebinding_event_failure_v2(
    error: NisarSubsetScienceTransferErrorV2,
) -> Option<NisarWorkspaceBindingFailureV2> {
    use NisarSubsetScienceTransferErrorV2 as Error;
    // Review 2026-10-17 round 3, P2-2: a full disk or an I/O error of the
    // local Store is transient with a local disk diagnostic.
    if let Some(reason_zh) = crate::nisar_science_transfer_v2::local_storage_failure_reason_zh(
        error,
        "workspace_rebinding_event",
        "无法记录输出目录位置变更，本次没有发起任何网络请求；",
    ) {
        return Some(NisarWorkspaceBindingFailureV2 {
            code: error.code(),
            reason_zh,
            retryable: true,
        });
    }
    Some(match error {
        Error::Paused | Error::Cancelled => return None,
        Error::Persistence => NisarWorkspaceBindingFailureV2 {
            code: error.code(),
            reason_zh: "本地任务存储暂时无法记录输出目录位置变更（任务存储忙或暂不可用）。\
                        本次没有发起任何网络请求，已下载的片段与断点记录保持不变，可稍后继续。\
                        [stage=workspace_rebinding_event source=local]"
                .to_owned(),
            retryable: true,
        },
        Error::StoreRecordRejected { store_code } => NisarWorkspaceBindingFailureV2 {
            code: store_code,
            reason_zh: format!(
                "本地任务存储拒绝记录输出目录位置变更，这不是暂时性故障。\
                 本次没有发起任何网络请求，已下载的片段与断点记录保持不变。\
                 请保留此任务供显式恢复，或重新创建任务。\
                 [stage=workspace_rebinding_event source=local store_error={store_code}]"
            ),
            retryable: false,
        },
        other => NisarWorkspaceBindingFailureV2 {
            code: other.code(),
            reason_zh: format!(
                "本地任务存储中的执行记录与恢复工作区不一致，无法记录输出目录位置变更。\
                 本次没有发起任何网络请求，已下载的片段与断点记录保持不变。\
                 [stage=workspace_rebinding_event source=local cause={}]",
                other.code()
            ),
            retryable: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: &str = "windows:3405691582:281474976710700";

    fn ntfs() -> NisarWorkspaceVolumeV2 {
        NisarWorkspaceVolumeV2 {
            file_system: "NTFS".to_owned(),
            local: true,
        }
    }

    fn permitted(
        windows: bool,
        recorded: (&str, &str),
        current: (&str, &str),
        volume: Option<&NisarWorkspaceVolumeV2>,
    ) -> Result<(), NisarWorkspaceRebindingRefusalV2> {
        nisar_workspace_rebinding_permitted_v2(
            windows, recorded.0, recorded.1, current.0, current.1, volume,
        )
    }

    #[test]
    fn rebinding_accepts_only_an_unchanged_local_ntfs_directory_identity() {
        use NisarWorkspaceRebindingRefusalV2 as Refusal;
        let old = r"\\?\C:\Data\EOStage\out\.nisar\ws-1";
        let new = r"\\?\C:\Data\EOStage\out\.nisar\ws-1";
        assert_eq!(
            permitted(true, (old, IDENTITY), (new, IDENTITY), Some(&ntfs())),
            Ok(())
        );
        // A drive-letter change of the same volume keeps the identity.
        assert_eq!(
            permitted(
                true,
                (old, IDENTITY),
                (r"\\?\G:\ScenePort\out\.nisar\ws-1", IDENTITY),
                Some(&ntfs())
            ),
            Ok(())
        );
        for (windows, recorded, current, volume, refusal) in [
            (
                false,
                (old, IDENTITY),
                (new, IDENTITY),
                Some(ntfs()),
                Refusal::UnsupportedPlatform,
            ),
            (
                true,
                (old, ""),
                (new, IDENTITY),
                Some(ntfs()),
                Refusal::IdentityMalformed,
            ),
            (
                true,
                (old, "windows:1:0"),
                (new, "windows:1:0"),
                Some(ntfs()),
                Refusal::IdentityMalformed,
            ),
            (
                true,
                (old, "unix:1:2"),
                (new, "unix:1:2"),
                Some(ntfs()),
                Refusal::IdentityMalformed,
            ),
            (
                true,
                (old, "windows:1:2:3"),
                (new, "windows:1:2:3"),
                Some(ntfs()),
                Refusal::IdentityMalformed,
            ),
            // A same-volume copy has another file ID.
            (
                true,
                (old, IDENTITY),
                (new, "windows:3405691582:281474976710701"),
                Some(ntfs()),
                Refusal::IdentityChanged,
            ),
            (
                true,
                (old, IDENTITY),
                (r"\\?\UNC\server\share\ws-1", IDENTITY),
                Some(ntfs()),
                Refusal::PathFormUnsupported,
            ),
            (
                true,
                (r"C:\Data\EOStage\ws-1", IDENTITY),
                (new, IDENTITY),
                Some(ntfs()),
                Refusal::PathFormUnsupported,
            ),
            (
                true,
                (old, IDENTITY),
                (r"\\?\C:\Data\EOStage\out\.nisar\ws-2", IDENTITY),
                Some(ntfs()),
                Refusal::LeafRenamed,
            ),
            (
                true,
                (old, IDENTITY),
                (new, IDENTITY),
                None,
                Refusal::VolumeNotLocalNtfs,
            ),
            (
                true,
                (old, IDENTITY),
                (new, IDENTITY),
                Some(NisarWorkspaceVolumeV2 {
                    file_system: "ReFS".to_owned(),
                    local: true,
                }),
                Refusal::VolumeNotLocalNtfs,
            ),
            (
                true,
                (old, IDENTITY),
                (new, IDENTITY),
                Some(NisarWorkspaceVolumeV2 {
                    file_system: "NTFS".to_owned(),
                    local: false,
                }),
                Refusal::VolumeNotLocalNtfs,
            ),
        ] {
            assert_eq!(
                permitted(windows, recorded, current, volume.as_ref()),
                Err(refusal),
                "{recorded:?} -> {current:?}"
            );
        }
    }

    fn assert_local_without_request(failure: &NisarWorkspaceBindingFailureV2, stage: &str) {
        assert!(
            failure.reason_zh.contains("本次没有发起任何网络请求")
                && failure
                    .reason_zh
                    .contains(&format!("[stage={stage} source=local")),
            "{}",
            failure.reason_zh
        );
    }

    /// Review 2026-10-17 round 3, P2-1: a failure of a step before the
    /// migration decision names its local stage in Chinese; only a failed
    /// local write, a busy workspace and a transient Store state are retryable,
    /// a Store refusal keeps the Store's own code and a stop stays a stop.
    #[test]
    fn workspace_binding_failures_name_the_local_stage_and_only_transient_ones_retry() {
        use NisarSubsetScienceTransferErrorV2 as Science;
        let persistence = Science::Persistence.code();
        let recovery = Science::RecoveryInvalid.code();
        for (anchored, stage) in [
            (true, "recovery_workspace_open"),
            (false, "recovery_workspace_create"),
        ] {
            let failure = recovery_workspace_failure_v2(anchored);
            assert_eq!((failure.code, failure.retryable), (recovery, false));
            assert_local_without_request(&failure, stage);
        }
        for (error, code, retryable) in [
            (Hdf5RangeTransferError::Persistence, persistence, true),
            (Hdf5RangeTransferError::WorkspaceBusy, persistence, true),
            (Hdf5RangeTransferError::WorkspaceInvalid, recovery, false),
            (Hdf5RangeTransferError::RecoveryInvalid, recovery, false),
        ] {
            let failure = rebinding_evidence_failure_v2(error);
            assert_eq!(
                (failure.code, failure.retryable),
                (code, retryable),
                "{error:?}"
            );
            assert_local_without_request(&failure, "workspace_rebinding_evidence");
        }
        assert_eq!(rebinding_event_failure_v2(Science::Paused), None);
        assert_eq!(rebinding_event_failure_v2(Science::Cancelled), None);
        for (error, code, retryable) in [
            (Science::Persistence, persistence, true),
            (
                Science::StoreRecordRejected {
                    store_code: "V3_STORE_CORRUPT_ROW",
                },
                "V3_STORE_CORRUPT_ROW",
                false,
            ),
            (Science::RecoveryInvalid, recovery, false),
        ] {
            let failure = rebinding_event_failure_v2(error).unwrap();
            assert_eq!(
                (failure.code, failure.retryable),
                (code, retryable),
                "{error:?}"
            );
            assert_local_without_request(&failure, "workspace_rebinding_event");
        }
    }

    /// Review 2026-10-17 round 3, P2-2: a full disk or an I/O error while the
    /// Store records the rebinding event is transient with a disk hint.
    #[test]
    fn workspace_rebinding_event_local_storage_failure_is_transient_with_a_disk_hint() {
        let error = NisarSubsetScienceTransferErrorV2::LocalStorageFailure {
            store_code: "V3_STORE_DATABASE_ERROR",
            cause: "io_error",
        };
        let failure = rebinding_event_failure_v2(error).unwrap();
        assert_eq!(
            (failure.code, failure.retryable),
            (NisarSubsetScienceTransferErrorV2::Persistence.code(), true)
        );
        assert_local_without_request(&failure, "workspace_rebinding_event");
        assert!(
            failure.reason_zh.contains("释放磁盘空间或检查磁盘后可继续")
                && failure.reason_zh.contains("cause=io_error"),
            "{}",
            failure.reason_zh
        );
    }
}
