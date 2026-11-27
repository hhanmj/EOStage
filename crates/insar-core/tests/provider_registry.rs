use std::sync::{Arc, Mutex};

use insar_core::{
    ContractError, DataProvider, DataSourceKind, ProviderCapability, ProviderMetadata,
    ProviderPlan, ProviderProduct, ProviderQuery, ProviderRegistry, ProviderResult,
};
use serde_json::{json, Map, Value};

#[test]
fn error_constructor_accepts_safe_context_and_has_safe_display() {
    let context = Map::from_iter([
        ("provider_id".to_string(), json!("fixture.provider")),
        ("details".to_string(), json!({"attempt": 2})),
    ]);

    let error = ContractError::new("V3_FIXTURE", "Fixture failed", context).unwrap();

    assert_eq!(error.code, "V3_FIXTURE");
    assert_eq!(error.user_message, "Fixture failed");
    assert_eq!(error.debug_context["provider_id"], "fixture.provider");
    assert_eq!(error.to_string(), "[V3_FIXTURE] Fixture failed");
    assert!(!error.to_string().contains("fixture.provider"));
}

#[test]
fn error_constructor_rejects_nested_sensitive_context_case_insensitively() {
    let context = Map::from_iter([(
        "details".to_string(),
        json!([{"AuThOrIzAtIoN": "must-not-leak"}]),
    )]);

    let message = ContractError::new("V3_FIXTURE", "Fixture failed", context)
        .expect_err("sensitive context must be rejected");

    assert!(message.contains("AuThOrIzAtIoN"));
    assert!(!message.contains("must-not-leak"));
}

#[test]
fn error_type_is_usable_through_std_error() {
    let error = ContractError::new("V3_FIXTURE", "Fixture failed", Map::new()).unwrap();
    let source: &dyn std::error::Error = &error;

    assert_eq!(source.to_string(), "[V3_FIXTURE] Fixture failed");
    assert_eq!(Value::Object(error.debug_context), json!({}));
}

struct FixtureProvider {
    metadata: ProviderMetadata,
    observed_product_count: Mutex<Option<Option<usize>>>,
}

impl FixtureProvider {
    fn new() -> Self {
        Self::with_metadata(
            "fixture.provider",
            DataSourceKind::Sentinel1,
            vec![ProviderCapability::Search, ProviderCapability::Plan],
        )
    }

    fn with_metadata(
        provider_id: &str,
        source_kind: DataSourceKind,
        capabilities: Vec<ProviderCapability>,
    ) -> Self {
        Self {
            metadata: ProviderMetadata {
                provider_id: provider_id.to_string(),
                display_name: format!("Fixture {provider_id}"),
                source_kind,
                version: "0.1.0".to_string(),
                capabilities,
            },
            observed_product_count: Mutex::new(None),
        }
    }
}

impl DataProvider for FixtureProvider {
    fn metadata(&self) -> ProviderMetadata {
        self.metadata.clone()
    }

    fn search(&self, _query: &ProviderQuery) -> ProviderResult<Vec<ProviderProduct>> {
        Ok(Vec::new())
    }

    fn plan(
        &self,
        _query: &ProviderQuery,
        products: Option<&[ProviderProduct]>,
    ) -> ProviderResult<ProviderPlan> {
        *self.observed_product_count.lock().unwrap() = Some(products.map(<[_]>::len));
        Ok(serde_json::from_value(json!({
            "provider_id": "fixture.provider",
            "source_kind": "sentinel1",
            "output_root": "E:/InSAR/fixture"
        }))
        .unwrap())
    }
}

#[test]
fn registry_replaces_by_id_and_filters_in_deterministic_order() {
    let mut registry = ProviderRegistry::new();
    registry.register(Arc::new(FixtureProvider::with_metadata(
        "z.dem",
        DataSourceKind::Dem,
        vec![ProviderCapability::Plan],
    )));
    registry.register(Arc::new(FixtureProvider::with_metadata(
        "a.sar",
        DataSourceKind::Sentinel1,
        vec![ProviderCapability::Search],
    )));
    registry.register(Arc::new(FixtureProvider::with_metadata(
        "z.dem",
        DataSourceKind::Dem,
        vec![ProviderCapability::Search, ProviderCapability::Plan],
    )));

    assert_eq!(
        registry
            .metadata()
            .into_iter()
            .map(|item| item.provider_id)
            .collect::<Vec<_>>(),
        ["a.sar", "z.dem"]
    );
    assert_eq!(
        registry.get("z.dem").unwrap().metadata().capabilities.len(),
        2
    );
    assert_eq!(
        registry.by_source_kind(DataSourceKind::Dem)[0].provider_id,
        "z.dem"
    );
    assert_eq!(
        registry
            .with_capability(ProviderCapability::Search)
            .into_iter()
            .map(|item| item.provider_id)
            .collect::<Vec<_>>(),
        ["a.sar", "z.dem"]
    );
    assert!(registry
        .with_capability(ProviderCapability::Download)
        .is_empty());
}

#[test]
fn registry_unknown_lookup_returns_structured_error() {
    let error = match ProviderRegistry::new().get("missing.provider") {
        Ok(_) => panic!("unknown provider lookup must fail"),
        Err(error) => error,
    };

    assert_eq!(error.code, "V3_PROVIDER_NOT_FOUND");
    assert_eq!(error.user_message, "未找到数据提供器");
    assert_eq!(
        error.debug_context,
        Map::from_iter([("provider_id".to_string(), json!("missing.provider"),)])
    );
}

#[test]
fn provider_trait_preserves_none_vs_explicit_empty_products() {
    let provider = FixtureProvider::new();
    let query: ProviderQuery = serde_json::from_value(json!({})).unwrap();

    assert_eq!(provider.metadata().provider_id, "fixture.provider");
    assert!(provider.search(&query).unwrap().is_empty());

    provider.plan(&query, None).unwrap();
    assert_eq!(*provider.observed_product_count.lock().unwrap(), Some(None));

    provider.plan(&query, Some(&[])).unwrap();
    assert_eq!(
        *provider.observed_product_count.lock().unwrap(),
        Some(Some(0))
    );
}
