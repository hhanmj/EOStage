use super::*;
use insar_core::{
    stable_asset_key, AssetExecutionAuthority, AssetExecutionPolicy, FrozenTaskSnapshot,
    SnapshotItem,
};
use serde_json::json;

// Issue #15: ASF names Burst assets by burst index inside the parent SAFE
// (`.../IW3/VV/0.tiff`, `.../IW3/VV/0.xml`), so different bursts of one batch
// share a basename. Real Utrecht batch (2026-10-21): five bursts froze
// "0.tiff", eight froze "8.tiff".
fn burst_asset_work(native_product_id: &str, file_name: &str, output_root: &str) -> DownloadWork {
    let source_item_id = format!("ASF::SLC-BURST::{native_product_id}");
    let stable_key =
        stable_asset_key(&source_item_id, &format!("{native_product_id}:{file_name}")).unwrap();
    let item: SnapshotItem = serde_json::from_value(json!({
        "item_id": stable_key,
        "provider_id": "asf.catalog",
        "display_name": file_name,
        "acquisition_datetime": "2026-10-12T05:49:48Z",
        "footprint_geojson": null,
        "metadata": {
            "provider": "asf.catalog",
            "dataset": "SLC-BURST",
            "product_family": "BURST",
            "processing_level": "L1",
            "product_id": native_product_id,
            "scene_id": native_product_id,
            "source_item_id": source_item_id,
            "asset_role": "primary",
            "asset_basename": file_name,
        },
        "payload": {"burst_asset": true, "file_name": file_name},
        "asset_access": null,
        "execution_policy": null,
    }))
    .unwrap();
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": "burst-target-test",
        "batch_id": "burst-batch",
        "provider_id": "asf.catalog",
        "created_at": "2026-10-21T00:00:00Z",
        "aoi_name": "Utrecht",
        "aoi_geojson": null,
        "query": {"dataset": "SLC-BURST"},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id],
        "output_root": output_root,
        "layout": "direct",
    }))
    .unwrap();
    let mut work = DownloadWork { task, item };
    work.item.execution_policy = Some(AssetExecutionPolicy {
        authority: AssetExecutionAuthority::ResolverAuthoritative,
        policy_id: crate::ASF_BURST_ASSET_EXECUTION_POLICY_ID.to_owned(),
        collection_profile_id: Some(crate::ASF_BURST_COLLECTION_PROFILE_ID.to_owned()),
    });
    work
}

#[test]
fn burst_assets_with_the_same_official_basename_get_distinct_planned_targets() {
    let root = "fixture-burst-output";
    let first = burst_asset_work(
        "S1_078090_IW3_20261006T054934_VV_A25C-BURST",
        "0.tiff",
        root,
    );
    let second = burst_asset_work(
        "S1_078090_IW3_20261012T054947_VV_F6D9-BURST",
        "0.tiff",
        root,
    );
    let a = plan_work_target(&first, "0.tiff").unwrap();
    let b = plan_work_target(&second, "0.tiff").unwrap();
    assert_ne!(a, b, "two bursts must never share one planned file");
    assert_eq!(
        a,
        std::path::Path::new(root)
            .join("S1_078090_IW3_20261006T054934_VV_A25C-BURST")
            .join("0.tiff"),
        "the burst product ID names the folder; the official basename is kept"
    );
    assert_eq!(
        b,
        std::path::Path::new(root)
            .join("S1_078090_IW3_20261012T054947_VV_F6D9-BURST")
            .join("0.tiff")
    );
}

#[test]
fn a_burst_tiff_and_its_xml_land_in_the_same_burst_folder() {
    let root = "fixture-burst-output";
    let id = "S1_078090_IW3_20261012T054947_VV_F6D9-BURST";
    let tiff = plan_work_target(&burst_asset_work(id, "0.tiff", root), "0.tiff").unwrap();
    let xml = plan_work_target(&burst_asset_work(id, "0.xml", root), "0.xml").unwrap();
    assert_eq!(tiff.parent(), xml.parent());
    assert_eq!(
        xml.file_name().and_then(|name| name.to_str()),
        Some("0.xml")
    );
}

#[test]
fn a_burst_product_id_that_is_not_a_safe_folder_name_is_refused() {
    let root = "fixture-burst-output";
    for unsafe_id in [
        "..",
        "a/b",
        "a\\b",
        "CON",
        "S1 BURST",
        "S1.BURST",
        "S1_078090_IW3",
        "",
    ] {
        let work = burst_asset_work(unsafe_id, "0.tiff", root);
        let error = plan_work_target(&work, "0.tiff").unwrap_err();
        assert_eq!(
            error.code(),
            "V3_INVALID_TARGET_PATH",
            "{unsafe_id:?} must not become a folder"
        );
    }
}

#[test]
fn a_burst_folder_name_may_use_up_to_128_characters() {
    let root = "fixture-burst-output";
    let longest = format!("{}-BURST", "S".repeat(128 - "-BURST".len()));
    let too_long = format!("S{longest}");
    assert!(plan_work_target(&burst_asset_work(&longest, "0.tiff", root), "0.tiff").is_ok());
    assert_eq!(
        plan_work_target(&burst_asset_work(&too_long, "0.tiff", root), "0.tiff")
            .unwrap_err()
            .code(),
        "V3_INVALID_TARGET_PATH"
    );
}

#[test]
fn a_burst_asset_outside_the_direct_layout_is_refused() {
    let mut work = burst_asset_work(
        "S1_078090_IW3_20261012T054947_VV_F6D9-BURST",
        "0.tiff",
        "fixture-burst-output",
    );
    for layout in [
        DownloadLayout::Sentinel1Slc,
        DownloadLayout::SarDatasetProcessing,
    ] {
        work.task.layout = layout;
        assert_eq!(
            plan_work_target(&work, "0.tiff").unwrap_err().code(),
            "V3_INVALID_TARGET_PATH"
        );
    }
}
