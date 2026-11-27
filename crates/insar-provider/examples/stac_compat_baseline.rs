//! Test-only JSON bridge for comparing incompatible Cargo dependency graphs.
//! No networking, credentials, Store, production route or application startup.
use std::io::{self, Read};

use insar_core::{
    AssetRole, CatalogDeliveryModel, CatalogSearchQuery, DataSourceKind, ProductKind,
    ProviderResult,
};
use insar_provider::catalog::{
    build_stac_search_request, map_stac_complete_search_response, map_stac_item,
    StacAssetRoleBinding, StacCollectionBinding, StacQueryOperator, StacQueryValueKind,
    StacQueryableBinding, StacRouteProfile,
};
use serde_json::{json, Value};

fn route() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: "fixture.stac".into(),
        endpoint: "https://catalog.example.test/stac/v1/".into(),
        search_path: "search".into(),
        page_cap: 100,
        accepted_stac_versions: vec!["1.0.0".into(), "1.1.0".into()],
    }
}

fn binding() -> StacCollectionBinding {
    StacCollectionBinding {
        logical_dataset: "optical.fixture.l2".into(),
        provider_id: "fixture.stac".into(),
        native_collection_id: "fixture-l2".into(),
        collection_profile_id: "fixture.stac.optical.l2.v1".into(),
        identity_rule_id: "fixture.stac.item-id.v1".into(),
        filename_rule_id: "fixture.stac.asset-filename.v1".into(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        source_kind: DataSourceKind::Sentinel2,
        product_kind: ProductKind::OpticalScene,
        production_enabled: false,
        product_id_property: Some("scene:id".into()),
        display_name_property: Some("title".into()),
        queryables: vec![StacQueryableBinding {
            sceneport_filter_key: "cloud_cover_lte".into(),
            stac_property: "eo:cloud_cover".into(),
            operator: StacQueryOperator::Lte,
            value_kind: StacQueryValueKind::Number,
        }],
        asset_role_bindings: vec![StacAssetRoleBinding {
            stac_asset_key: "data".into(),
            role: AssetRole::Primary,
            required: true,
            allowed_hosts: vec!["data.example.test".into()],
        }],
    }
}

fn encode<T: serde::Serialize>(result: ProviderResult<T>) -> Value {
    match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"code":error.code}),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    io::stdin().take(1024 * 1024 + 1).read_to_end(&mut input)?;
    if input.len() > 1024 * 1024 {
        return Err("fixture input exceeds one MiB".into());
    }
    let request: Value = serde_json::from_slice(&input)?;
    let output = match request["operation"].as_str() {
        Some("map") => encode(map_stac_item(&route(), &binding(), &request["raw"])),
        Some("query") => {
            let query: CatalogSearchQuery = serde_json::from_value(request["query"].clone())?;
            encode(build_stac_search_request(&route(), &binding(), &query).map(|r| r.body))
        }
        Some("response") => {
            let query: CatalogSearchQuery = serde_json::from_value(request["query"].clone())?;
            encode(map_stac_complete_search_response(
                &route(),
                &binding(),
                &query,
                &request["raw"],
            ))
        }
        _ => return Err("unsupported fixture operation".into()),
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
