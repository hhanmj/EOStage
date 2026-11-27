#![cfg(feature = "nisar-subset-live-acceptance")]

use base64::{engine::general_purpose::STANDARD, Engine as _};
use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationSarSearchAndFreezeRequest,
    ApplicationService, ApplicationServiceConfig,
};
use insar_core::{CatalogSearchQuery, PreviewAssetRequest};
use insar_provider::catalog::{
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROVIDER_ID, NISAR_LOGICAL_DATASET,
    NISAR_NATIVE_COLLECTION_ID, NISAR_PROVIDER_ID,
};
use insar_store::TaskStore;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

// Deliberately uses the production service and a new private Store. No user task
// is created, and only bounded catalogue pages plus reviewed browse assets move.
#[test]
#[ignore = "explicit production Sentinel-1, ALOS-2 and NISAR frozen history preview smoke"]
fn production_shared_preview_survives_workbench_switch_and_store_reopen() {
    assert_eq!(
        std::env::var("INSAR_SHARED_PREVIEW_LIVE").ok().as_deref(),
        Some("1")
    );
    let root = PathBuf::from(
        std::env::var("INSAR_SHARED_PREVIEW_LIVE_ROOT").expect("explicit evidence root"),
    );
    assert!(
        root.is_absolute() && !root.exists(),
        "fresh absolute evidence root required"
    );
    fs::create_dir_all(&root).unwrap();
    let database = root.join("tasks.sqlite3");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let geometry = json!({"type":"Polygon","coordinates":[[[110.0,30.0],[113.0,30.0],[113.0,33.0],[110.0,33.0],[110.0,30.0]]]}).as_object().unwrap().clone();
    let mut evidence = json!({"status":"running","production_service":true,"catalogue_limit_per_dataset":3,"search":[],"previews":[]});
    let evidence_path = root.join("shared-preview-redacted.json");
    let write = |value: &Value| {
        fs::write(&evidence_path, serde_json::to_vec_pretty(value).unwrap()).unwrap()
    };
    write(&evidence);
    let sentinel: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
      "request_id":"preview-live-s1-search","batch_id":"preview-live-s1","created_at":"2026-09-05T00:00:00Z",
      "aoi_name":"Preview live rectangle","aoi_geojson":geometry,"page_size":3,
      "search":{"query":{"provider":"asf","dataset":"SENTINEL-1","processing_levels":["GRD_HD"],"beam_modes":["IW"],
        "start":"2024-07-01T00:00:00Z","end":"2024-08-01T00:00:00Z","aoi_geojson":geometry,"max_results":3}}
    })).unwrap();
    let s1 = service.sar_search_and_freeze(&sentinel);
    evidence["search"].as_array_mut().unwrap().push(json!({"dataset":"sentinel-1","status":if s1.is_ok(){"frozen"}else{"failed"},"code":s1.as_ref().err().map(|error|error.code())}));
    write(&evidence);
    assert!(
        s1.is_ok(),
        "Sentinel-1 production search failed; see redacted evidence"
    );
    for (label, dataset, provider, collection) in [
        (
            "alos2",
            ALOS2_LOGICAL_DATASET,
            ALOS2_PROVIDER_ID,
            ALOS2_NATIVE_COLLECTION_ID,
        ),
        (
            "nisar",
            NISAR_LOGICAL_DATASET,
            NISAR_PROVIDER_ID,
            NISAR_NATIVE_COLLECTION_ID,
        ),
    ] {
        let catalogue_geometry = if label == "alos2" {
            json!({"type":"Polygon","coordinates":[[[130.0,30.0],[146.0,30.0],[146.0,45.0],[130.0,45.0],[130.0,30.0]]]}).as_object().unwrap().clone()
        } else {
            geometry.clone()
        };
        let search = CatalogSearchQuery {
            logical_dataset: dataset.into(),
            provider_id: provider.into(),
            native_collection_id: collection.into(),
            aoi_geojson: if label == "alos2" {
                None
            } else {
                Some(catalogue_geometry.clone())
            },
            cursor: None,
            result_limit: Some(3),
            filters: if label == "alos2" {
                json!({"datetime":{"start":"2023-03-30T00:00:00Z","end":"2023-04-02T00:00:00Z"}})
                    .as_object()
                    .unwrap()
                    .clone()
            } else {
                Default::default()
            },
        };
        let result = service.catalog_search_and_freeze(&ApplicationCatalogSearchAndFreezeRequest {
            request_id: format!("preview-live-{label}-search"),
            batch_id: format!("preview-live-{label}"),
            search,
            created_at: "2026-09-05T00:00:00Z".into(),
            aoi_name: "Preview live rectangle".into(),
            aoi_geojson: if label == "alos2" {
                None
            } else {
                Some(catalogue_geometry.clone())
            },
            workbench_page_size: 3,
        });
        evidence["search"].as_array_mut().unwrap().push(json!({"dataset":label,"status":if result.is_ok(){"frozen"}else{"failed"},"code":result.as_ref().err().map(|error|error.code())}));
        write(&evidence);
        assert!(
            result.is_ok(),
            "production catalogue search failed; see redacted evidence"
        );
    }
    service.shutdown().unwrap();
    let store = TaskStore::open_existing(&database).unwrap();
    let batches = ["s1", "alos2", "nisar"].map(|label| {
        (
            label,
            store
                .load_search_batch(&format!("preview-live-{label}"))
                .unwrap()
                .unwrap(),
        )
    });
    drop(store);
    let reopened =
        ApplicationService::new(ApplicationServiceConfig::existing_business_store(&database));
    let mut unexpected = false;
    for (label, batch) in batches {
        if batch.candidates.is_empty() {
            unexpected = true;
            evidence["previews"]
                .as_array_mut()
                .unwrap()
                .push(json!({"dataset":label,"batch_id":batch.batch_id,"status":"no_candidates"}));
            write(&evidence);
            continue;
        }
        // S1 and ALOS-2 are now historical: the mutable workbench is NISAR.
        let item = &batch.candidates[0];
        let result = reopened.preview_asset(PreviewAssetRequest {
            request_id: format!("preview-live-{label}-reopened"),
            batch_id: batch.batch_id.clone(),
            item_id: item.item_id.clone(),
        });
        let observation = match result {
            Ok(preview) => {
                assert_eq!(preview.product_key, item.item_id);
                if label == "nisar" {
                    verify_nisar_preview_after_task_creation(
                        &reopened, &database, &root, &batch, item, &preview,
                    );
                }
                let bytes = STANDARD
                    .decode(preview.data_url.split_once(',').unwrap().1)
                    .unwrap();
                assert_eq!(bytes.len() as u64, preview.byte_length);
                let extension = if preview.media_type == "image/png" {
                    "png"
                } else if preview.media_type == "image/webp" {
                    "webp"
                } else {
                    "jpg"
                };
                let image_name = format!("{label}-preview.{extension}");
                fs::write(root.join(&image_name), bytes).unwrap();
                json!({"dataset":label,"batch_id":batch.batch_id,"item_id":item.item_id,"status":"available","source_asset_key":preview.source_asset_key,"media_type":preview.media_type,"byte_length":preview.byte_length,"image":image_name,"reopened":true})
            }
            Err(error) => {
                let code = error.code();
                if label != "alos2" || code != "provider_no_preview" {
                    unexpected = true;
                }
                json!({"dataset":label,"batch_id":batch.batch_id,"item_id":item.item_id,"status":if code=="provider_no_preview"{"unavailable"}else{"failed"},"code":code,"reopened":true})
            }
        };
        evidence["previews"]
            .as_array_mut()
            .unwrap()
            .push(observation);
        write(&evidence);
    }
    reopened.shutdown().unwrap();
    evidence["status"] = json!(if unexpected { "failed" } else { "pass" });
    write(&evidence);
    assert!(
        !unexpected,
        "production preview outcome failed; see redacted evidence"
    );
}

fn verify_nisar_preview_after_task_creation(
    service: &ApplicationService,
    database: &std::path::Path,
    root: &std::path::Path,
    batch: &insar_core::FrozenSearchBatch,
    item: &insar_core::SnapshotItem,
    baseline: &insar_core::ResolvedPreviewAsset,
) {
    use insar_application_service::{
        NisarConfirmSubsetBatchItemV2, NisarConfirmSubsetBatchTaskV2Request,
        NisarPrepareSubsetBatchV2Request,
    };
    use insar_core::{DownloadLayout, StoreCreateTaskRequest};
    let revision = service
        .core_restore(false)
        .unwrap()
        .core
        .workbench
        .unwrap()
        .revision;
    // Freeze complete-product membership paused so this preview-only smoke does
    // not transfer the multi-GB science product. The ordinary Store owner is used.
    let mut store = TaskStore::open_existing(database).unwrap();
    let mut complete_item = item.clone();
    complete_item
        .metadata
        .insert("source_item_id".into(), json!(item.item_id));
    let task = store
        .create_paused_task_with_explicit_items(
            &StoreCreateTaskRequest {
                task_id: "preview-live-nisar-complete-task".into(),
                batch_id: batch.batch_id.clone(),
                selected_item_ids: vec![item.item_id.clone()],
                checked_item_ids: vec![item.item_id.clone()],
                page_index: 0,
                page_size: 3,
                output_root: root.join("complete-output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-09-05T00:00:01Z".into(),
            },
            revision,
            std::slice::from_ref(&item.item_id),
            &[complete_item],
        )
        .unwrap();
    assert!(task.runtime.task_paused);
    assert!(task.runtime.queued_item_ids.is_empty());
    drop(store);
    let verify = |stage: &str| {
        let current = service
            .preview_asset(PreviewAssetRequest {
                request_id: format!("preview-live-nisar-{stage}"),
                batch_id: batch.batch_id.clone(),
                item_id: item.item_id.clone(),
            })
            .unwrap_or_else(|error| panic!("preview {stage} failed: {}", error.code()));
        assert_eq!(current.product_key, baseline.product_key);
        assert_eq!(current.source_asset_key, baseline.source_asset_key);
        assert_eq!(current.data_url, baseline.data_url);
    };
    verify("after-complete-task");
    let prepared = service
        .prepare_nisar_subset_batch_v2(&NisarPrepareSubsetBatchV2Request {
            source_batch_id: batch.batch_id.clone(),
            source_item_ids: vec![item.item_id.clone()],
            expected_workbench_revision: revision,
            subset_draft_geometry: batch.aoi_geojson.clone().unwrap(),
            selected_frequency: "A".into(),
            selected_polarizations: vec!["HH".into()],
        })
        .unwrap_or_else(|error| panic!("preview subset prepare failed: {}", error.code()));
    service
        .confirm_nisar_subset_batch_task_v2(&NisarConfirmSubsetBatchTaskV2Request {
            source_batch_id: batch.batch_id.clone(),
            expected_workbench_revision: revision,
            items: prepared
                .intents
                .into_iter()
                .map(|prepared| NisarConfirmSubsetBatchItemV2 {
                    source_item_id: prepared.source_item_id,
                    prepared_intent: prepared.intent,
                })
                .collect(),
            task_id: "preview-live-nisar-subset-task".into(),
            output_root: root.join("subset-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-09-05T00:00:02Z".into(),
        })
        .unwrap_or_else(|error| panic!("preview subset confirmation failed: {}", error.code()));
    verify("after-subset-task");
    let store = TaskStore::open_existing(database).unwrap();
    assert_eq!(
        store.load_search_batch(&batch.batch_id).unwrap().unwrap(),
        *batch
    );
    fs::write(root.join("nisar-preview-task-stability.json"),serde_json::to_vec_pretty(&json!({
        "status":"pass","complete_task":"paused","subset_task":"paused","after_complete_task_preview":"same_asset_same_bytes",
        "after_subset_task_preview":"same_asset_same_bytes","source_batch_unchanged":true,"science_transfer_started":false,
    })).unwrap()).unwrap();
}
