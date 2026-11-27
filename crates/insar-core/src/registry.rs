use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Map};

use crate::{
    ContractError, DataProvider, DataSourceKind, ProviderCapability, ProviderMetadata,
    ProviderResult,
};

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn DataProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, provider: Arc<dyn DataProvider>) {
        self.providers
            .insert(provider.metadata().provider_id, provider);
    }

    pub fn get(&self, provider_id: &str) -> ProviderResult<Arc<dyn DataProvider>> {
        if let Some(provider) = self.providers.get(provider_id) {
            return Ok(Arc::clone(provider));
        }

        let context = Map::from_iter([("provider_id".to_string(), json!(provider_id))]);
        let error = ContractError::new("V3_PROVIDER_NOT_FOUND", "未找到数据提供器", context)
            .expect("the static provider-not-found error context is safe");
        Err(error)
    }

    pub fn metadata(&self) -> Vec<ProviderMetadata> {
        let mut metadata = self
            .providers
            .values()
            .map(|provider| provider.metadata())
            .collect::<Vec<_>>();
        metadata.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        metadata
    }

    pub fn by_source_kind(&self, source_kind: DataSourceKind) -> Vec<ProviderMetadata> {
        self.metadata()
            .into_iter()
            .filter(|metadata| metadata.source_kind == source_kind)
            .collect()
    }

    pub fn with_capability(&self, capability: ProviderCapability) -> Vec<ProviderMetadata> {
        self.metadata()
            .into_iter()
            .filter(|metadata| metadata.capabilities.contains(&capability))
            .collect()
    }
}
