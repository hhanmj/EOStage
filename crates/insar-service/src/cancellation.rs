use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use insar_core::{ContractError, ProviderResult};
use serde_json::{Map, Value};

#[derive(Default)]
pub struct CancellationRegistry {
    requests: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl CancellationRegistry {
    /// Registers a request only when its identifier is not already in flight.
    ///
    /// Callers that receive request IDs across an external boundary must use
    /// this variant so a duplicate cannot replace the flag owned by an earlier
    /// operation and make that operation uncancellable.
    pub fn try_register(&self, request_id: &str) -> Option<Arc<AtomicBool>> {
        let mut requests = self
            .requests
            .lock()
            .expect("cancellation registry lock poisoned");
        if requests.contains_key(request_id) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        requests.insert(request_id.to_string(), Arc::clone(&flag));
        Some(flag)
    }

    pub fn register(&self, request_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.requests
            .lock()
            .expect("cancellation registry lock poisoned")
            .insert(request_id.to_string(), Arc::clone(&flag));
        flag
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        let flag = self
            .requests
            .lock()
            .expect("cancellation registry lock poisoned")
            .get(request_id)
            .cloned();
        if let Some(flag) = flag {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub fn finish(&self, request_id: &str) {
        self.requests
            .lock()
            .expect("cancellation registry lock poisoned")
            .remove(request_id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogueRequestState {
    Active,
    Cancelled,
    Publishing,
}

#[derive(Default)]
pub struct CatalogueRequestRegistry {
    requests: Mutex<HashMap<String, CatalogueRequestState>>,
}

pub struct CatalogueRequestLease {
    registry: Arc<CatalogueRequestRegistry>,
    request_id: String,
}

fn catalogue_error(code: &str, message: &str, reason: &str) -> ContractError {
    ContractError::new(
        code,
        message,
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    )
    .expect("bounded catalogue cancellation context is safe")
}

impl CatalogueRequestRegistry {
    pub fn has_active_requests(&self) -> bool {
        !self
            .requests
            .lock()
            .expect("catalogue cancellation registry lock poisoned")
            .is_empty()
    }

    pub fn register(self: &Arc<Self>, request_id: &str) -> ProviderResult<CatalogueRequestLease> {
        let mut requests = self
            .requests
            .lock()
            .expect("catalogue cancellation registry lock poisoned");
        if requests.contains_key(request_id) {
            return Err(catalogue_error(
                "CATALOGUE_REQUEST_DUPLICATE",
                "目录请求已在运行",
                "duplicate_request_id",
            ));
        }
        requests.insert(request_id.to_string(), CatalogueRequestState::Active);
        Ok(CatalogueRequestLease {
            registry: Arc::clone(self),
            request_id: request_id.to_string(),
        })
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        let mut requests = self
            .requests
            .lock()
            .expect("catalogue cancellation registry lock poisoned");
        match requests.get_mut(request_id) {
            Some(state @ CatalogueRequestState::Active) => {
                *state = CatalogueRequestState::Cancelled;
                true
            }
            Some(CatalogueRequestState::Cancelled | CatalogueRequestState::Publishing) | None => {
                false
            }
        }
    }

    fn is_cancelled(&self, request_id: &str) -> bool {
        self.requests
            .lock()
            .expect("catalogue cancellation registry lock poisoned")
            .get(request_id)
            .is_none_or(|state| *state == CatalogueRequestState::Cancelled)
    }

    fn begin_publication(&self, request_id: &str) -> ProviderResult<()> {
        let mut requests = self
            .requests
            .lock()
            .expect("catalogue cancellation registry lock poisoned");
        match requests.get_mut(request_id) {
            Some(state @ CatalogueRequestState::Active) => {
                *state = CatalogueRequestState::Publishing;
                Ok(())
            }
            Some(CatalogueRequestState::Cancelled) => Err(catalogue_error(
                "CATALOGUE_REQUEST_CANCELLED",
                "目录请求已取消",
                "before_publication",
            )),
            Some(CatalogueRequestState::Publishing) | None => Err(catalogue_error(
                "CATALOGUE_REQUEST_STATE_INVALID",
                "目录请求发布状态无效",
                "publication_state",
            )),
        }
    }

    fn finish(&self, request_id: &str) {
        self.requests
            .lock()
            .expect("catalogue cancellation registry lock poisoned")
            .remove(request_id);
    }
}

impl CatalogueRequestLease {
    pub fn is_cancelled(&self) -> bool {
        self.registry.is_cancelled(&self.request_id)
    }

    pub fn begin_publication(&self) -> ProviderResult<()> {
        self.registry.begin_publication(&self.request_id)
    }
}

impl Drop for CatalogueRequestLease {
    fn drop(&mut self) {
        self.registry.finish(&self.request_id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::CancellationRegistry;

    #[test]
    fn checked_registration_never_replaces_an_in_flight_cancellation_flag() {
        let registry = CancellationRegistry::default();
        let first = registry
            .try_register("quicklook-request")
            .expect("first request registers");

        assert!(registry.try_register("quicklook-request").is_none());
        assert!(registry.cancel("quicklook-request"));
        assert!(first.load(Ordering::SeqCst));

        registry.finish("quicklook-request");
        assert!(!registry.cancel("quicklook-request"));
    }
}
