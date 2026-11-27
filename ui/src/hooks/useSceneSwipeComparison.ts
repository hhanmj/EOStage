/**
 * useSceneSwipeComparison.ts
 * Shared dual-scene map layer swipe comparison hook (`左图 / 右图` 卷帘对比),
 * reused between `ManualWorkbench` and `DesktopAgentWorkbench` so both maps
 * have identical swipe capability across Sentinel-2, Landsat, HLS, SAR, and GEE layers.
 */

import { useCallback, useState } from "react";
import { getSentinel2MapPreview, type Bbox, type SceneRow } from "@/lib/bridge";
import {
  isSentinel2L2aCatalogProduct,
  sentinel2MapTilePreview,
  sentinel2ResolvedMapTilePreview,
} from "@/lib/sentinel2Catalog";
import { landsatMapTilePreview } from "@/lib/landsatCatalog";
import { hlsMapTilePreview } from "@/lib/hlsCatalog";
import { opticalQuicklookItemId } from "@/shell/api/opticalQuicklookLogic";
import { geeCloudAnalyze } from "@/lib/geeCloudSuiteClient";
import type { MapSwipeLayerSpec } from "@/components/WorkbenchMap";

export interface UseSceneSwipeComparisonOptions {
  workingAoi: Bbox | null;
  defaultBatchId?: string | null;
  resolveBatchIdForScene?: (scene: SceneRow) => string | null | undefined;
}

export function useSceneSwipeComparison({
  workingAoi,
  defaultBatchId,
  resolveBatchIdForScene,
}: UseSceneSwipeComparisonOptions) {
  const [swipeLeftLayer, setSwipeLeftLayer] = useState<MapSwipeLayerSpec | null>(null);
  const [swipeRightLayer, setSwipeRightLayer] = useState<MapSwipeLayerSpec | null>(null);
  const [swipePosition, setSwipePosition] = useState<number>(50);

  const sceneToSwipeSpec = useCallback(
    (scene: SceneRow): MapSwipeLayerSpec | null => {
      const bbox = scene.footprint_bbox ?? workingAoi;
      if (!bbox) return null;
      const meta = (
        scene.native_metadata &&
        typeof scene.native_metadata === "object" &&
        !Array.isArray(scene.native_metadata)
          ? scene.native_metadata
          : {}
      ) as Record<string, unknown>;
      const tileUrlTemplate =
        (typeof meta.tile_url_template === "string" && meta.tile_url_template.trim()) ||
        (scene.sar_product ? sentinel2MapTilePreview(scene.sar_product)?.urlTemplate : null) ||
        (scene.sar_product ? landsatMapTilePreview(scene.sar_product)?.urlTemplate : null) ||
        hlsMapTilePreview(scene)?.urlTemplate ||
        undefined;
      const previewAssetUrl = scene.assets?.find(
        (a) =>
          a.role === "preview" ||
          a.role === "browse" ||
          /thumbnail|quicklook|browse/i.test(a.asset_key || "")
      )?.url;
      const previewUrl =
        (typeof meta.thumbnail_url === "string" && meta.thumbnail_url.trim()) ||
        (typeof meta.quicklook_url === "string" && meta.quicklook_url.trim()) ||
        (typeof previewAssetUrl === "string" && previewAssetUrl.trim()) ||
        undefined;
      return {
        id: scene.scene_id,
        title: scene.file_name || scene.scene_id,
        acquisitionDate: scene.acquisition_datetime?.slice(0, 10) || undefined,
        bbox,
        tileUrlTemplate,
        previewDataUrl: previewUrl,
      };
    },
    [workingAoi]
  );

  const resolveSentinel2SwipeTileAsync = useCallback(
    (
      scene: SceneRow,
      setLayer: React.Dispatch<React.SetStateAction<MapSwipeLayerSpec | null>>
    ) => {
      const product = scene.sar_product;
      const bId = (resolveBatchIdForScene?.(scene) ?? defaultBatchId ?? "").trim();
      if (!product || !bId || !isSentinel2L2aCatalogProduct(product)) return;
      const itemId = opticalQuicklookItemId(scene);
      void getSentinel2MapPreview({
        request_id: `swipe-s2-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        batch_id: bId,
        item_id: itemId,
      })
        .then((res) => {
          if (!res.ok || res.preview.product_key !== itemId) return;
          const resolved = sentinel2ResolvedMapTilePreview(product, res.preview.map_item_id);
          if (!resolved?.urlTemplate) return;
          setLayer((curr) =>
            curr && curr.id === scene.scene_id
              ? { ...curr, tileUrlTemplate: resolved.urlTemplate }
              : curr
          );
        })
        .catch(() => {
          // Ignore transient preview lookup failure; footprint/HUD remain intact.
        });
    },
    [defaultBatchId, resolveBatchIdForScene]
  );

  const resolveSwipeTileAsync = useCallback(
    (
      scene: SceneRow,
      spec: MapSwipeLayerSpec,
      setLayer: React.Dispatch<React.SetStateAction<MapSwipeLayerSpec | null>>
    ) => {
      const isGeeComposite = scene.scene_id.startsWith("gee-composite::");
      const isGeeScene = scene.scene_id.startsWith("gee-scene::");
      if (isGeeComposite || isGeeScene) {
        const targetAssetId = scene.scene_id.slice(
          isGeeComposite ? "gee-composite::".length : "gee-scene::".length
        );
        const bboxTuple: [number, number, number, number] = Array.isArray(spec.bbox)
          ? spec.bbox
          : [spec.bbox.west, spec.bbox.south, spec.bbox.east, spec.bbox.north];
        const meta = (scene.native_metadata || {}) as Record<string, unknown>;
        const reqStart = typeof meta.start_date === "string" ? meta.start_date : undefined;
        const reqEnd = typeof meta.end_date === "string" ? meta.end_date : undefined;
        const maxCloud = typeof meta.max_cloud === "number" ? meta.max_cloud : undefined;
        void geeCloudAnalyze({
          asset_id: targetAssetId,
          operation: "composite",
          reducer: "median",
          bbox: bboxTuple,
          ...(isGeeComposite && reqStart ? { start_date: reqStart } : {}),
          ...(isGeeComposite && reqEnd ? { end_date: reqEnd } : {}),
          ...(isGeeComposite && typeof maxCloud === "number" ? { max_cloud: maxCloud } : {}),
        })
          .then((res) => {
            if (!res) return;
            setLayer((curr) =>
              curr && curr.id === scene.scene_id
                ? {
                    ...curr,
                    tileUrlTemplate: res.tile_url_template ?? curr.tileUrlTemplate,
                    previewDataUrl: res.preview_data_url ?? curr.previewDataUrl,
                  }
                : curr
            );
          })
          .catch(() => {
            // Ignore transient GEE swipe preview resolution failure; footprint remains intact.
          });
        return;
      }
      resolveSentinel2SwipeTileAsync(scene, setLayer);
    },
    [resolveSentinel2SwipeTileAsync]
  );

  const handlePinSwipeLeft = useCallback(
    (scene: SceneRow) => {
      if (swipeLeftLayer?.id === scene.scene_id) {
        setSwipeLeftLayer(null);
        return;
      }
      const spec = sceneToSwipeSpec(scene);
      setSwipeLeftLayer(spec);
      if (spec && !spec.tileUrlTemplate) {
        resolveSwipeTileAsync(scene, spec, setSwipeLeftLayer);
      }
    },
    [swipeLeftLayer?.id, sceneToSwipeSpec, resolveSwipeTileAsync]
  );

  const handlePinSwipeRight = useCallback(
    (scene: SceneRow) => {
      if (swipeRightLayer?.id === scene.scene_id) {
        setSwipeRightLayer(null);
        return;
      }
      const spec = sceneToSwipeSpec(scene);
      setSwipeRightLayer(spec);
      if (spec && !spec.tileUrlTemplate) {
        resolveSwipeTileAsync(scene, spec, setSwipeRightLayer);
      }
    },
    [swipeRightLayer?.id, sceneToSwipeSpec, resolveSwipeTileAsync]
  );

  const clearSwipe = useCallback(() => {
    setSwipeLeftLayer(null);
    setSwipeRightLayer(null);
    setSwipePosition(50);
  }, []);

  return {
    swipeLeftLayer,
    swipeRightLayer,
    swipePosition,
    setSwipePosition,
    clearSwipe,
    handlePinSwipeLeft,
    handlePinSwipeRight,
  };
}
