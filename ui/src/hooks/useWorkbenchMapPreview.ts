/**
 * Shared map preview, quicklook/tile resolution, identity mapping, and same-track stack hook
 * shared between `ManualWorkbench` and `DesktopAgentWorkbench` (Slice D).
 *
 * Preserves single state ownership: takes authoritative `scenes`, `selectedSceneId`, `batchId`,
 * and `onSelectScene` without creating any second scene or preview cache.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { landsatMapTilePreview } from "../lib/landsatCatalog";
import {
  isSentinel2L2aCatalogProduct,
  sentinel2MapTilePreview,
  sentinel2ResolvedMapTilePreview,
  type Sentinel2MapTilePreview,
} from "../lib/sentinel2Catalog";
import { hlsMapTilePreview } from "../lib/hlsCatalog";
import { sceneFootprintRenderModel } from "../lib/sceneFootprint";
import { getPreviewAsset, getSentinel2MapPreview, type Bbox, type SceneRow } from "../lib/bridge";
import {
  isAllowedByCsp,
  isProxyablePreviewUrl,
  thumbnailUrlFor,
} from "../shell/api/thumbnailLogic";
import { sharedGeodataPreviewQueue } from "../shell/api/geodataPreviewLoader";
import { opticalQuicklookCacheKey, opticalQuicklookItemId, opticalQuicklookPlan, type OpticalQuicklookRequest } from "../shell/api/opticalQuicklookLogic";
import { loadOpticalQuicklook, sharedOpticalQuicklookQueue } from "../shell/api/opticalQuicklookLoader";
import { buildIdentityIndex, stackFor } from "../shell/api/mapLinkLogic";
import {
  bhoonidhiThumbnailAsset,
  isBhoonidhiSsarCatalogProduct,
} from "../lib/bhoonidhiSsarCatalog";
import { emitTopToast } from "../components/ui/TopCenterToast";

export function directPreviewImageUrlFor(
  scene: SceneRow | null | undefined,
): string | null {
  if (!scene) return null;
  const meta = scene.native_metadata as Record<string, unknown> | undefined;
  const metaDataUrl = meta?.preview_data_url;
  if (typeof metaDataUrl === "string" && metaDataUrl.trim() && isAllowedByCsp(metaDataUrl.trim())) {
    return metaDataUrl.trim();
  }
  const directThumb = thumbnailUrlFor(scene);
  if (directThumb && isAllowedByCsp(directThumb)) {
    return directThumb;
  }
  for (const cand of [meta?.preview_url, meta?.thumbnail_url, meta?.browse_url]) {
    if (typeof cand === "string" && cand.trim() && isAllowedByCsp(cand.trim())) {
      return cand.trim();
    }
  }
  return null;
}

export function proxyablePreviewImageUrlFor(
  scene: SceneRow | null | undefined,
): string | null {
  if (!scene) return null;
  const directThumb = thumbnailUrlFor(scene);
  if (directThumb && isProxyablePreviewUrl(directThumb)) {
    return directThumb.trim();
  }
  const meta = scene.native_metadata as Record<string, unknown> | undefined;
  for (const cand of [meta?.preview_url, meta?.thumbnail_url, meta?.browse_url]) {
    if (typeof cand === "string" && isProxyablePreviewUrl(cand)) {
      return cand.trim();
    }
  }
  return null;
}

export function canSceneShowMapPreview(
  scene: SceneRow | null | undefined,
  batchId?: string | null,
): boolean {
  if (!scene) return false;
  if (scene.scene_id.startsWith("gee-composite::") || scene.scene_id.startsWith("gee-scene::")) {
    return false;
  }
  const bbox =
    scene.footprint_bbox ??
    scene.sar_product?.footprint_bbox ??
    sceneFootprintRenderModel(scene.footprint_geojson)?.bbox ??
    null;
  if (!bbox) return false;

  if (directPreviewImageUrlFor(scene)) return true;
  if (proxyablePreviewImageUrlFor(scene)) return true;
  const meta = scene.native_metadata as Record<string, unknown> | undefined;

  if (scene.sar_product && (landsatMapTilePreview(scene.sar_product) || sentinel2MapTilePreview(scene.sar_product))) {
    return true;
  }
  if (hlsMapTilePreview(scene)) return true;

  const optPlan = opticalQuicklookPlan(scene);
  if (optPlan && optPlan.kind !== "unavailable") {
    if (optPlan.kind === "mpc_tile") return true;
    return Boolean(batchId);
  }

  if (batchId && scene.product_key) {
    const prod = scene.sar_product;
    if (prod && isBhoonidhiSsarCatalogProduct(prod) && bhoonidhiThumbnailAsset(prod)) {
      return true;
    }
    const datasetUpper = String(scene.dataset || prod?.dataset || "").toUpperCase();
    if (
      datasetUpper.includes("NISAR") ||
      (datasetUpper === "SENTINEL-1" && String(prod?.product_family || scene.product_type || "").toUpperCase() === "GRD")
    ) {
      const rawAssets = [
        ...(scene.assets ?? []),
        ...(prod?.assets ?? []),
        ...(Array.isArray(meta?.assets) ? (meta.assets as Array<{ role?: string }>) : []),
      ];
      if (rawAssets.some((a) => ["thumbnail", "browse", "preview"].includes(String(a?.role || "").toLowerCase()))) {
        return true;
      }
    }
  }

  return false;
}

export type ResolvedMapTilePreview =
  | NonNullable<ReturnType<typeof landsatMapTilePreview>>
  | Sentinel2MapTilePreview;

function upgradeRetinaMpcTile<T extends { urlTemplate: string } | null>(tile: T): T {
  if (!tile) return tile;
  const isRetina = typeof window !== "undefined" && (window.devicePixelRatio ?? 1) > 1;
  if (
    isRetina &&
    tile.urlTemplate.includes("planetarycomputer.microsoft.com/api/data/v1/item/tiles/WebMercatorQuad/")
  ) {
    return {
      ...tile,
      urlTemplate: tile.urlTemplate.replace("{z}/{x}/{y}@1x", "{z}/{x}/{y}@2x"),
    };
  }
  return tile;
}

function buildHlsMapTileEntry(sc: SceneRow, statusLabel: string): ResolvedMapTilePreview | null {
  const hls = hlsMapTilePreview(sc);
  if (!hls) return null;
  const productKey = sc.product_key || sc.scene_id;
  return {
    productKey,
    identity: `${productKey}:mpc-hls-true-color-v1`,
    origin: "https://planetarycomputer.microsoft.com",
    urlTemplate: hls.urlTemplate,
    attribution: "Microsoft Planetary Computer · NASA HLS v2.0",
    statusLabel,
  };
}

export interface PinnedMapPreviewEntry {
  sceneId: string;
  identity: string;
  productKey: string;
  url: string | null;
  tileLayer: ResolvedMapTilePreview | null;
  bbox: Bbox;
  footprintGeojson: unknown;
  message: string;
}

export interface UseWorkbenchMapPreviewParams {
  scenes: SceneRow[];
  /** Optional subset of selectable scenes (e.g., current page in ManualWorkbench); defaults to `scenes`. */
  selectableScenes?: SceneRow[];
  selectedSceneId?: string | null;
  batchId?: string | null;
  onSelectScene?: (sceneId: string | null) => void;
  defaultPreviewEnabled?: boolean;
  /** When true (e.g. ManualWorkbench), clicking a row only highlights its footprint; previews require explicit `预览` toggle. */
  explicitToggleMode?: boolean;
  /** Injectable resolver override for tests; defaults to `loadOpticalQuicklook`. */
  resolveOpticalQuicklook?: (request: OpticalQuicklookRequest) => Promise<string>;
}

export function useWorkbenchMapPreview({
  scenes,
  selectableScenes,
  selectedSceneId = null,
  batchId,
  onSelectScene,
  defaultPreviewEnabled = true,
  explicitToggleMode = false,
  resolveOpticalQuicklook = loadOpticalQuicklook,
}: UseWorkbenchMapPreviewParams) {
  const { t } = useI18n();
  const [mapPreviewEnabled, setMapPreviewEnabled] = useState<boolean>(defaultPreviewEnabled);
  const [hoveredSceneId, setHoveredSceneId] = useState<string | null>(null);
  const [resolvedSceneAssetUrl, setResolvedSceneAssetUrl] = useState<string | null>(null);
  const [pinnedPreviewIds, setPinnedPreviewIds] = useState<Set<string>>(() => new Set());
  const [resolvedPinnedUrls, setResolvedPinnedUrls] = useState<Record<string, string>>({});
  const [resolvedS2TileLayers, setResolvedS2TileLayers] = useState<Record<string, Sentinel2MapTilePreview>>({});
  const inflightPinnedResolveRef = useRef<Set<string>>(new Set());
  const inflightS2TileResolveRef = useRef<Set<string>>(new Set());
  const fallbackAttemptedRef = useRef<Set<string>>(new Set());
  const pinnedPreviewIdsRef = useRef<Set<string>>(pinnedPreviewIds);
  pinnedPreviewIdsRef.current = pinnedPreviewIds;
  const isMountedRef = useRef<boolean>(true);
  const activeBatchRef = useRef<string | undefined>(batchId ?? undefined);
  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const findSceneById = useCallback(
    (id: string) =>
      scenes.find(
        (s) =>
          s.scene_id === id ||
          s.product_key === id ||
          s.product_id === id ||
          s.file_name === id ||
          s.scene_id.endsWith(`::ITEM::${id}`)
      ) ?? null,
    [scenes]
  );

  // Drop pinned IDs that no longer exist in active scenes (e.g. source switch or new empty search).
  useEffect(() => {
    setPinnedPreviewIds((prev) => {
      if (prev.size === 0) return prev;
      let changed = false;
      const next = new Set<string>();
      for (const id of prev) {
        if (findSceneById(id)) {
          next.add(id);
        } else {
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [findSceneById]);

  const togglePinPreviewScene = useCallback(
    (scene: SceneRow) => {
      const id = scene.scene_id;
      if (id.startsWith("gee-composite::") || id.startsWith("gee-scene::")) {
        return;
      }
      setPinnedPreviewIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) {
          next.delete(id);
          fallbackAttemptedRef.current.delete(id);
        } else {
          fallbackAttemptedRef.current.delete(id);
          next.add(id);
          setMapPreviewEnabled(true);
        }
        return next;
      });
    },
    []
  );

  const clearPinnedPreviews = useCallback(() => {
    fallbackAttemptedRef.current.clear();
    setPinnedPreviewIds(new Set());
  }, []);

  const unpinWithFailureNotice = useCallback((sceneId: string) => {
    const wasPinned = pinnedPreviewIdsRef.current.has(sceneId);
    setPinnedPreviewIds((prev) => {
      if (!prev.has(sceneId)) return prev;
      const next = new Set(prev);
      next.delete(sceneId);
      return next;
    });
    if (wasPinned) {
      emitTopToast({ text: t("mapWorkbench.desktop.quicklookLoadFailed"), tone: "danger" });
    }
  }, [t]);

  const selectedSceneObj = useMemo(
    () =>
      selectedSceneId
        ? scenes.find(
            (s) =>
              s.scene_id === selectedSceneId ||
              s.product_key === selectedSceneId ||
              s.product_id === selectedSceneId ||
              s.file_name === selectedSceneId ||
              s.scene_id.endsWith(`::ITEM::${selectedSceneId}`)
          ) ?? null
        : null,
    [scenes, selectedSceneId]
  );

  const resolveDirectTileForScene = useCallback(
    (sc: SceneRow | null | undefined): ResolvedMapTilePreview | null => {
      if (!sc) return null;
      if (sc.sar_product) {
        const direct =
          landsatMapTilePreview(sc.sar_product) ??
          sentinel2MapTilePreview(sc.sar_product) ??
          resolvedS2TileLayers[sc.scene_id] ??
          null;
        if (direct) return upgradeRetinaMpcTile(direct);
      }
      const hls = buildHlsMapTileEntry(sc, t("mapWorkbench.desktop.hlsTrueColorTilesPreview"));
      if (hls) return upgradeRetinaMpcTile(hls);
      return null;
    },
    [resolvedS2TileLayers, t],
  );

  const availablePreviewTileLayer = useMemo(
    () => resolveDirectTileForScene(selectedSceneObj),
    [resolveDirectTileForScene, selectedSceneObj],
  );

  const directSceneThumbnailUrl = useMemo(() => {
    if (!selectedSceneObj) return null;
    return directPreviewImageUrlFor(selectedSceneObj);
  }, [selectedSceneObj]);

  const effectiveBatchId = batchId ?? undefined;
  activeBatchRef.current = effectiveBatchId;

  // Resolve high-DPI Sentinel-2 L2A MPC tile layers for CDSE scenes whenever selected or pinned.
  useEffect(() => {
    if (!effectiveBatchId) return;
    let cancelled = false;
    const candidates: SceneRow[] = [];
    if (selectedSceneObj && !explicitToggleMode) candidates.push(selectedSceneObj);
    for (const pinnedId of pinnedPreviewIds) {
      const sc = findSceneById(pinnedId);
      if (sc && !candidates.some((item) => item.scene_id === sc.scene_id)) {
        candidates.push(sc);
      }
    }
    for (const sc of candidates) {
      if (!sc.sar_product || !isSentinel2L2aCatalogProduct(sc.sar_product)) continue;
      if (sentinel2MapTilePreview(sc.sar_product) || resolvedS2TileLayers[sc.scene_id]) continue;
      const itemId = opticalQuicklookItemId(sc);
      if (!itemId) continue;
      const lockKey = `${effectiveBatchId}:${sc.scene_id}`;
      if (inflightS2TileResolveRef.current.has(lockKey)) continue;
      inflightS2TileResolveRef.current.add(lockKey);
      const reqId = `s2-map-tile:${effectiveBatchId}:${Date.now()}:${sc.scene_id}`;
      void getSentinel2MapPreview({
        request_id: reqId,
        batch_id: effectiveBatchId,
        item_id: itemId,
      })
        .then((res) => {
          if (cancelled || !res.ok || res.preview.product_key !== itemId || !sc.sar_product) return;
          const tile = sentinel2ResolvedMapTilePreview(sc.sar_product, res.preview.map_item_id);
          if (tile) {
            setResolvedS2TileLayers((prev) => ({ ...prev, [sc.scene_id]: tile }));
          }
        })
        .catch(() => {})
        .finally(() => {
          inflightS2TileResolveRef.current.delete(lockKey);
        });
    }
    return () => {
      cancelled = true;
    };
  }, [effectiveBatchId, explicitToggleMode, findSceneById, pinnedPreviewIds, resolvedS2TileLayers, selectedSceneObj]);

  // Resolve async quicklook URLs directly for all explicitly pinned preview scenes (including Sentinel-2 L2A/L1C CDSE scenes so quicklook ImageOverlay never starves behind background row thumbnail queues or gets dropped on sibling updates; if MPC high-DPI tiles resolve, tileLayer supersedes resolvedUrl seamlessly).
  useEffect(() => {
    if (pinnedPreviewIds.size === 0) return;
    const capturedBatchId = effectiveBatchId;
    for (const pinnedId of pinnedPreviewIds) {
      const sc = findSceneById(pinnedId);
      if (!sc) continue;
      if (sc.scene_id.startsWith("gee-composite::") || sc.scene_id.startsWith("gee-scene::")) {
        continue;
      }
      if (resolveDirectTileForScene(sc)) continue;
      if (directPreviewImageUrlFor(sc)) continue;
      if (resolvedPinnedUrls[sc.scene_id]) continue;

      const proxyableUrl = proxyablePreviewImageUrlFor(sc);
      if (proxyableUrl) {
        const cachedDataUrl = sharedGeodataPreviewQueue.getCachedUrl(proxyableUrl);
        if (cachedDataUrl) {
          setResolvedPinnedUrls((prev) => ({ ...prev, [sc.scene_id]: cachedDataUrl }));
          continue;
        }
        const lockKey = `proxy:${sc.scene_id}:${proxyableUrl}`;
        if (!inflightPinnedResolveRef.current.has(lockKey)) {
          inflightPinnedResolveRef.current.add(lockKey);
          void sharedGeodataPreviewQueue
            .enqueue(proxyableUrl)
            .then((result) => {
              if (!isMountedRef.current || !pinnedPreviewIdsRef.current.has(sc.scene_id)) return;
              if (result.status === "loaded" && result.dataUrl) {
                const resolvedDataUrl = result.dataUrl;
                setResolvedPinnedUrls((prev) => ({ ...prev, [sc.scene_id]: resolvedDataUrl }));
              } else {
                unpinWithFailureNotice(sc.scene_id);
              }
            })
            .catch(() => {
              if (isMountedRef.current && pinnedPreviewIdsRef.current.has(sc.scene_id)) {
                unpinWithFailureNotice(sc.scene_id);
              }
            })
            .finally(() => {
              inflightPinnedResolveRef.current.delete(lockKey);
            });
        }
        continue;
      }

      const optPlan = opticalQuicklookPlan(sc);
      const optKey =
        optPlan && optPlan.kind !== "unavailable"
          ? opticalQuicklookCacheKey(sc, capturedBatchId)
          : null;
      if (optKey) {
        const cached = sharedOpticalQuicklookQueue.getCachedUrl(optKey);
        if (cached) {
          setResolvedPinnedUrls((prev) => ({ ...prev, [sc.scene_id]: cached }));
          continue;
        }
        const lockKey = `${capturedBatchId ?? ""}:${sc.scene_id}`;
        if (!inflightPinnedResolveRef.current.has(lockKey)) {
          inflightPinnedResolveRef.current.add(lockKey);
          void resolveOpticalQuicklook({ scene: sc, batchId: capturedBatchId })
            .then((url) => {
              if (!isMountedRef.current || activeBatchRef.current !== capturedBatchId) return;
              if (url) {
                setResolvedPinnedUrls((prev) => ({ ...prev, [sc.scene_id]: url }));
              } else {
                unpinWithFailureNotice(sc.scene_id);
              }
            })
            .catch(() => {
              if (isMountedRef.current && activeBatchRef.current === capturedBatchId) {
                unpinWithFailureNotice(sc.scene_id);
              }
            })
            .finally(() => {
              inflightPinnedResolveRef.current.delete(lockKey);
            });
        }
        continue;
      }

      const itemId = sc.product_key || sc.scene_id;
      const lockKey = `${capturedBatchId ?? ""}:${sc.scene_id}`;
      if (capturedBatchId && itemId) {
        if (!inflightPinnedResolveRef.current.has(lockKey)) {
          inflightPinnedResolveRef.current.add(lockKey);
          const reqId = `pin-preview:${capturedBatchId}:${itemId}:${Date.now()}`;
          void getPreviewAsset({
            request_id: reqId,
            batch_id: capturedBatchId,
            item_id: itemId,
          })
            .then((res) => {
              if (!isMountedRef.current || activeBatchRef.current !== capturedBatchId) return;
              if (res.ok && res.preview?.data_url) {
                const dataUrl = res.preview.data_url;
                setResolvedPinnedUrls((prev) => ({ ...prev, [sc.scene_id]: dataUrl }));
              } else {
                unpinWithFailureNotice(sc.scene_id);
              }
            })
            .catch(() => {
              if (isMountedRef.current && activeBatchRef.current === capturedBatchId) {
                unpinWithFailureNotice(sc.scene_id);
              }
            })
            .finally(() => {
              inflightPinnedResolveRef.current.delete(lockKey);
            });
        }
      } else {
        unpinWithFailureNotice(sc.scene_id);
      }
    }
  }, [
    pinnedPreviewIds,
    findSceneById,
    effectiveBatchId,
    resolvedPinnedUrls,
    resolveDirectTileForScene,
    resolveOpticalQuicklook,
    unpinWithFailureNotice,
  ]);

  const pinnedPreviews = useMemo<PinnedMapPreviewEntry[]>(() => {
    const entries: PinnedMapPreviewEntry[] = [];
    for (const pinnedId of pinnedPreviewIds) {
      const sc = findSceneById(pinnedId);
      if (!sc) continue;
      const bbox: Bbox | null =
        sc.footprint_bbox ??
        sc.sar_product?.footprint_bbox ??
        sceneFootprintRenderModel(sc.footprint_geojson)?.bbox ??
        null;
      if (!bbox) continue;
      const tileLayer = resolveDirectTileForScene(sc);
      const allowedDirect = directPreviewImageUrlFor(sc);
      const resolvedUrl = tileLayer ? null : (resolvedPinnedUrls[sc.scene_id] ?? allowedDirect);
      if (!tileLayer && !resolvedUrl) continue;
      const productKey = sc.product_key || sc.scene_id;
      entries.push({
        sceneId: sc.scene_id,
        identity: `${sc.scene_id}:${tileLayer?.identity ?? resolvedUrl ?? ""}`,
        productKey,
        url: tileLayer ? null : resolvedUrl,
        tileLayer,
        bbox,
        footprintGeojson: sc.footprint_geojson ?? sc.sar_product?.footprint_geojson ?? null,
        message: tileLayer?.statusLabel ?? t("mapWorkbench.desktop.quicklookOverlayPreview"),
      });
    }
    return entries;
  }, [pinnedPreviewIds, findSceneById, resolvedPinnedUrls, resolveDirectTileForScene, effectiveBatchId, t]);

  useEffect(() => {
    setResolvedSceneAssetUrl(null);
    if (explicitToggleMode) return;
    if (!selectedSceneObj || availablePreviewTileLayer) return;
    if (selectedSceneObj.sar_product && isSentinel2L2aCatalogProduct(selectedSceneObj.sar_product) && effectiveBatchId) {
      return;
    }
    let cancelled = false;

    const optPlan = directSceneThumbnailUrl ? null : opticalQuicklookPlan(selectedSceneObj);
    const optKey =
      optPlan && optPlan.kind !== "unavailable"
        ? opticalQuicklookCacheKey(selectedSceneObj, effectiveBatchId)
        : null;
    if (optKey) {
      const cached = sharedOpticalQuicklookQueue.getCachedUrl(optKey);
      if (cached) {
        setResolvedSceneAssetUrl(cached);
        return;
      }
      void sharedOpticalQuicklookQueue
        .enqueue({ scene: selectedSceneObj, batchId: effectiveBatchId })
        .then(() => {
          if (!cancelled) {
            setResolvedSceneAssetUrl(sharedOpticalQuicklookQueue.getCachedUrl(optKey) ?? null);
          }
        });
      return () => {
        cancelled = true;
      };
    }

    const itemId = selectedSceneObj.product_key || selectedSceneObj.scene_id;
    if (effectiveBatchId && itemId && !directSceneThumbnailUrl) {
      const reqId = `map-preview:${effectiveBatchId}:${itemId}:${Date.now()}`;
      void getPreviewAsset({
        request_id: reqId,
        batch_id: effectiveBatchId,
        item_id: itemId,
      })
        .then((res) => {
          if (!cancelled && res.ok && res.preview?.data_url) {
            setResolvedSceneAssetUrl(res.preview.data_url);
          }
        })
        .catch(() => {
          // ignore fallback error
        });
    }
    return () => {
      cancelled = true;
    };
  }, [selectedSceneObj, effectiveBatchId, directSceneThumbnailUrl, availablePreviewTileLayer]);

  const selectedScenePreviewUrl = resolvedSceneAssetUrl ?? directSceneThumbnailUrl;
  const selectedScenePreviewKey = selectedSceneObj
    ? selectedSceneObj.product_key || selectedSceneObj.scene_id
    : null;
  const selectedScenePreviewBbox: Bbox | null = useMemo(
    () =>
      selectedSceneObj
        ? (selectedSceneObj.footprint_bbox ??
            selectedSceneObj.sar_product?.footprint_bbox ??
            sceneFootprintRenderModel(selectedSceneObj.footprint_geojson)?.bbox ??
            null)
        : null,
    [selectedSceneObj]
  );

  const latestPinned = pinnedPreviews.length > 0 ? pinnedPreviews[pinnedPreviews.length - 1] : null;
  const hasMapPreviewAvailable = explicitToggleMode
    ? pinnedPreviewIds.size > 0
    : Boolean((availablePreviewTileLayer || selectedScenePreviewUrl) && selectedScenePreviewBbox);
  const activePreviewTileLayer = explicitToggleMode
    ? (mapPreviewEnabled ? (latestPinned?.tileLayer ?? null) : null)
    : (mapPreviewEnabled ? availablePreviewTileLayer : null);
  const activePreviewKey = explicitToggleMode
    ? (mapPreviewEnabled && latestPinned ? (latestPinned.tileLayer?.productKey ?? latestPinned.productKey) : null)
    : (mapPreviewEnabled && hasMapPreviewAvailable
        ? (activePreviewTileLayer?.productKey ?? selectedScenePreviewKey)
        : null);

  const mapPreviewState = useMemo(() => {
    if (explicitToggleMode) {
      return mapPreviewEnabled && latestPinned && activePreviewKey
        ? {
            status: "available" as const,
            product_key: activePreviewKey,
            descriptor: null,
            url: latestPinned.tileLayer ? null : (latestPinned.url ?? null),
            message: latestPinned.message,
          }
        : undefined;
    }
    return mapPreviewEnabled && hasMapPreviewAvailable && activePreviewKey
      ? {
          status: "available" as const,
          product_key: activePreviewKey,
          descriptor: null,
          url: availablePreviewTileLayer ? null : (selectedScenePreviewUrl ?? null),
          message: activePreviewTileLayer?.statusLabel ?? t("mapWorkbench.desktop.quicklookOverlayPreview"),
        }
      : undefined;
  }, [
    explicitToggleMode,
    latestPinned,
    mapPreviewEnabled,
    hasMapPreviewAvailable,
    activePreviewKey,
    selectedScenePreviewUrl,
    activePreviewTileLayer,
    t,
  ]);

  const identityIndex = useMemo(() => buildIdentityIndex(scenes), [scenes]);

  const stackScenesFor = useCallback(
    (sceneOrIdentity: string) => {
      const sceneId = identityIndex.toSceneId.get(sceneOrIdentity) ?? sceneOrIdentity;
      const ids = stackFor(scenes, sceneId);
      if (ids.length < 2) return [];
      const idSet = new Set(ids);
      return scenes.filter((s) => idSet.has(s.scene_id));
    },
    [identityIndex.toSceneId, scenes]
  );

  const allowedScenes = selectableScenes ?? scenes;

  const handleSceneSelectFromMap = useCallback(
    (identityOrSceneId: string | null) => {
      if (!onSelectScene) return;
      if (!identityOrSceneId) {
        onSelectScene(null);
        return;
      }
      const sceneId = identityIndex.toSceneId.get(identityOrSceneId) ?? identityOrSceneId;
      if (allowedScenes.some((scene) => scene.scene_id === sceneId)) {
        onSelectScene(sceneId);
      }
    },
    [identityIndex.toSceneId, onSelectScene, allowedScenes]
  );

  const mappedSelectedSceneId = selectedSceneId
    ? (identityIndex.toIdentity.get(selectedSceneId) ?? selectedSceneId)
    : null;
  const mappedHoveredSceneId = hoveredSceneId
    ? (identityIndex.toIdentity.get(hoveredSceneId) ?? hoveredSceneId)
    : null;

  const handlePinnedPreviewError = useCallback(
    (sceneId: string) => {
      const sc = findSceneById(sceneId);
      const capturedBatchId = activeBatchRef.current;
      const alreadyDataUrl = resolvedPinnedUrls[sceneId]?.startsWith("data:");
      if (
        sc &&
        capturedBatchId &&
        !alreadyDataUrl &&
        !fallbackAttemptedRef.current.has(sceneId)
      ) {
        const itemId = sc.product_key || sc.scene_id;
        fallbackAttemptedRef.current.add(sceneId);
        const lockKey = `fallback:${capturedBatchId}:${sceneId}`;
        if (!inflightPinnedResolveRef.current.has(lockKey)) {
          inflightPinnedResolveRef.current.add(lockKey);
          void getPreviewAsset({
            request_id: `pin-fallback:${capturedBatchId}:${itemId}:${Date.now()}`,
            batch_id: capturedBatchId,
            item_id: itemId,
          })
            .then((res) => {
              if (!isMountedRef.current || activeBatchRef.current !== capturedBatchId) return;
              if (res.ok && res.preview?.data_url) {
                const dataUrl = res.preview.data_url;
                setResolvedPinnedUrls((prev) => ({ ...prev, [sceneId]: dataUrl }));
              } else {
                unpinWithFailureNotice(sceneId);
              }
            })
            .catch(() => {
              if (isMountedRef.current && activeBatchRef.current === capturedBatchId) {
                unpinWithFailureNotice(sceneId);
              }
            })
            .finally(() => {
              inflightPinnedResolveRef.current.delete(lockKey);
            });
          return;
        }
      }
      unpinWithFailureNotice(sceneId);
    },
    [findSceneById, resolvedPinnedUrls, unpinWithFailureNotice],
  );

  return {
    mapPreviewEnabled,
    setMapPreviewEnabled,
    pinnedPreviewIds,
    pinnedPreviews,
    togglePinPreviewScene,
    clearPinnedPreviews,
    handlePinnedPreviewError,
    hoveredSceneId,
    setHoveredSceneId,
    selectedSceneObj,
    hasMapPreviewAvailable,
    activePreviewTileLayer,
    activePreviewKey,
    selectedScenePreviewBbox: explicitToggleMode ? (latestPinned?.bbox ?? null) : selectedScenePreviewBbox,
    mapPreviewState,
    identityIndex,
    stackScenesFor,
    handleSceneSelectFromMap,
    mappedSelectedSceneId,
    mappedHoveredSceneId,
  };
}
