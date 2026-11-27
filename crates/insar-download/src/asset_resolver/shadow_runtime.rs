use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Duration;

use insar_core::FrozenAssetAccess;

use super::{
    AssetResolverRegistry, Clock, ResolveError, ShadowCredentialShapeProvider,
    ShadowDestinationPolicy, ShadowMismatchCode, ShadowResolveContext, TransferIntentFingerprint,
};

pub const SHADOW_DESCRIPTOR_UNAVAILABLE: &str = "SHADOW_DESCRIPTOR_UNAVAILABLE";
pub const SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE: &str = "SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowAssetClass {
    Sentinel1Standard,
    BurstTiff,
    BurstXml,
    HistoricalAsf,
    OrbitDirectEof,
    LegacyUrl,
    OrbitSceneId,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowOutcome {
    Parity,
    Mismatch,
    Ineligible,
    ResolverError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverShadowObservation {
    pub asset_class: ShadowAssetClass,
    pub outcome: ShadowOutcome,
    pub mismatch_codes: Vec<ShadowMismatchCode>,
    pub resolver_error_code: Option<&'static str>,
}

pub struct ResolverShadowDiagnostics {
    capacity: usize,
    items: Mutex<VecDeque<ResolverShadowObservation>>,
}

impl ResolverShadowDiagnostics {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            items: Mutex::new(VecDeque::new()),
        }
    }

    fn record(&self, observation: ResolverShadowObservation) {
        let Ok(mut items) = self.items.try_lock() else {
            return;
        };
        if items.len() == self.capacity {
            items.pop_front();
        }
        items.push_back(observation);
    }

    pub fn observations(&self) -> Vec<ResolverShadowObservation> {
        match self.items.try_lock() {
            Ok(items) => items.iter().cloned().collect(),
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => Vec::new(),
        }
    }
}

pub struct ResolverShadowInput {
    asset_class: ShadowAssetClass,
    frozen: Option<FrozenAssetAccess>,
    legacy: TransferIntentFingerprint,
    destination_policy: ShadowDestinationPolicy,
    cancelled: bool,
}

impl ResolverShadowInput {
    pub fn new(
        asset_class: ShadowAssetClass,
        frozen: Option<FrozenAssetAccess>,
        legacy: TransferIntentFingerprint,
        destination_policy: ShadowDestinationPolicy,
    ) -> Self {
        Self {
            asset_class,
            frozen,
            legacy,
            destination_policy,
            cancelled: false,
        }
    }

    pub fn with_cancelled(mut self, cancelled: bool) -> Self {
        self.cancelled = cancelled;
        self
    }
}

impl fmt::Debug for ResolverShadowInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolverShadowInput")
            .field("asset_class", &self.asset_class)
            .field("frozen", &self.frozen.is_some())
            .field("legacy", &self.legacy)
            .field("destination_policy", &"[REDACTED]")
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

pub trait ResolverShadowObserver: Send + Sync {
    fn observe(&self, input: &ResolverShadowInput);
}

pub struct ResolverShadowRuntime {
    registry: AssetResolverRegistry,
    credential_shapes: Arc<dyn ShadowCredentialShapeProvider>,
    clock: Arc<dyn Clock>,
    diagnostics: Arc<ResolverShadowDiagnostics>,
    enabled_classes: HashSet<ShadowAssetClass>,
    budget: Duration,
}

impl ResolverShadowRuntime {
    pub fn new(
        registry: AssetResolverRegistry,
        credential_shapes: Arc<dyn ShadowCredentialShapeProvider>,
        clock: Arc<dyn Clock>,
        diagnostics: Arc<ResolverShadowDiagnostics>,
        enabled_classes: impl IntoIterator<Item = ShadowAssetClass>,
        budget: Duration,
    ) -> Result<Self, ResolveError> {
        if budget.is_zero() {
            return Err(ResolveError::descriptor_invalid());
        }
        Ok(Self {
            registry,
            credential_shapes,
            clock,
            diagnostics,
            enabled_classes: enabled_classes.into_iter().collect(),
            budget,
        })
    }

    pub fn diagnostics(&self) -> &Arc<ResolverShadowDiagnostics> {
        &self.diagnostics
    }

    fn record_error(&self, input: &ResolverShadowInput, error: ResolveError) {
        self.diagnostics.record(ResolverShadowObservation {
            asset_class: input.asset_class,
            outcome: ShadowOutcome::ResolverError,
            mismatch_codes: Vec::new(),
            resolver_error_code: Some(error.code()),
        });
    }
}

impl ResolverShadowObserver for ResolverShadowRuntime {
    fn observe(&self, input: &ResolverShadowInput) {
        if !self.enabled_classes.contains(&input.asset_class) {
            return;
        }
        let Some(frozen) = input.frozen.as_ref() else {
            let error_code = if input.asset_class == ShadowAssetClass::OrbitSceneId {
                SHADOW_ORBIT_DESCRIPTOR_UNAVAILABLE
            } else {
                SHADOW_DESCRIPTOR_UNAVAILABLE
            };
            self.diagnostics.record(ResolverShadowObservation {
                asset_class: input.asset_class,
                outcome: ShadowOutcome::Ineligible,
                mismatch_codes: Vec::new(),
                resolver_error_code: Some(error_code),
            });
            return;
        };
        let cancelled = || input.cancelled;
        let context = match ShadowResolveContext::new(
            self.credential_shapes.as_ref(),
            &cancelled,
            self.clock.as_ref(),
            self.budget,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.record_error(input, error);
                return;
            }
        };
        let intent = match self.registry.plan_shadow(frozen, &context) {
            Ok(value) => value,
            Err(error) => {
                self.record_error(input, error);
                return;
            }
        };
        let Some(file_name) = intent.file_name() else {
            self.record_error(input, ResolveError::descriptor_invalid());
            return;
        };
        let destination = match input.destination_policy.destination_for(file_name) {
            Ok(value) => value,
            Err(error) => {
                self.record_error(input, error);
                return;
            }
        };
        let shadow = match TransferIntentFingerprint::from_shadow(
            &intent,
            destination,
            input.destination_policy.layout().clone(),
        ) {
            Ok(value) => value,
            Err(error) => {
                self.record_error(input, error);
                return;
            }
        };
        let mismatches = input.legacy.compare(&shadow);
        self.diagnostics.record(ResolverShadowObservation {
            asset_class: input.asset_class,
            outcome: if mismatches.is_empty() {
                ShadowOutcome::Parity
            } else {
                ShadowOutcome::Mismatch
            },
            mismatch_codes: mismatches,
            resolver_error_code: None,
        });
    }
}

pub fn run_shadow_isolated(
    observer: Option<&dyn ResolverShadowObserver>,
    input: Option<&ResolverShadowInput>,
) {
    let (Some(observer), Some(input)) = (observer, input) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| observer.observe(input)));
}

pub fn run_shadow_then_legacy<T>(
    observer: Option<&dyn ResolverShadowObserver>,
    input: Option<&ResolverShadowInput>,
    legacy: impl FnOnce() -> T,
) -> T {
    run_shadow_isolated(observer, input);
    legacy()
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn observation(asset_class: ShadowAssetClass) -> ResolverShadowObservation {
        ResolverShadowObservation {
            asset_class,
            outcome: ShadowOutcome::Parity,
            mismatch_codes: Vec::new(),
            resolver_error_code: None,
        }
    }

    #[test]
    fn diagnostics_never_waits_on_a_busy_lock_and_keeps_newest_at_capacity() {
        let diagnostics = ResolverShadowDiagnostics::new(2);
        let guard = diagnostics.items.lock().unwrap();
        diagnostics.record(observation(ShadowAssetClass::Sentinel1Standard));
        drop(guard);
        assert!(diagnostics.observations().is_empty());

        diagnostics.record(observation(ShadowAssetClass::Sentinel1Standard));
        diagnostics.record(observation(ShadowAssetClass::BurstTiff));
        diagnostics.record(observation(ShadowAssetClass::BurstXml));
        assert_eq!(
            diagnostics
                .observations()
                .into_iter()
                .map(|item| item.asset_class)
                .collect::<Vec<_>>(),
            vec![ShadowAssetClass::BurstTiff, ShadowAssetClass::BurstXml]
        );
    }

    #[test]
    fn poisoned_diagnostics_is_dropped_without_propagation() {
        let diagnostics = Arc::new(ResolverShadowDiagnostics::new(2));
        let poison = Arc::clone(&diagnostics);
        let _ = thread::spawn(move || {
            let _guard = poison.items.lock().unwrap();
            panic!("poison fixture diagnostics");
        })
        .join();
        diagnostics.record(observation(ShadowAssetClass::Sentinel1Standard));
        assert!(diagnostics.observations().is_empty());
    }
}
