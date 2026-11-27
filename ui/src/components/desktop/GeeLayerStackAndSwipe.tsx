import React, { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  Columns2,
  Eye,
  EyeOff,
  Layers,
  LocateFixed,
  Pin,
  PinOff,
  Trash2,
  X,
} from "lucide-react";
import {
  ImageOverlay,
  Pane,
  TileLayer,
  useMap,
} from "@/components/map/leafletBindings";
import type { Bbox } from "@/lib/bridge";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import {
  getGeeLayerStoreSnapshot,
  removeGeeMapLayer,
  setGeeSwipeState,
  subscribeGeeLayerStore,
  updateGeeMapLayer,
  zoomToGeeBbox,
  type GeeMapLayerItem,
} from "@/lib/geeCloudSuiteClient";

export interface MapSwipeLayerSpec {
  id: string;
  title: string;
  acquisitionDate?: string | null;
  bbox: Bbox | [number, number, number, number];
  tileUrlTemplate?: string;
  previewDataUrl?: string;
  opacity?: number;
}

function useGeeLayerStore() {
  return useSyncExternalStore(
    subscribeGeeLayerStore,
    getGeeLayerStoreSnapshot,
    getGeeLayerStoreSnapshot
  );
}

function renderLeafletRasterLayer(
  layer: Pick<GeeMapLayerItem, "id" | "tileUrlTemplate" | "previewDataUrl"> & {
    bbox: Bbox | [number, number, number, number];
    opacity?: number;
  },
  paneName: string,
  zIndex: number,
) {
  const [west, south, east, north] = Array.isArray(layer.bbox)
    ? layer.bbox
    : [layer.bbox.west, layer.bbox.south, layer.bbox.east, layer.bbox.north];
  const bounds: [[number, number], [number, number]] = [
    [south, west],
    [north, east],
  ];
  const opacity = layer.opacity ?? 0.92;
  if (layer.tileUrlTemplate) {
    return (
      <TileLayer
        key={`${paneName}:tile:${layer.id}:${layer.tileUrlTemplate}`}
        pane={paneName}
        url={layer.tileUrlTemplate}
        opacity={opacity}
        zIndex={zIndex}
        bounds={bounds}
      />
    );
  }
  if (layer.previewDataUrl) {
    return (
      <ImageOverlay
        key={`${paneName}:img:${layer.id}`}
        pane={paneName}
        url={layer.previewDataUrl}
        bounds={bounds}
        opacity={opacity}
        zIndex={zIndex}
        interactive={false}
      />
    );
  }
  return null;
}

export function GeeMapViewportAndSwipeSync({
  swipeEnabled,
  positionPct,
  zoomTarget,
  layerSignature = "",
}: {
  swipeEnabled: boolean;
  positionPct: number;
  zoomTarget: { bbox: [number, number, number, number]; nonce: number } | null;
  layerSignature?: string;
}) {
  const map = useMap();
  const lastNonceRef = useRef<number>(0);

  useEffect(() => {
    if (!map || !zoomTarget || zoomTarget.nonce === lastNonceRef.current) return;
    lastNonceRef.current = zoomTarget.nonce;
    const [west, south, east, north] = zoomTarget.bbox;
    if (
      Number.isFinite(west) &&
      Number.isFinite(south) &&
      Number.isFinite(east) &&
      Number.isFinite(north) &&
      east > west &&
      north > south
    ) {
      try {
        map.fitBounds(
          [
            [south, west],
            [north, east],
          ],
          { padding: [28, 28], maxZoom: 14 }
        );
      } catch {
        // Ignore fitBounds errors in synthetic headless test mocks
      }
    }
  }, [map, zoomTarget]);

  const applySwipeClip = useCallback(() => {
    if (!map || typeof map.getPane !== "function") return;
    const leftPane = map.getPane("gee-swipe-left-pane");
    const rightPane = map.getPane("gee-swipe-right-pane");
    if (!leftPane || !rightPane) return;
    if (!swipeEnabled) {
      leftPane.style.clip = "";
      rightPane.style.clip = "";
      leftPane.style.clipPath = "";
      rightPane.style.clipPath = "";
      return;
    }
    try {
      const size = map.getSize();
      const splitContainerX = Math.round((size.x * positionPct) / 100);
      const nw = map.containerPointToLayerPoint([0, 0]);
      const se = map.containerPointToLayerPoint([size.x, size.y]);
      const splitPt = map.containerPointToLayerPoint([splitContainerX, 0]);
      leftPane.style.clipPath = "";
      rightPane.style.clipPath = "";
      leftPane.style.clip = `rect(${nw.y}px, ${splitPt.x}px, ${se.y}px, ${nw.x}px)`;
      rightPane.style.clip = `rect(${nw.y}px, ${se.x}px, ${se.y}px, ${splitPt.x}px)`;
    } catch {
      // Fallback CSS clipPath in DOM-only unit environments
      leftPane.style.clip = "";
      rightPane.style.clip = "";
      leftPane.style.clipPath = `inset(0 ${100 - positionPct}% 0 0)`;
      rightPane.style.clipPath = `inset(0 0 0 ${positionPct}%)`;
    }
  }, [map, swipeEnabled, positionPct]);

  useEffect(() => {
    applySwipeClip();
    const raf =
      typeof requestAnimationFrame === "function"
        ? requestAnimationFrame(() => applySwipeClip())
        : 0;
    if (!map || typeof map.on !== "function") {
      return () => {
        if (raf && typeof cancelAnimationFrame === "function") cancelAnimationFrame(raf);
      };
    }
    map.on("move zoom resize", applySwipeClip);
    return () => {
      if (raf && typeof cancelAnimationFrame === "function") cancelAnimationFrame(raf);
      if (typeof map.off === "function") {
        map.off("move zoom resize", applySwipeClip);
      }
      if (typeof map.getPane === "function") {
        const leftPane = map.getPane("gee-swipe-left-pane");
        const rightPane = map.getPane("gee-swipe-right-pane");
        if (leftPane) {
          leftPane.style.clip = "";
          leftPane.style.clipPath = "";
        }
        if (rightPane) {
          rightPane.style.clip = "";
          rightPane.style.clipPath = "";
        }
      }
    };
  }, [map, applySwipeClip, layerSignature]);

  return null;
}

/**
 * Rendered inside `<MapContainer>` in `WorkbenchMap.tsx`.
 * Supports both GEE Layer Store layers and controlled multi-temporal swipe layers.
 */
export function GeeMapLeafletOverlays({
  swipeLeftLayer = null,
  swipeRightLayer = null,
  swipePosition = 50,
}: {
  swipeLeftLayer?: MapSwipeLayerSpec | null;
  swipeRightLayer?: MapSwipeLayerSpec | null;
  swipePosition?: number;
} = {}) {
  const { layers, swipe, zoomTarget } = useGeeLayerStore();

  const hasControlledPair = Boolean(
    swipeLeftLayer &&
    swipeRightLayer &&
    swipeLeftLayer.id !== swipeRightLayer.id
  );
  const storeLeft = swipe.enabled
    ? layers.find((l) => l.id === swipe.leftLayerId && l.visible) ?? null
    : null;
  const storeRight = swipe.enabled
    ? layers.find((l) => l.id === swipe.rightLayerId && l.visible) ?? null
    : null;
  const leftLayer = hasControlledPair ? swipeLeftLayer : storeLeft;
  const rightLayer = hasControlledPair ? swipeRightLayer : storeRight;
  const activeSwipePair = Boolean(leftLayer && rightLayer && leftLayer.id !== rightLayer.id);
  const positionPct = hasControlledPair
    ? Math.min(98, Math.max(2, swipePosition))
    : swipe.positionPct;

  // Render bottom-to-top (oldest lower, newest higher)
  const regularLayers = [...layers]
    .reverse()
    .filter(
      (layer) =>
        layer.visible &&
        (!activeSwipePair || (layer.id !== leftLayer?.id && layer.id !== rightLayer?.id))
    );

  return (
    <>
      <GeeMapViewportAndSwipeSync
        swipeEnabled={activeSwipePair}
        positionPct={positionPct}
        zoomTarget={zoomTarget}
        layerSignature={`${leftLayer?.id ?? ""}:${leftLayer?.tileUrlTemplate ?? ""}|${rightLayer?.id ?? ""}:${rightLayer?.tileUrlTemplate ?? ""}`}
      />
      {regularLayers.length > 0 && (
        <Pane name="gee-stack-pane" style={{ zIndex: 625, pointerEvents: "none" }}>
          {regularLayers.map((layer, idx) =>
            renderLeafletRasterLayer(layer, "gee-stack-pane", 625 + idx)
          )}
        </Pane>
      )}
      {activeSwipePair && leftLayer && (
        <Pane name="gee-swipe-left-pane" style={{ zIndex: 628, pointerEvents: "none" }}>
          {renderLeafletRasterLayer(leftLayer, "gee-swipe-left-pane", 628)}
        </Pane>
      )}
      {activeSwipePair && rightLayer && (
        <Pane name="gee-swipe-right-pane" style={{ zIndex: 629, pointerEvents: "none" }}>
          {renderLeafletRasterLayer(rightLayer, "gee-swipe-right-pane", 629)}
        </Pane>
      )}
    </>
  );
}

/**
 * Strips internal technical `PROVIDER::...::COLLECTION::...::ITEM::` prefixes and domain slugs
 * so swipe HUD badges and status bars display human-readable sensor/scene/date labels.
 */
export function formatCleanSwipeLayerTitle(
  rawTitle: string,
  acquisitionDate?: string | null
): string {
  let cleaned = (rawTitle ?? "").trim();
  if (!cleaned) {
    return acquisitionDate ? String(acquisitionDate) : "";
  }

  const lowerRaw = cleaned.toLowerCase();
  const fallbackSensor =
    lowerRaw.includes("sentinel-2") || /\bs2[abc]_/i.test(cleaned)
      ? "Sentinel-2"
      : lowerRaw.includes("sentinel-1") || /\bs1[ab]_/i.test(cleaned)
        ? "Sentinel-1"
        : lowerRaw.includes("landsat") || /\bl[ce]0[89]_/i.test(cleaned)
          ? "Landsat"
          : lowerRaw.includes("nisar")
            ? "NISAR"
            : lowerRaw.includes("copernicus")
              ? "Sentinel"
              : "遥感图层";

  if (cleaned.includes("::ITEM::")) {
    cleaned = cleaned.split("::ITEM::").pop()?.trim() ?? cleaned;
  }

  cleaned = cleaned
    .replace(
      /(?:PROVIDER|COLLECTION)::[A-Za-z0-9._-]+(?:::(?:PROVIDER|COLLECTION|ITEM)::[A-Za-z0-9._-]+)*(?:::|\.\.\.|…|\s+|-|\/|:)?/gi,
      ""
    )
    .replace(/(?:PROVIDER|COLLECTION)::[A-Za-z0-9._-]*(?:\.\.\.|…)?/gi, "")
    .replace(
      /\b(?:copernicus\.dataspace(?:\.[a-z0-9_-]+)*|microsoft\.planetary-computer(?:\.[a-z0-9_-]+)*|usgs\.eros(?:\.[a-z0-9_-]+)*|asf\.nasa(?:\.[a-z0-9_-]+)*|asf\.catalog(?:\.[a-z0-9_-]+)*|element84\.earth-search(?:\.[a-z0-9_-]+)*|science\.[a-z0-9._-]+)\b(?:\.\.\.|…)?[:/\s-]*/gi,
      ""
    )
    .replace(/^[:/\s._-]+|[:/\s._-]+$/g, "")
    .trim();

  const s2Match = cleaned.match(
    /^(S2[ABC])_MSIL(1C|2A)_(\d{4})(\d{2})(\d{2})T\d+_N\d+_R\d+_(T[0-9A-Z]{5})_/i
  );
  if (s2Match) {
    const [, sat, level, yyyy, mm, dd, tile] = s2Match;
    const dateStr = acquisitionDate || `${yyyy}-${mm}-${dd}`;
    return `${sat.toUpperCase()} L${level.toUpperCase()} ${tile.toUpperCase()} (${dateStr})`;
  }

  const s1Match = cleaned.match(
    /^(S1[AB])_([A-Z0-9]+)_([A-Z0-9]+)(?:_[A-Z0-9]+)?_(\d{4})(\d{2})(\d{2})T\d+/i
  );
  if (s1Match) {
    const [, sat, mode, prod, yyyy, mm, dd] = s1Match;
    const dateStr = acquisitionDate || `${yyyy}-${mm}-${dd}`;
    return `${sat.toUpperCase()} ${mode.toUpperCase()} ${prod.toUpperCase()} (${dateStr})`;
  }

  const landsatMatch = cleaned.match(
    /^(L[CE]0[89])_(L[12][A-Z]+)_(\d{6})_(\d{4})(\d{2})(\d{2})_/i
  );
  if (landsatMatch) {
    const [, sat, level, pathRow, yyyy, mm, dd] = landsatMatch;
    const dateStr = acquisitionDate || `${yyyy}-${mm}-${dd}`;
    return `${sat.toUpperCase()} ${level.toUpperCase()} ${pathRow} (${dateStr})`;
  }

  if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(cleaned)) {
    cleaned = cleaned.slice(0, 8);
  }

  if (!cleaned || cleaned === "..." || cleaned === "…") {
    cleaned = fallbackSensor;
  }

  if (acquisitionDate && !cleaned.includes(acquisitionDate)) {
    return `${cleaned} (${acquisitionDate})`;
  }
  return cleaned;
}

/**
 * Rendered outside `<MapContainer>` inside `WorkbenchMap`'s root relative container.
 * Provides the Multi-Layer Stack Manager + Interactive Split-Screen Swipe Divider.
 */
export function GeeLayerStackHud({
  swipeLeftLayer = null,
  swipeRightLayer = null,
  swipePosition = 50,
  onSwipePositionChange,
  onClearSwipe,
}: {
  swipeLeftLayer?: MapSwipeLayerSpec | null;
  swipeRightLayer?: MapSwipeLayerSpec | null;
  swipePosition?: number;
  onSwipePositionChange?: (position: number) => void;
  onClearSwipe?: () => void;
} = {}) {
  const { t } = useI18n();
  const { layers, swipe } = useGeeLayerStore();
  const [panelOpen, setPanelOpen] = useState(true);
  const [hasTopSplitHeader, setHasTopSplitHeader] = useState(false);
  const overlayTrackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  const hasControlledPair = Boolean(
    swipeLeftLayer &&
    swipeRightLayer &&
    swipeLeftLayer.id !== swipeRightLayer.id
  );
  const hasControlledPin = Boolean(swipeLeftLayer || swipeRightLayer);

  useEffect(() => {
    const track = overlayTrackRef.current;
    if (!track) return;
    const hostScope = track.closest("aside") ?? track.parentElement?.parentElement;
    const foundSplitHeader = Boolean(
      hostScope?.querySelector?.('[data-testid="agent-split-panel-header"]')
    );
    setHasTopSplitHeader(foundSplitHeader);
  }, [layers.length, hasControlledPin]);

  const storeLeft = swipe.enabled
    ? layers.find((l) => l.id === swipe.leftLayerId && l.visible) ?? null
    : null;
  const storeRight = swipe.enabled
    ? layers.find((l) => l.id === swipe.rightLayerId && l.visible) ?? null
    : null;

  const leftLayer = hasControlledPair ? swipeLeftLayer : storeLeft;
  const rightLayer = hasControlledPair ? swipeRightLayer : storeRight;
  const activeSwipePair = Boolean(leftLayer && rightLayer && leftLayer.id !== rightLayer.id);
  const positionPct = hasControlledPair
    ? Math.min(98, Math.max(2, swipePosition))
    : swipe.positionPct;

  const commitSwipePosition = useCallback(
    (nextPct: number) => {
      const clamped = Math.min(98, Math.max(2, nextPct));
      if (hasControlledPair && onSwipePositionChange) {
        onSwipePositionChange(clamped);
      } else {
        setGeeSwipeState({ positionPct: clamped });
      }
    },
    [hasControlledPair, onSwipePositionChange],
  );

  const updatePctFromClientX = useCallback(
    (clientX: number) => {
      const track = overlayTrackRef.current;
      if (!track) return;
      const rect = track.getBoundingClientRect();
      if (rect.width <= 0) return;
      const pct = ((clientX - rect.left) / rect.width) * 100;
      commitSwipePosition(pct);
    },
    [commitSwipePosition],
  );

  if (layers.length === 0 && !hasControlledPin) {
    return null;
  }

  const formatLayerBadgeTitle = (item: Pick<MapSwipeLayerSpec, "title" | "acquisitionDate">) =>
    formatCleanSwipeLayerTitle(item.title, item.acquisitionDate);

  const swipeBadgesTopClass =
    hasTopSplitHeader && hasControlledPin && onClearSwipe ? "top-24" : "top-14";

  return (
    <div
      ref={overlayTrackRef}
      className="pointer-events-none absolute inset-0 z-[620]"
      data-testid="gee-map-hud"
    >
      {/* Interactive Split-Screen Swipe Vertical Divider */}
      {activeSwipePair && leftLayer && rightLayer && (
        <>
          <div
            className={cn(
              "pointer-events-none absolute left-16 z-[625] max-w-[calc(50%-5rem)] truncate rounded-full border border-sp-line bg-sp-surface px-3 py-1 text-xs font-semibold text-sp-text shadow-overlay backdrop-blur-md",
              swipeBadgesTopClass
            )}
            data-testid="gee-swipe-left-badge"
            title={formatLayerBadgeTitle(leftLayer as MapSwipeLayerSpec)}
          >
            {t("mapWorkbench.gee.swipeLeftBadge", {
              title: formatLayerBadgeTitle(leftLayer as MapSwipeLayerSpec),
            })}
          </div>
          <div
            className={cn(
              "pointer-events-none absolute right-16 z-[625] max-w-[calc(50%-5rem)] truncate rounded-full border border-sp-line bg-sp-surface px-3 py-1 text-xs font-semibold text-sp-text shadow-overlay backdrop-blur-md",
              swipeBadgesTopClass
            )}
            data-testid="gee-swipe-right-badge"
            title={formatLayerBadgeTitle(rightLayer as MapSwipeLayerSpec)}
          >
            {t("mapWorkbench.gee.swipeRightBadge", {
              title: formatLayerBadgeTitle(rightLayer as MapSwipeLayerSpec),
            })}
          </div>
          <div
            role="slider"
            tabIndex={0}
            data-testid="gee-swipe-divider"
            aria-label={t("mapWorkbench.gee.swipeDividerAria")}
            aria-valuemin={2}
            aria-valuemax={98}
            aria-valuenow={Math.round(positionPct)}
            style={{ left: `${positionPct}%` }}
            onPointerDown={(e) => {
              e.stopPropagation();
              draggingRef.current = true;
              try {
                e.currentTarget.setPointerCapture(e.pointerId);
              } catch {
                // Ignore in synthetic jsdom pointer events
              }
            }}
            onPointerMove={(e) => {
              if (!draggingRef.current) return;
              e.stopPropagation();
              updatePctFromClientX(e.clientX);
            }}
            onPointerUp={(e) => {
              if (!draggingRef.current) return;
              e.stopPropagation();
              draggingRef.current = false;
              try {
                e.currentTarget.releasePointerCapture(e.pointerId);
              } catch {
                // Ignore
              }
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft") {
                e.preventDefault();
                e.stopPropagation();
                commitSwipePosition(positionPct - 5);
              } else if (e.key === "ArrowRight") {
                e.preventDefault();
                e.stopPropagation();
                commitSwipePosition(positionPct + 5);
              }
            }}
            className="pointer-events-auto absolute top-0 bottom-0 -ml-[11px] w-[22px] cursor-ew-resize flex items-center justify-center focus:outline-none group"
          >
            <div className="h-full w-[2px] bg-sp-surface shadow-overlay" />
            <div className="absolute top-1/2 -translate-y-1/2 h-8 w-8 rounded-full border border-sp-line bg-sp-surface text-sp-text shadow-overlay flex items-center justify-center group-hover:scale-105 transition-transform">
              <Columns2 className="h-4 w-4 text-sp-accent" />
            </div>
          </div>
        </>
      )}

      {hasControlledPin && onClearSwipe && (
        <div
          className={cn(
            "pointer-events-none absolute left-16 right-36 z-[630] flex justify-center",
            hasTopSplitHeader ? "top-14" : "top-3"
          )}
        >
          <div
            data-testid="scene-swipe-status-bar"
            className="pointer-events-auto flex max-w-full min-w-0 items-center gap-2 rounded-full border border-sp-line bg-sp-surface px-3 py-1 text-xs text-sp-text shadow-overlay backdrop-blur-md"
            onPointerDown={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
          >
            <Columns2 className="h-3.5 w-3.5 text-sp-accent shrink-0" />
            <span
              className="min-w-0 truncate max-w-[160px]"
              title={
                swipeLeftLayer
                  ? formatCleanSwipeLayerTitle(swipeLeftLayer.title, swipeLeftLayer.acquisitionDate)
                  : undefined
              }
            >
              左(Before):{" "}
              {swipeLeftLayer
                ? formatCleanSwipeLayerTitle(swipeLeftLayer.title, swipeLeftLayer.acquisitionDate)
                : "待选右图后开启（点击可取消）"}
            </span>
            <span className="text-sp-text-muted">|</span>
            <span
              className="min-w-0 truncate max-w-[160px]"
              title={
                swipeRightLayer
                  ? formatCleanSwipeLayerTitle(swipeRightLayer.title, swipeRightLayer.acquisitionDate)
                  : undefined
              }
            >
              右(After):{" "}
              {swipeRightLayer
                ? formatCleanSwipeLayerTitle(swipeRightLayer.title, swipeRightLayer.acquisitionDate)
                : "待选对比景（再点已选可取消）"}
            </span>
            <button
              type="button"
              data-testid="scene-swipe-clear"
              onClick={onClearSwipe}
              aria-label="退出卷帘对比"
              className="h-4 w-4 rounded-full inline-flex items-center justify-center text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
        </div>
      )}

      {layers.length > 0 && (
      <div
        className="pointer-events-auto absolute bottom-9 left-3 z-[630] flex flex-col items-start gap-2"
        onPointerDown={(e) => e.stopPropagation()}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onWheel={(e) => e.stopPropagation()}
      >
        {panelOpen && (
          <div
            data-testid="gee-layer-stack-panel"
            className="w-[300px] max-h-[300px] overflow-y-auto rounded-sp-modal border border-sp-line bg-sp-surface/95 p-3 shadow-overlay backdrop-blur-xl flex flex-col gap-2"
          >
            <div className="flex items-center justify-between gap-2 border-b border-sp-line pb-2">
              <div className="flex items-center gap-2 text-xs font-semibold text-sp-text">
                <Layers className="h-3.5 w-3.5 text-sp-accent" />
                <span>
                  {t("mapWorkbench.gee.layerStackHeading", { count: layers.length })}
                </span>
              </div>
              <div className="flex items-center gap-1">
                {layers.length >= 2 && (
                  <button
                    type="button"
                    data-testid="gee-toggle-swipe-btn"
                    onClick={() =>
                      setGeeSwipeState({
                        enabled: !swipe.enabled,
                        leftLayerId: swipe.leftLayerId ?? layers[1]?.id ?? null,
                        rightLayerId: swipe.rightLayerId ?? layers[0]?.id ?? null,
                      })
                    }
                    className={cn(
                      "h-6 px-2 rounded-full text-[13px] font-semibold border transition-colors cursor-pointer inline-flex items-center gap-1",
                      swipe.enabled
                        ? "border-sp-accent bg-sp-accent/12 text-sp-accent"
                        : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover"
                    )}
                  >
                    <Columns2 className="h-3 w-3" />
                    <span>{t("mapWorkbench.gee.swipeToggleBtn")}</span>
                  </button>
                )}
                <button
                  type="button"
                  aria-label={t("mapWorkbench.gee.collapseLayerStackAria")}
                  onClick={() => setPanelOpen(false)}
                  className="h-6 w-6 rounded-sp-sm inline-flex items-center justify-center text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>

            <div className="flex flex-col gap-2">
              {layers.map((layer) => {
                const isLeft = swipe.enabled && swipe.leftLayerId === layer.id;
                const isRight = swipe.enabled && swipe.rightLayerId === layer.id;
                const gradientCss =
                  layer.palette && layer.palette.length >= 2
                    ? `linear-gradient(to right, ${layer.palette.join(", ")})`
                    : null;

                return (
                  <div
                    key={layer.id}
                    data-testid={`gee-layer-item-${layer.id}`}
                    className="rounded-sp-card border border-sp-line bg-sp-surface p-2 flex flex-col gap-2"
                  >
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        aria-label={
                          layer.visible
                            ? t("mapWorkbench.gee.hideLayerAria", { title: layer.title })
                            : t("mapWorkbench.gee.showLayerAria", { title: layer.title })
                        }
                        onClick={() => updateGeeMapLayer(layer.id, { visible: !layer.visible })}
                        className="h-5 w-5 rounded-sp-sm inline-flex items-center justify-center text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer shrink-0"
                      >
                        {layer.visible ? (
                          <Eye className="h-3.5 w-3.5 text-sp-accent" />
                        ) : (
                          <EyeOff className="h-3.5 w-3.5 text-sp-text-muted" />
                        )}
                      </button>
                      <span
                        className="text-xs font-semibold text-sp-text truncate flex-1"
                        title={formatCleanSwipeLayerTitle(layer.title)}
                      >
                        {formatCleanSwipeLayerTitle(layer.title)}
                      </span>
                      <button
                        type="button"
                        title={
                          layer.pinned
                            ? t("mapWorkbench.gee.pinnedTitle")
                            : t("mapWorkbench.gee.unpinnedTitle")
                        }
                        aria-label={
                          layer.pinned
                            ? t("mapWorkbench.gee.unpinLayerAria")
                            : t("mapWorkbench.gee.pinLayerAria")
                        }
                        onClick={() => updateGeeMapLayer(layer.id, { pinned: !layer.pinned })}
                        className={cn(
                          "h-5 w-5 rounded-sp-sm inline-flex items-center justify-center hover:bg-sp-hover cursor-pointer shrink-0",
                          layer.pinned ? "text-sp-accent" : "text-sp-text-muted"
                        )}
                      >
                        {layer.pinned ? <Pin className="h-3 w-3" /> : <PinOff className="h-3 w-3" />}
                      </button>
                      <button
                        type="button"
                        title={t("mapWorkbench.gee.zoomToLayerTitle")}
                        aria-label={t("mapWorkbench.gee.zoomToLayerAria", { title: layer.title })}
                        onClick={() => zoomToGeeBbox(layer.bbox)}
                        className="h-5 w-5 rounded-sp-sm inline-flex items-center justify-center text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer shrink-0"
                      >
                        <LocateFixed className="h-3 w-3" />
                      </button>
                      <button
                        type="button"
                        title={t("mapWorkbench.gee.removeLayerTitle")}
                        aria-label={t("mapWorkbench.gee.removeLayerAria", { title: layer.title })}
                        onClick={() => removeGeeMapLayer(layer.id)}
                        className="h-5 w-5 rounded-sp-sm inline-flex items-center justify-center text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer shrink-0"
                      >
                        <Trash2 className="h-3 w-3" />
                      </button>
                    </div>

                    {/* Opacity Slider & Swipe Side Selector */}
                    <div className="flex items-center gap-2 text-xs text-sp-text-secondary">
                      <span className="shrink-0">{t("mapWorkbench.gee.opacityLabel")}</span>
                      <input
                        type="range"
                        min={0}
                        max={100}
                        value={Math.round(layer.opacity * 100)}
                        aria-label={t("mapWorkbench.gee.layerOpacityAria", { title: layer.title })}
                        onChange={(e) =>
                          updateGeeMapLayer(layer.id, {
                            opacity: Number(e.currentTarget.value) / 100,
                          })
                        }
                        className="flex-1 accent-sp-accent cursor-pointer h-1.5"
                      />
                      <span className="w-8 text-right font-mono">
                        {Math.round(layer.opacity * 100)}%
                      </span>
                    </div>

                    {layers.length >= 2 && (
                      <div className="flex items-center gap-1">
                        <button
                          type="button"
                          onClick={() =>
                            setGeeSwipeState({
                              enabled: true,
                              leftLayerId: layer.id,
                              rightLayerId:
                                swipe.rightLayerId && swipe.rightLayerId !== layer.id
                                  ? swipe.rightLayerId
                                  : layers.find((l) => l.id !== layer.id)?.id ?? null,
                            })
                          }
                          className={cn(
                            "px-2 py-1 rounded-sp-sm text-[13px] border cursor-pointer",
                            isLeft
                              ? "border-sp-accent bg-sp-accent/12 text-sp-accent font-semibold"
                              : "border-sp-line text-sp-text-muted hover:text-sp-text"
                          )}
                        >
                          {t("mapWorkbench.gee.swipeLeftBtn")}
                        </button>
                        <button
                          type="button"
                          onClick={() =>
                            setGeeSwipeState({
                              enabled: true,
                              rightLayerId: layer.id,
                              leftLayerId:
                                swipe.leftLayerId && swipe.leftLayerId !== layer.id
                                  ? swipe.leftLayerId
                                  : layers.find((l) => l.id !== layer.id)?.id ?? null,
                            })
                          }
                          className={cn(
                            "px-2 py-1 rounded-sp-sm text-[13px] border cursor-pointer",
                            isRight
                              ? "border-sp-accent bg-sp-accent/12 text-sp-accent font-semibold"
                              : "border-sp-line text-sp-text-muted hover:text-sp-text"
                          )}
                        >
                          {t("mapWorkbench.gee.swipeRightBtn")}
                        </button>
                      </div>
                    )}

                    {/* Continuous Color-Ramp Legend */}
                    {gradientCss && (
                      <div className="flex flex-col gap-1 pt-1">
                        <div
                          className="h-2 w-full rounded-full border border-sp-line"
                          style={{ background: gradientCss }}
                        />
                        <div className="flex items-center justify-between text-xs font-mono text-sp-text-muted">
                          <span>{Number(layer.visMin).toFixed(2)}</span>
                          {layer.unit && <span>{layer.unit}</span>}
                          <span>{Number(layer.visMax).toFixed(2)}</span>
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {!panelOpen && (
          <button
            type="button"
            data-testid="gee-layer-stack-pill"
            onClick={() => setPanelOpen(true)}
            className="h-7 rounded-full border border-sp-line bg-sp-surface/92 px-3 text-[13px] font-semibold text-sp-text shadow-overlay hover:bg-sp-surface inline-flex items-center gap-2 cursor-pointer"
          >
            <Layers className="h-3.5 w-3.5 text-sp-accent" />
            <span>{t("mapWorkbench.gee.layerPillLabel", { count: layers.length })}</span>
            {activeSwipePair && (
              <span className="text-sp-accent">{t("mapWorkbench.gee.swipingTag")}</span>
            )}
          </button>
        )}
      </div>
      )}
    </div>
  );
}
