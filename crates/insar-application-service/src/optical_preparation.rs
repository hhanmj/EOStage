//! Short-lived admission observations, not a second task queue or a creation receipt.
use super::{
    invalid_request, valid_catalogue_opaque_id, ApplicationService, ApplicationServiceError,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpticalPreparationPhase {
    Preparing,
    Committing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpticalPreparationProgress {
    pub request_id: String,
    pub requested_count: u64,
    pub prepared_count: u64,
    pub admitted_count: u64,
    pub rejected_count: u64,
    pub phase: OpticalPreparationPhase,
}

#[derive(Default)]
pub(super) struct OpticalPreparationRegistry {
    active: Mutex<BTreeMap<String, Arc<Mutex<OpticalPreparationProgress>>>>,
}

pub(super) struct OpticalPreparationGuard<'a> {
    registry: &'a OpticalPreparationRegistry,
    request_id: String,
    progress: Arc<Mutex<OpticalPreparationProgress>>,
}

impl OpticalPreparationRegistry {
    pub(super) fn begin(
        &self,
        request_id: &str,
        count: usize,
    ) -> Result<OpticalPreparationGuard<'_>, ApplicationServiceError> {
        if !valid_catalogue_opaque_id(request_id) || !(1..=250).contains(&count) {
            return Err(invalid_request(
                "invalid optical preparation identity or count",
            ));
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        // Never evict an active operation to make room for another one.
        if active.contains_key(request_id) || active.len() >= 32 {
            return Err(invalid_request(
                "optical preparation is already active or at capacity",
            ));
        }
        let progress = Arc::new(Mutex::new(OpticalPreparationProgress {
            request_id: request_id.to_owned(),
            requested_count: count as u64,
            prepared_count: 0,
            admitted_count: 0,
            rejected_count: 0,
            phase: OpticalPreparationPhase::Preparing,
        }));
        active.insert(request_id.to_owned(), Arc::clone(&progress));
        Ok(OpticalPreparationGuard {
            registry: self,
            request_id: request_id.to_owned(),
            progress,
        })
    }

    fn read(
        &self,
        request_id: &str,
    ) -> Result<Option<OpticalPreparationProgress>, ApplicationServiceError> {
        let progress = self
            .active
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .get(request_id)
            .cloned();
        progress
            .map(|value| {
                value
                    .lock()
                    .map(|value| value.clone())
                    .map_err(|_| ApplicationServiceError::StatePoisoned)
            })
            .transpose()
    }
}

impl OpticalPreparationGuard<'_> {
    pub(super) fn record(&self, admitted: bool) -> Result<(), ApplicationServiceError> {
        let mut value = self
            .progress
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if value.phase != OpticalPreparationPhase::Preparing
            || value.prepared_count >= value.requested_count
        {
            return Err(invalid_request(
                "optical preparation count exceeds its frozen selection",
            ));
        }
        value.prepared_count += 1;
        if admitted {
            value.admitted_count += 1;
        } else {
            value.rejected_count += 1;
        }
        Ok(())
    }

    pub(super) fn committing(&self) -> Result<(), ApplicationServiceError> {
        let mut value = self
            .progress
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?;
        if value.prepared_count != value.requested_count || value.admitted_count == 0 {
            return Err(invalid_request(
                "incomplete optical preparation cannot commit",
            ));
        }
        value.phase = OpticalPreparationPhase::Committing;
        Ok(())
    }
}

impl Drop for OpticalPreparationGuard<'_> {
    fn drop(&mut self) {
        let mut active = self
            .registry
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .get(&self.request_id)
            .is_some_and(|value| Arc::ptr_eq(value, &self.progress))
        {
            active.remove(&self.request_id);
        }
    }
}

impl ApplicationService {
    pub fn optical_preparation_progress(
        &self,
        request_id: &str,
    ) -> Result<Option<OpticalPreparationProgress>, ApplicationServiceError> {
        self.ensure_running()?;
        if !valid_catalogue_opaque_id(request_id) {
            return Err(invalid_request("invalid preparation request identity"));
        }
        self.optical_preparations.read(request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_counts_are_atomic_bounded_and_cleared_before_retry() {
        let registry = OpticalPreparationRegistry::default();
        let guard = registry.begin("request", 100).unwrap();
        assert!(registry.begin("request", 100).is_err());
        assert!(guard.committing().is_err());
        std::thread::scope(|scope| {
            for index in 0..100 {
                let guard = &guard;
                scope.spawn(move || guard.record(index % 2 == 0).unwrap());
            }
        });
        let progress = registry.read("request").unwrap().unwrap();
        assert_eq!(
            (
                progress.prepared_count,
                progress.admitted_count,
                progress.rejected_count
            ),
            (100, 50, 50)
        );
        guard.committing().unwrap();
        assert!(guard.record(true).is_err());
        drop(guard);
        assert!(registry.read("request").unwrap().is_none());
        let _retry = registry.begin("request", 2).unwrap();
        assert_eq!(registry.read("request").unwrap().unwrap().prepared_count, 0);
    }
    #[test]
    fn rejected_and_unwound_operations_never_leave_progress_or_claim_commit() {
        let registry = OpticalPreparationRegistry::default();
        let guard = registry.begin("rejected", 1).unwrap();
        guard.record(false).unwrap();
        assert!(guard.committing().is_err());
        drop(guard);
        let _ = std::panic::catch_unwind(|| {
            let _guard = registry.begin("panic", 1).unwrap();
            panic!("fixture unwind");
        });
        assert!(registry.read("panic").unwrap().is_none());
        assert!(registry.read("rejected").unwrap().is_none());
        assert!(registry.read("never-created").unwrap().is_none());
    }
}
