use super::*;
use insar_provider::planetary_computer_asset::{
    PlanetaryComputerReviewedAssetClass, PlanetaryComputerReviewedAssetEvidence,
    MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
};
use serde_json::json;

const FIRST: &str = "S2C_MSIL2A_20260706T032511_R018_T48SYC_20260706T065916";
const FILE: &str = "T48SYC_20260706T032511_B02_10m.tif";
fn work(processed: &str) -> DownloadWork {
    let id = FIRST.replace("20260706T065916", processed);
    let class = PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog;
    let key = insar_core::provider_product_key(
        "microsoft.planetary-computer",
        class.collection_id(),
        &id,
    )
    .unwrap();
    let asset_key = insar_core::stable_asset_key(&key, "B02").unwrap();
    let source = serde_json::from_value(json!({
        "item_id":key,"provider_id":"microsoft.planetary-computer","display_name":id,
        "acquisition_datetime":"2026-07-06T03:25:11Z","footprint_geojson":null,
        "metadata":{"logical_dataset":class.logical_dataset(),"native_collection_id":class.collection_id(),
            "collection_profile_id":class.collection_profile_id(),"identity_rule_id":MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
            "provider_product_id":id,"provider_product_key":key,"stac_item_id":id,
            "source_kind":"sentinel2","product_kind":"optical_scene",
            "assets":[{"asset_key":"B02","stable_asset_key":asset_key,"role":"primary","native_roles":["data"]}]},
        "payload":{},"asset_access":null,"execution_policy":null})).unwrap();
    let href = format!("https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/48/S/YC/2026/07/06/S2C_MSIL2A_20260706T032511_N0512_R018_T48SYC_{processed}.SAFE/GRANULE/L2A_T48SYC_A009569_20260706T033711/IMG_DATA/R10m/{FILE}");
    let mut item = crate::build_planetary_computer_reviewed_cog_item(
        &source,
        class,
        "B02",
        &href,
        &PlanetaryComputerReviewedAssetEvidence {
            file_name: FILE.into(),
            expected_size_bytes: 1024,
        },
    )
    .unwrap();
    item.metadata
        .insert("delivery_product_directory".into(), json!(id));
    let task = serde_json::from_value(json!({"schema_version":1,"task_id":"batch-target-test","batch_id":"batch-test",
        "provider_id":"microsoft.planetary-computer","created_at":"2026-09-30T00:00:00Z","aoi_name":"Synthetic",
        "aoi_geojson":null,"query":{"delivery_kind":"sentinel2_l2a_analysis_band_batch"},
        "candidates":[item.clone()],"initial_selected_item_ids":[item.item_id],"output_root":"fixture-band-output","layout":"direct"})).unwrap();
    DownloadWork { task, item }
}

#[test]
fn batch_targets_isolate_processing_versions_preserve_basenames_and_restore_exactly() {
    let first = work("20260706T065916");
    let second = work("20260707T065916");
    let a = plan_work_target(&first, FILE).unwrap();
    let b = plan_work_target(&second, FILE).unwrap();
    assert_ne!(a, b);
    assert_eq!(a.file_name(), b.file_name());
    assert_eq!(a, first.task.output_root.join(FIRST).join(FILE));
    let restored = DownloadWork {
        task: serde_json::from_str(&serde_json::to_string(&second.task).unwrap()).unwrap(),
        item: second.item.clone(),
    };
    assert_eq!(plan_work_target(&restored, FILE).unwrap(), b);
    let mut single = first.clone();
    single.task.query.insert(
        "delivery_kind".into(),
        json!("sentinel2_l2a_analysis_band_set"),
    );
    single.task.output_root = single.task.output_root.join(FIRST);
    assert_eq!(
        plan_work_target(&single, FILE).unwrap(),
        a,
        "legacy single-scene output is not nested twice"
    );
}

#[test]
fn cross_provider_source_identity_does_not_replace_reviewed_mpc_product_identity() {
    let mut derived = work("20260706T065916").item;
    derived.metadata.insert(
        "source_item_id".to_owned(),
        json!("cdse-source-product-opaque-id"),
    );
    assert!(matches!(
        crate::classify_frozen_execution(
            &derived,
            &crate::AsfCollectionProfileRegistry,
            &crate::AssetExecutionPolicyRegistry,
        ),
        crate::FrozenExecution::ResolverAuthoritative(_)
    ));

    derived.metadata.insert(
        "provider_product_key".to_owned(),
        json!("tampered-provider-product-key"),
    );
    assert!(matches!(
        crate::classify_frozen_execution(
            &derived,
            &crate::AsfCollectionProfileRegistry,
            &crate::AssetExecutionPolicyRegistry,
        ),
        crate::FrozenExecution::Invalid(_)
    ));
}

#[test]
fn batch_targets_reject_traversal_foreign_identity_and_file_substitution() {
    for directory in [
        "../escape",
        "..\\escape",
        "C:\\escape",
        "other-product",
        "",
        "CON",
    ] {
        let mut item = work("20260706T065916");
        item.item
            .metadata
            .insert("delivery_product_directory".into(), json!(directory));
        assert!(plan_work_target(&item, FILE).is_err(), "{directory}");
    }
    let mut item = work("20260706T065916");
    item.item
        .asset_access
        .as_mut()
        .unwrap()
        .reference
        .collection_id = "landsat-c2-l2".into();
    assert!(plan_work_target(&item, FILE).is_err());
    assert!(plan_work_target(&work("20260706T065916"), "another.tif").is_err());
}
