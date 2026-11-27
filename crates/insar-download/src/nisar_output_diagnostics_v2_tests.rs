use super::*;

#[test]
fn an_output_above_the_bound_is_a_local_non_retryable_refusal_with_its_sizes() {
    let failure = nisar_v2_output_refusal(
        "planning_admission",
        V2OutputAdmissionRefusal::OutputLimit {
            required: 3 * 1024 * 1024 * 1024,
            limit: 2 * 1024 * 1024 * 1024,
        },
        "计划未冻结，没有发起科学数据请求；",
    );
    assert_eq!(failure.code, "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT");
    assert!(!failure.retryable);
    for expected in [
        "裁剪结果超出本机输出上限",
        "派生数据约 3072.0 MiB，上限 2048.0 MiB",
        "计划未冻结，没有发起科学数据请求；已下载数据与断点已保留",
        "升级后重试只重新生成文件、不重新下载",
        "[stage=planning_admission source=local code=NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT \
         required=3221225472 limit=2147483648]",
    ] {
        assert!(
            failure.reason_zh.contains(expected),
            "{}",
            failure.reason_zh
        );
    }
}

#[test]
fn missing_free_space_is_a_local_retryable_refusal_with_its_sizes() {
    let failure = nisar_v2_output_refusal(
        "pre_science_admission",
        V2OutputAdmissionRefusal::DiskSpace {
            required: 5 * 1024 * 1024,
            available: 1024 * 1024,
        },
        "",
    );
    assert_eq!(
        failure.code,
        "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT"
    );
    assert!(failure.retryable);
    for expected in [
        "输出目录所在磁盘剩余空间不足：需要约 5.0 MiB，可用 1.0 MiB",
        "已下载数据与断点已保留",
        "释放磁盘空间后重试即可",
        "[stage=pre_science_admission source=local \
         code=NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT required=5242880 limit=1048576]",
    ] {
        assert!(
            failure.reason_zh.contains(expected),
            "{}",
            failure.reason_zh
        );
    }
}

#[test]
fn published_output_and_plan_installation_failures_are_local_and_final() {
    for (failure, code, stage) in [
        (
            nisar_v2_published_output_failure(false),
            "NISAR_SUBSET_V2_ARTIFACT_INVALID",
            "writer",
        ),
        (
            nisar_v2_published_output_failure(true),
            "NISAR_SUBSET_V2_SIDECAR_INVALID",
            "writer",
        ),
        (
            nisar_v2_frozen_plan_installation_failure(),
            "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID",
            "plan_installation",
        ),
    ] {
        assert_eq!(failure.code, code);
        assert!(!failure.retryable, "{code}");
        assert!(failure.reason_zh.contains("已下载数据与断点已保留"));
        assert!(failure
            .reason_zh
            .ends_with(&format!("[stage={stage} source=local code={code}]")));
    }
}

#[test]
fn a_size_bound_writer_failure_names_its_required_and_limit_bytes() {
    let failure =
        nisar_v2_writer_failure_zh(NisarSubsetWriteError::LogicalCaptureLimitExceeded, None);
    assert_eq!(failure.code, "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT");
    assert!(!failure.retryable);
    assert!(
        failure.reason_zh.contains(&format!(
            "required={} limit={}]",
            u64::MAX,
            crate::nisar_subset_stream_v2::V2_OUTPUT_LOGICAL_LIMIT_BYTES
        )),
        "{}",
        failure.reason_zh
    );
    assert!(failure
        .reason_zh
        .contains("升级后重试只重新生成文件、不重新下载"));
}

/// Review of slice 6 (P2): NISAR_SUBSET_ARTIFACT_INVALID names both a local
/// read or write failure, which a retry regenerates, and an unsafe output
/// directory, which a retry cannot fix; each text matches its retry class.
#[test]
fn artifact_invalid_texts_match_their_retry_class() {
    let persistence = nisar_v2_writer_failure_zh(NisarSubsetWriteError::Persistence, None);
    assert_eq!(persistence.code, "NISAR_SUBSET_ARTIFACT_INVALID");
    assert!(persistence.retryable);
    for expected in ["读取", "写入", "重试只重新生成文件、不重新下载"] {
        assert!(
            persistence.reason_zh.contains(expected),
            "{}",
            persistence.reason_zh
        );
    }
    let unsafe_output = nisar_v2_writer_failure_zh(NisarSubsetWriteError::OutputUnsafe, None);
    assert_eq!(unsafe_output.code, "NISAR_SUBSET_ARTIFACT_INVALID");
    assert!(!unsafe_output.retryable);
    assert!(
        !unsafe_output.reason_zh.contains("重试只重新生成"),
        "{}",
        unsafe_output.reason_zh
    );
    assert!(
        unsafe_output.reason_zh.contains("重试不会自动修复"),
        "{}",
        unsafe_output.reason_zh
    );
}
