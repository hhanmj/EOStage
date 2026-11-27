import type { DesktopCoreRestoreSnapshot } from "../lib/desktopShellBridge";

/** Synthetic native mutation acknowledgement; never connects to a Store or provider. */
export function opticalTaskReceiptFixture(
  request: { task_id: string; output_root: string; created_at: string },
  batchPrefix = "landsat-assets-",
  selectedCount = 1,
): Extract<DesktopCoreRestoreSnapshot, { ok: true }> {
  const sentinel2 = batchPrefix.startsWith("sentinel2-");
  const provider = ["landsat-delivery-", "landsat-bundles-batch-"].includes(batchPrefix) ? "usgs.eros.m2m"
    : batchPrefix.startsWith("sentinel2-archive-") ? "copernicus.dataspace.odata"
      : "microsoft.planetary-computer";
  const dataset = sentinel2 ? "optical.sentinel-2.l2a"
    : batchPrefix.startsWith("hls-") ? "optical.hls.s30.v2"
      : "optical.landsat.c2.l2";
  return {
    ok: true,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: {
      readiness: { schema_version: 11, ready: true }, workspace_context: null,
      region_aoi: null, region_aois: [], dem_source_identity: null, legacy_task_warnings: [],
      workbench: {
        batch_id: "source-batch", provider_id: "microsoft.planetary-computer",
        created_at: request.created_at, aoi_name: "Fixture AOI",
        query: { provider: "microsoft.planetary-computer", dataset: "optical.landsat.c2.l2",
          start: null, end: null, target_count: 2 },
        search_stats: null, candidate_count: 2, checked_count: 1, current_page_item_count: 2,
        page_index: 0, page_size: 250, revision: 7, deferred: true, reason_code: "candidate_payload_deferred",
      },
      tasks: [{
        task_id: request.task_id, batch_id: `${batchPrefix}${request.task_id}`,
        provider_id: provider, dataset_id: dataset,
        created_at: request.created_at, updated_at: request.created_at, status: "queued", revision: 0,
        selected_count: selectedCount, checked_count: selectedCount, active_count: 0,
        paused_count: 0, queued_count: selectedCount, completed_count: 0, failed_count: 0,
        skipped_count: 0, cancelled_count: 0, terminal_bytes: 0, active_bytes: 0,
        other_bytes: 0, transferred_bytes: 0, total_bytes: null, bytes_per_second: 0,
        transfer_active_millis: 0, active_item: null, output_root: request.output_root,
        layout: "direct", operation: null, actionable: true, concurrency: 2,
      }],
    },
    aoi_display: null,
  };
}
