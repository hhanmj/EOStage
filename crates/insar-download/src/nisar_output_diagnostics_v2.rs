//! Local diagnostics of NISAR V2 output failures (Issue #8, slice 6).
//!
//! A failure after planning or in the writer is local: no provider answered
//! it. Each stable code is kept and gets a Chinese summary of the local cause,
//! the fact that downloaded data and the checkpoint are kept, what a retry
//! does, and the tail `[stage=… source=local code=…]`, with `required=` and
//! `limit=` (bytes) when a size bound or the free space refused the output
//! (for free space, `limit` is the available space). A resource failure that
//! freeing space resolves is retryable; a size bound is not, and a retry after
//! an upgrade only regenerates the file.

use crate::nisar_planning_v2::NisarSubsetPhysicalExecutionPlanV2;
use crate::nisar_subset_stream_v2::{
    v2_derived_output_logical_bytes, v2_largest_output_chunk_bytes, v2_output_logical_limit,
    V2OutputAdmissionRefusal, V2_STREAM_TILE_LIMIT_BYTES,
};
use crate::NisarSubsetWriteError;

const KEPT: &str = "已下载数据与断点已保留";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarV2LocalFailure {
    pub(crate) code: &'static str,
    pub(crate) reason_zh: String,
    pub(crate) retryable: bool,
}

fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

fn tail(stage: &str, code: &str, sizes: Option<(u64, u64)>) -> String {
    match sizes {
        Some((required, limit)) => {
            format!("[stage={stage} source=local code={code} required={required} limit={limit}]")
        }
        None => format!("[stage={stage} source=local code={code}]"),
    }
}

fn failure(
    stage: &str,
    code: &'static str,
    summary: String,
    sizes: Option<(u64, u64)>,
    retryable: bool,
) -> NisarV2LocalFailure {
    NisarV2LocalFailure {
        code,
        reason_zh: format!("{summary} {}", tail(stage, code, sizes)),
        retryable,
    }
}

/// An output refused by the shared bound or by missing free space at
/// `stage`; `network` states what the refusal spared (for example that no
/// science request was sent).
pub(crate) fn nisar_v2_output_refusal(
    stage: &str,
    refusal: V2OutputAdmissionRefusal,
    network: &str,
) -> NisarV2LocalFailure {
    match refusal {
        V2OutputAdmissionRefusal::OutputLimit { required, limit } => failure(
            stage,
            NisarSubsetWriteError::LogicalCaptureLimitExceeded.code(),
            format!(
                "裁剪结果超出本机输出上限：派生数据约 {} MiB，上限 {} MiB，未生成文件。{network}{KEPT}。\
                 请缩小 AOI 后重新创建任务；若升级放宽了上限，升级后重试只重新生成文件、不重新下载。",
                mib(required),
                mib(limit)
            ),
            Some((required, limit)),
            false,
        ),
        V2OutputAdmissionRefusal::DiskSpace {
            required,
            available,
        } => failure(
            stage,
            (NisarSubsetWriteError::InsufficientDiskSpace {
                required,
                available,
            })
            .code(),
            format!(
                "输出目录所在磁盘剩余空间不足：需要约 {} MiB，可用 {} MiB，未生成文件。{network}{KEPT}。\
                 释放磁盘空间后重试即可，已下载的分段不会重新下载。",
                mib(required),
                mib(available)
            ),
            Some((required, available)),
            true,
        ),
    }
}

/// A writer failure with its stable code (`RecoveryInvalid` keeps the
/// writer-specific recovery code). `plan` supplies the sizes of a size-bound
/// refusal. Every writer failure follows a complete science transfer.
pub(crate) fn nisar_v2_writer_failure_zh(
    error: NisarSubsetWriteError,
    plan: Option<&NisarSubsetPhysicalExecutionPlanV2>,
) -> NisarV2LocalFailure {
    const STAGE: &str = "writer";
    const REGENERATES: &str = "重试只重新生成文件、不重新下载";
    const NO_REPAIR: &str = "重试不会自动修复，请保留此任务与输出目录供排查";
    let code = match error {
        NisarSubsetWriteError::RecoveryInvalid => "NISAR_SUBSET_V2_WRITER_RECOVERY_INVALID",
        _ => error.code(),
    };
    let (summary, sizes, retryable) = match error {
        NisarSubsetWriteError::LogicalCaptureLimitExceeded => {
            let chunk = plan.map_or(0, v2_largest_output_chunk_bytes);
            let tile_limit = V2_STREAM_TILE_LIMIT_BYTES as u64;
            if chunk > tile_limit {
                (
                    format!(
                        "源数据的单个数据块过大（约 {} MiB，写出缓冲上限 {} MiB），未生成文件。{KEPT}；\
                         若升级放宽了上限，升级后{REGENERATES}。",
                        mib(chunk),
                        mib(tile_limit)
                    ),
                    Some((chunk, tile_limit)),
                    false,
                )
            } else {
                let required = plan
                    .and_then(v2_derived_output_logical_bytes)
                    .unwrap_or(u64::MAX);
                let limit = v2_output_logical_limit();
                (
                    format!(
                        "裁剪结果超出本机输出上限：派生数据约 {} MiB，上限 {} MiB，未生成文件。{KEPT}；\
                         请缩小 AOI 后重新创建任务；若升级放宽了上限，升级后{REGENERATES}。",
                        mib(required),
                        mib(limit)
                    ),
                    Some((required, limit)),
                    false,
                )
            }
        }
        NisarSubsetWriteError::InsufficientDiskSpace {
            required,
            available,
        } => (
            format!(
                "输出目录所在磁盘剩余空间不足：需要约 {} MiB，可用 {} MiB，未生成文件。{KEPT}；\
                 释放磁盘空间后{REGENERATES}。",
                mib(required),
                mib(available)
            ),
            Some((required, available)),
            true,
        ),
        NisarSubsetWriteError::Persistence => (
            format!(
                "写入或复核读取裁剪文件时本地磁盘读写失败（磁盘可能已满、文件被其他程序占用或暂时无法读取），\
                 文件未发布。{KEPT}；检查磁盘后{REGENERATES}。"
            ),
            None,
            true,
        ),
        NisarSubsetWriteError::AtomicPromotion => (
            format!(
                "裁剪文件已生成并通过复核，但发布到输出目录失败（目标可能被其他程序占用）。{KEPT}；\
                 关闭占用该文件的程序后{REGENERATES}。"
            ),
            None,
            true,
        ),
        NisarSubsetWriteError::SelectedScienceMissing => (
            format!(
                "本地已下载的数据中缺少所选极化的科学数据或有效性掩膜，无法生成裁剪结果。{KEPT}；{NO_REPAIR}。"
            ),
            None,
            false,
        ),
        NisarSubsetWriteError::MultiPolarizationInvalid => (
            format!("冻结计划中的极化与写出选择不一致，未生成文件。{KEPT}；{NO_REPAIR}。"),
            None,
            false,
        ),
        NisarSubsetWriteError::PostWriteScienceValidation => (
            format!(
                "生成的裁剪文件与已下载的源数据逐位复核不一致，文件未发布。{KEPT}；{NO_REPAIR}。"
            ),
            None,
            false,
        ),
        NisarSubsetWriteError::ManifestInvalid => (
            format!("裁剪结果清单（sidecar）与复核结果不一致。{KEPT}；{NO_REPAIR}。"),
            None,
            false,
        ),
        NisarSubsetWriteError::RecoveryInvalid => (
            format!(
                "输出目录中上次写出遗留的文件或恢复记录与本任务不一致，未覆盖已有文件。{KEPT}；\
                 {NO_REPAIR}，不要手动删除恢复记录。"
            ),
            None,
            false,
        ),
        NisarSubsetWriteError::IncompleteScienceAuthority => (
            format!(
                "写出前复核已下载的分段未通过（断点或分段材料与记录不一致），未生成文件。{KEPT}；{NO_REPAIR}。"
            ),
            None,
            false,
        ),
        NisarSubsetWriteError::WriterBinding => (
            format!("写出器与冻结计划或已下载数据的绑定复核未通过，未生成文件。{KEPT}；{NO_REPAIR}。"),
            None,
            false,
        ),
        NisarSubsetWriteError::OutputUnsafe => (
            format!(
                "输出目录不安全或无法访问（路径被替换、权限不足或指向链接），未生成文件。{KEPT}；\
                 这是输出目录的安全检查，重试不会自动修复；请先修复输出目录的权限或链接。"
            ),
            None,
            false,
        ),
        NisarSubsetWriteError::OutputBusy => (
            format!(
                "同一输出目录正在被另一个写出任务占用（输出目录写出锁忙），本次未生成文件。{KEPT}；\
                 等另一个写出结束后{REGENERATES}。"
            ),
            None,
            true,
        ),
        NisarSubsetWriteError::MetadataInvalid
        | NisarSubsetWriteError::RequestInvalid
        | NisarSubsetWriteError::BlockEvidenceInvalid
        | NisarSubsetWriteError::ValidationFailed
        | NisarSubsetWriteError::TransferIncomplete
        | NisarSubsetWriteError::Cancelled
        | NisarSubsetWriteError::InjectedFailure => (
            format!("生成裁剪文件时本地复核未通过，未生成文件。{KEPT}；{NO_REPAIR}。"),
            None,
            false,
        ),
    };
    failure(STAGE, code, summary, sizes, retryable)
}

/// The Store-frozen plan could not be installed into the execution snapshot.
pub(crate) fn nisar_v2_frozen_plan_installation_failure() -> NisarV2LocalFailure {
    failure(
        "plan_installation",
        "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID",
        format!(
            "任务存储中冻结的裁剪计划与本次执行快照不一致，本次没有发起科学数据请求。{KEPT}；\
             重试不会自动修复，请保留此任务供排查或重新创建任务。"
        ),
        None,
        false,
    )
}

/// The writer reported success but the published artifact or its sidecar is
/// not a durable regular file afterwards.
pub(crate) fn nisar_v2_published_output_failure(sidecar: bool) -> NisarV2LocalFailure {
    let (code, what) = if sidecar {
        ("NISAR_SUBSET_V2_SIDECAR_INVALID", "裁剪结果清单（sidecar）")
    } else {
        ("NISAR_SUBSET_V2_ARTIFACT_INVALID", "裁剪文件")
    };
    failure(
        "writer",
        code,
        format!(
            "写出完成后{what}不是可用的普通文件（可能被其他程序移动或删除）。{KEPT}；\
             请保留输出目录供排查，不要手动删除恢复记录。"
        ),
        None,
        false,
    )
}

#[cfg(test)]
#[path = "nisar_output_diagnostics_v2_tests.rs"]
mod tests;
