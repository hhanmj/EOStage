//! Downloader enhancement D2 (2026-10-18): stage timings of one NISAR V2
//! writer run. L4 measured 2.5 to 3.5 minutes without any byte after a
//! continue while the writer revalidated its science material; each run now
//! reports how long it spent in every stage. One secret-free line goes to the
//! local diagnostic stream when the run returns, whatever its outcome:
//!
//! `NISAR_SUBSET_V2_WRITER_TIMINGS task_id=… item_id=… outcome=… total_ms=…
//! <stage>_ms=…` (only the stages the run reached, in order).

use std::time::{Duration, Instant};

/// The stages of one writer run, in the order they are entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarWriterStageV2 {
    /// Waiting for the artifact lease, its locks and an output slot (D1).
    Admission,
    /// The Store authority revalidation, the pin and the first full digest of
    /// the science material at the writer entry.
    EntryRevalidation,
    /// Reading the metadata closure from the pinned source.
    MetadataCapture,
    /// Reading the derived window axes and planning their tiles.
    WindowCapture,
    /// Validating an artifact a previous run already published.
    ArtifactRecovery,
    /// Writing (or reopening the kept) candidate.
    CandidateWrite,
    /// The streamed bit-for-bit validation and digest of the candidate.
    CandidateValidation,
    /// The Store authority revalidation and the second and last full digest of
    /// the science material before publication.
    FinalRevalidation,
    /// Promotion, post-promotion digests, sidecar and recovery anchors.
    Publication,
}

impl NisarWriterStageV2 {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::EntryRevalidation => "entry_revalidation",
            Self::MetadataCapture => "metadata_capture",
            Self::WindowCapture => "window_capture",
            Self::ArtifactRecovery => "artifact_recovery",
            Self::CandidateWrite => "candidate_write",
            Self::CandidateValidation => "candidate_validation",
            Self::FinalRevalidation => "final_revalidation",
            Self::Publication => "publication",
        }
    }
}

/// Records how long each entered stage lasted.
#[derive(Debug)]
pub(crate) struct NisarWriterStageTimerV2 {
    started: Instant,
    current: Option<(NisarWriterStageV2, Instant)>,
    stages: Vec<(NisarWriterStageV2, Duration)>,
}

impl NisarWriterStageTimerV2 {
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
            current: None,
            stages: Vec::new(),
        }
    }

    /// Ends the current stage and starts `stage`.
    pub(crate) fn enter(&mut self, stage: NisarWriterStageV2) {
        let now = Instant::now();
        self.close(now);
        self.current = Some((stage, now));
    }

    fn close(&mut self, now: Instant) {
        if let Some((stage, entered)) = self.current.take() {
            self.stages
                .push((stage, now.saturating_duration_since(entered)));
        }
    }

    pub(crate) fn finish(mut self) -> NisarWriterStageTimingsV2 {
        let now = Instant::now();
        self.close(now);
        NisarWriterStageTimingsV2 {
            total: now.saturating_duration_since(self.started),
            stages: self.stages,
        }
    }
}

/// The stage durations of one finished writer run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarWriterStageTimingsV2 {
    total: Duration,
    stages: Vec<(NisarWriterStageV2, Duration)>,
}

impl NisarWriterStageTimingsV2 {
    #[cfg(test)]
    pub(crate) fn stages(&self) -> Vec<NisarWriterStageV2> {
        self.stages.iter().map(|(stage, _)| *stage).collect()
    }

    /// The line written to the local diagnostic stream: identifiers, the
    /// outcome and milliseconds only, never paths or provider text.
    pub(crate) fn diagnostic_line(&self, task_id: &str, item_id: &str, outcome: &str) -> String {
        let mut line = format!(
            "NISAR_SUBSET_V2_WRITER_TIMINGS task_id={task_id} item_id={item_id} \
             outcome={outcome} total_ms={}",
            self.total.as_millis()
        );
        for (stage, duration) in &self.stages {
            line.push_str(&format!(" {}_ms={}", stage.label(), duration.as_millis()));
        }
        line
    }
}

#[cfg(test)]
thread_local! {
    static LAST_WRITER_TIMINGS: std::cell::RefCell<Option<NisarWriterStageTimingsV2>> =
        const { std::cell::RefCell::new(None) };
}

/// Keeps the timings of the last writer run on this thread (tests only).
#[cfg(test)]
pub(crate) fn record_last_writer_timings_for_test(timings: NisarWriterStageTimingsV2) {
    LAST_WRITER_TIMINGS.with(|slot| *slot.borrow_mut() = Some(timings));
}

/// The timings of the last writer run on this thread (tests only).
#[cfg(test)]
pub(crate) fn last_writer_timings_for_test() -> Option<NisarWriterStageTimingsV2> {
    LAST_WRITER_TIMINGS.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timings_list_each_entered_stage_once_in_order_with_the_outcome() {
        let mut timer = NisarWriterStageTimerV2::start();
        timer.enter(NisarWriterStageV2::Admission);
        timer.enter(NisarWriterStageV2::EntryRevalidation);
        std::thread::sleep(Duration::from_millis(20));
        timer.enter(NisarWriterStageV2::Publication);
        let timings = timer.finish();
        assert_eq!(
            timings.stages(),
            vec![
                NisarWriterStageV2::Admission,
                NisarWriterStageV2::EntryRevalidation,
                NisarWriterStageV2::Publication
            ]
        );
        let line = timings.diagnostic_line("task-1", "item-1", "completed");
        assert!(line.starts_with(
            "NISAR_SUBSET_V2_WRITER_TIMINGS task_id=task-1 item_id=item-1 outcome=completed total_ms="
        ));
        let entry_ms = line
            .split(' ')
            .find_map(|field| field.strip_prefix("entry_revalidation_ms="))
            .and_then(|value| value.parse::<u64>().ok())
            .expect("the entry revalidation stage is reported");
        assert!(entry_ms >= 20, "{line}");
        assert!(line.contains(" admission_ms=") && line.contains(" publication_ms="));
        assert!(!line.contains("candidate_write_ms"), "{line}");
    }
}
