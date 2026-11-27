//! Free-space floor below which downloads pause (PM decision 2026-10-26,
//! A1 §3.1). Pure arithmetic shared by the download engine, which pauses a
//! task, and the application service, which refuses to resume it; neither
//! takes an Agent permission mode.
//!
//! Threshold = max(2 GiB, min(5 % of the volume, 50 GiB)), raised to the
//! remaining bytes of the file being written + 1 GiB when those are known.
//! The 50 GiB cap follows peer download managers, which use absolute floors:
//! without it a 4 TB volume would stop downloading with 200 GB still free.

use crate::{TaskPauseReason, TaskPauseReasonKind, TypedError};

const GIB: u64 = 1 << 30;

/// Error code of a resume refused while the drive is still below the floor.
pub const DISK_LOW_ERROR_CODE: &str = "DISK_LOW";

/// The free-space policy. `Default` is the product rule; tests may tighten it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskLowPolicy {
    /// Absolute floor, whatever the drive size.
    pub floor_bytes: u64,
    /// Floor as a percentage of the drive's total size...
    pub floor_percent_of_total: u64,
    /// ...capped at this many bytes.
    pub floor_percent_cap_bytes: u64,
    /// Headroom kept beyond the remaining bytes of the file being written.
    pub file_headroom_bytes: u64,
}

impl Default for DiskLowPolicy {
    fn default() -> Self {
        Self {
            floor_bytes: 2 * GIB,
            floor_percent_of_total: 5,
            floor_percent_cap_bytes: 50 * GIB,
            file_headroom_bytes: GIB,
        }
    }
}

impl DiskLowPolicy {
    /// The drive floor `max(floor, min(percent of total, cap))`, raised to
    /// `remaining + headroom` when the remaining bytes of the current file are
    /// known.
    pub fn threshold_bytes(&self, total_bytes: u64, remaining_file_bytes: Option<u64>) -> u64 {
        let share = u128::from(total_bytes) * u128::from(self.floor_percent_of_total) / 100;
        let share = u64::try_from(share)
            .unwrap_or(u64::MAX)
            .min(self.floor_percent_cap_bytes);
        let floor = self.floor_bytes.max(share);
        match remaining_file_bytes {
            Some(remaining) => floor.max(remaining.saturating_add(self.file_headroom_bytes)),
            None => floor,
        }
    }

    /// The threshold a resume must meet: the drive floor, raised to the
    /// threshold recorded when the engine paused the task (which may include
    /// the file rule) while it still names the same drive.
    pub fn resume_threshold_bytes(&self, total_bytes: u64, recorded_threshold: Option<u64>) -> u64 {
        self.threshold_bytes(total_bytes, None)
            .max(recorded_threshold.unwrap_or(0))
    }

    /// The typed pause reason when `free_bytes` is below the threshold.
    pub fn evaluate(
        &self,
        free_bytes: u64,
        total_bytes: u64,
        remaining_file_bytes: Option<u64>,
        drive: Option<&str>,
    ) -> Option<TaskPauseReason> {
        below(
            free_bytes,
            self.threshold_bytes(total_bytes, remaining_file_bytes),
            drive,
        )
    }
}

/// The typed reason when `free_bytes < threshold_bytes`. An empty or unknown
/// drive is `None`, which the UI shows as 未知盘符.
pub fn below(
    free_bytes: u64,
    threshold_bytes: u64,
    drive: Option<&str>,
) -> Option<TaskPauseReason> {
    (free_bytes < threshold_bytes).then(|| TaskPauseReason {
        kind: TaskPauseReasonKind::DiskLow,
        free_bytes,
        threshold_bytes,
        drive: drive.filter(|drive| !drive.is_empty()).map(str::to_owned),
    })
}

/// Bytes as the GB figure Windows Explorer shows (binary units, one decimal).
pub fn format_gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / GIB as f64)
}

/// `DISK_LOW`: a resume refused because the drive is still below the floor.
pub fn disk_low_resume_error(reason: &TaskPauseReason) -> TypedError {
    TypedError {
        code: DISK_LOW_ERROR_CODE.to_owned(),
        message_zh: format!(
            "磁盘剩余空间不足，无法继续下载：{} 剩余 {} GB",
            reason.drive.as_deref().unwrap_or("未知盘符"),
            format_gb(reason.free_bytes)
        ),
        retryable: true,
        needs_account: None,
        needs_user_confirmation: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: u64 = 1_000_000_000_000;

    #[test]
    fn floor_is_the_larger_of_two_gib_and_five_percent() {
        let policy = DiskLowPolicy::default();
        // 20 GiB drive: 5 % is 1 GiB, so the 2 GiB floor applies.
        assert_eq!(policy.threshold_bytes(20 * GIB, None), 2 * GIB);
        // 1 TB drive: 5 % is 50 GB, under the 50 GiB cap.
        assert_eq!(policy.threshold_bytes(TB, None), TB / 20);
    }

    #[test]
    fn the_percentage_floor_is_capped_at_fifty_gib() {
        let policy = DiskLowPolicy::default();
        // 4 TB volume: 5 % would be 200 GB; the cap keeps it at 50 GiB.
        assert_eq!(policy.threshold_bytes(4 * TB, None), 50 * GIB);
        assert_eq!(policy.evaluate(60 * GIB, 4 * TB, None, Some("F:")), None);
        assert!(policy
            .evaluate(49 * GIB, 4 * TB, None, Some("F:"))
            .is_some());
        // A large file still raises it above the cap.
        assert_eq!(policy.threshold_bytes(4 * TB, Some(80 * GIB)), 81 * GIB);
    }

    #[test]
    fn known_remaining_bytes_raise_the_floor_only_when_larger() {
        let policy = DiskLowPolicy::default();
        assert_eq!(policy.threshold_bytes(20 * GIB, Some(5 * GIB)), 6 * GIB);
        assert_eq!(policy.threshold_bytes(20 * GIB, Some(GIB / 2)), 2 * GIB);
        assert_eq!(policy.threshold_bytes(u64::MAX, Some(u64::MAX)), u64::MAX);
    }

    #[test]
    fn a_resume_keeps_the_recorded_file_threshold() {
        let policy = DiskLowPolicy::default();
        // Paused writing an 8 GiB file with 6 GiB free: recorded 9 GiB.
        assert_eq!(
            policy.resume_threshold_bytes(20 * GIB, Some(9 * GIB)),
            9 * GIB
        );
        assert_eq!(policy.resume_threshold_bytes(20 * GIB, None), 2 * GIB);
        assert_eq!(policy.resume_threshold_bytes(20 * GIB, Some(GIB)), 2 * GIB);
    }

    #[test]
    fn evaluate_reports_free_threshold_and_drive_only_below_the_floor() {
        let policy = DiskLowPolicy::default();
        let reason = policy
            .evaluate(GIB, 20 * GIB, None, Some("F:"))
            .expect("1 GiB free is below 2 GiB");
        assert_eq!(reason.kind, TaskPauseReasonKind::DiskLow);
        assert_eq!(reason.free_bytes, GIB);
        assert_eq!(reason.threshold_bytes, 2 * GIB);
        assert_eq!(reason.drive.as_deref(), Some("F:"));
        assert_eq!(policy.evaluate(2 * GIB, 20 * GIB, None, Some("F:")), None);
        assert!(policy
            .evaluate(3 * GIB, 20 * GIB, Some(3 * GIB), Some("F:"))
            .is_some());
        let unknown = policy.evaluate(GIB, 20 * GIB, None, None).unwrap();
        assert_eq!(unknown.drive, None);
        assert_eq!(
            policy
                .evaluate(GIB, 20 * GIB, None, Some(""))
                .unwrap()
                .drive,
            None
        );
    }

    #[test]
    fn resume_error_names_the_drive_and_remaining_gigabytes() {
        let mut reason = TaskPauseReason {
            kind: TaskPauseReasonKind::DiskLow,
            free_bytes: GIB + GIB / 2,
            threshold_bytes: 2 * GIB,
            drive: Some("F:".to_owned()),
        };
        let error = disk_low_resume_error(&reason);
        assert_eq!(error.code, "DISK_LOW");
        assert_eq!(
            error.message_zh,
            "磁盘剩余空间不足，无法继续下载：F: 剩余 1.5 GB"
        );
        assert!(error.retryable);
        reason.drive = None;
        assert_eq!(
            disk_low_resume_error(&reason).message_zh,
            "磁盘剩余空间不足，无法继续下载：未知盘符 剩余 1.5 GB"
        );
    }
}
