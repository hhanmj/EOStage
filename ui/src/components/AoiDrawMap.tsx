import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  LatLng,
  LatLngBoundsExpression,
  LatLngExpression,
  LeafletMouseEvent,
} from "leaflet";
import { Info, Upload } from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import {
  ImageOverlay,
  MapContainer,
  Pane,
  Polygon,
  Polyline,
  Rectangle,
  TileLayer,
  useMap,
  useMapEvents,
  ZoomControl,
} from "@/components/map/leafletBindings";
import type { Bbox, Json } from "@/lib/bridge";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import {
  canonicalLeafletLongitude,
  canonicalizeLeafletPolygonRing,
  canonicalizeLeafletRectangle,
} from "@/lib/antimeridian";
import { DEFAULT_MAP_PROVIDER, mapTileLayer, type MapProviderKey } from "@/lib/mapProviders";
import {
  importVectorFromHtml5Files,
  importVectorFromNativePaths,
  useNativeVectorDropTarget,
} from "@/lib/vectorDropImport";
import { AoiOverlay } from "./AoiOverlay";
import { PreviewImageOverlay } from "./PreviewImageOverlay";

export type DrawMode = "rect" | "polygon" | "point";

export type AoiDrawMapPreviewOverlay = {
  identity: string;
  url: string | null;
  bbox: Bbox | null;
  geometry?: Json | null;
  label?: string;
};

const DEFAULT_BBOX: Bbox = {
  west: 110.22,
  east: 110.52,
  south: 30.92,
  north: 31.14,
  crs: "EPSG:4326",
};

function boundsFromBbox(bbox: Bbox): LatLngBoundsExpression {
  return [
    [bbox.south, bbox.west],
    [bbox.north, bbox.east],
  ];
}

function centerFromBbox(bbox: Bbox): LatLngExpression {
  return [(bbox.south + bbox.north) / 2, (bbox.west + bbox.east) / 2];
}

function FitToBbox({ bbox }: { bbox: Bbox }) {
  const map = useMap();

  useEffect(() => {
    map.fitBounds(boundsFromBbox(bbox), {
      animate: false,
      maxZoom: 12,
      padding: [24, 24],
    });
  }, [bbox.east, bbox.north, bbox.south, bbox.west, map]);

  return null;
}

function DrawLayer({
  mode,
  active,
  onRect,
  onPolygon,
  onPoint,
}: {
  mode: DrawMode;
  active: boolean;
  onRect: (bbox: Bbox) => void;
  onPolygon: (ring: [number, number][]) => void;
  onPoint: (lat: number, lng: number) => void;
}) {
  const map = useMap();
  const [start, setStart] = useState<LatLng | null>(null);
  const [current, setCurrent] = useState<LatLng | null>(null);
  const [sketch, setSketch] = useState<LatLng[]>([]);
  const [hover, setHover] = useState<LatLng | null>(null);

  const appendUniquePoint = useCallback((points: LatLng[], point: LatLng) => {
    const last = points[points.length - 1];
    if (last && last.distanceTo(point) < 0.5) return points;
    return [...points, point];
  }, []);

  const finishPolygon = useCallback(
    (extra?: LatLng) => {
      const points = extra ? appendUniquePoint(sketch, extra) : sketch;
      if (points.length < 3) {
        setSketch([]);
        setHover(null);
        return;
      }
      const ring = canonicalizeLeafletPolygonRing(points.map((point) => [point.lng, point.lat]));
      if (!ring) {
        setSketch([]);
        setHover(null);
        return;
      }
      onPolygon(ring);
      setSketch([]);
      setHover(null);
    },
    [appendUniquePoint, onPolygon, sketch],
  );

  useEffect(() => {
    if (!active) return;
    const dragWasEnabled = map.dragging.enabled();
    const dblClickWasEnabled = map.doubleClickZoom.enabled();
    if (mode === "rect" || mode === "polygon") map.dragging.disable();
    if (mode === "polygon") map.doubleClickZoom.disable();
    return () => {
      if (dragWasEnabled) map.dragging.enable();
      else map.dragging.disable();
      if (dblClickWasEnabled) map.doubleClickZoom.enable();
      else map.doubleClickZoom.disable();
    };
  }, [active, map, mode]);

  useEffect(() => {
    setStart(null);
    setCurrent(null);
    setSketch([]);
    setHover(null);
  }, [mode]);

  useEffect(() => {
    if (!active || mode !== "polygon") return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Enter") finishPolygon();
      if (event.key === "Escape") {
        setSketch([]);
        setHover(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, finishPolygon, mode]);

  useMapEvents({
    mousedown(e) {
      if (!active || mode !== "rect") return;
      e.originalEvent.preventDefault();
      if (mode === "rect") {
        setStart(e.latlng);
        setCurrent(e.latlng);
      }
    },
    mousemove(e) {
      if (!active) return;
      if (mode === "rect" && start) setCurrent(e.latlng);
      if (mode === "polygon" && sketch.length) setHover(e.latlng);
    },
    mouseup() {
      if (!active || mode !== "rect") return;
      if (mode === "rect" && start && current) {
        const authoritative = canonicalizeLeafletRectangle(
          [start.lng, start.lat],
          [current.lng, current.lat],
        );
        if (authoritative?.kind === "bbox") onRect(authoritative.bbox);
        if (authoritative?.kind === "polygon") onPolygon(authoritative.ring);
        setStart(null);
        setCurrent(null);
      }
    },
    click(e: LeafletMouseEvent) {
      if (!active) return;
      if (mode === "point") {
        const longitude = canonicalLeafletLongitude(e.latlng.lng);
        if (longitude !== null) onPoint(e.latlng.lat, longitude);
      }
      if (mode === "polygon") {
        setSketch((prev) => appendUniquePoint(prev, e.latlng));
        setHover(null);
      }
    },
    dblclick(e: LeafletMouseEvent) {
      if (!active || mode !== "polygon") return;
      e.originalEvent.preventDefault();
      finishPolygon(e.latlng);
    },
  });

  if (mode === "rect" && start && current) {
    const south = Math.min(start.lat, current.lat);
    const north = Math.max(start.lat, current.lat);
    const west = Math.min(start.lng, current.lng);
    const east = Math.max(start.lng, current.lng);
    return (
      <Rectangle
        bounds={[
          [south, west],
          [north, east],
        ]}
        pathOptions={{ color: "#2563eb", weight: 2, dashArray: "6 4", fillOpacity: 0.12 }}
      />
    );
  }

  const polygonPreview = hover ? [...sketch, hover] : sketch;
  if (mode === "polygon" && polygonPreview.length > 1) {
    return (
      <Polyline
        positions={polygonPreview.map((p) => [p.lat, p.lng])}
        pathOptions={{ color: "#2563eb", weight: 2.5 }}
      />
    );
  }

  return null;
}

function GeometryOverlay({
  geometry,
  className,
  color,
  fillColor,
  fillOpacity,
  dashArray,
}: {
  geometry: Json;
  className: string;
  color: string;
  fillColor: string;
  fillOpacity: number;
  dashArray?: string;
}) {
  return (
    <AoiOverlay
      geometry={geometry}
      bbox={null}
      renderPolygon={({ key, positions }) => (
        <Polygon
          key={`${className}:${key}`}
          positions={positions!}
          pathOptions={{
            color,
            weight: 2,
            fillColor,
            fillOpacity,
            dashArray,
            className,
          }}
          interactive={false}
        />
      )}
      renderRectangle={() => null}
    />
  );
}

export function AoiDrawMap({
  bbox = DEFAULT_BBOX,
  boundBbox,
  mode = "rect",
  drawActive = true,
  minHeight = 520,
  providerKey = DEFAULT_MAP_PROVIDER.key,
  footprintGeometries = [],
  requestedGeometry = null,
  effectiveGeometries = [],
  previewOverlay = null,
  onRectDraw,
  onPolygonDraw,
  onPointDraw,
  onGeometryImport,
}: {
  bbox?: Bbox;
  boundBbox?: Bbox | null;
  mode?: DrawMode;
  drawActive?: boolean;
  minHeight?: number;
  providerKey?: MapProviderKey;
  footprintGeometries?: readonly Json[];
  requestedGeometry?: Json | null;
  effectiveGeometries?: readonly Json[];
  previewOverlay?: AoiDrawMapPreviewOverlay | null;
  onRectDraw?: (bbox: Bbox) => void;
  onPolygonDraw?: (ring: [number, number][]) => void;
  onPointDraw?: (lat: number, lng: number) => void;
  onGeometryImport?: (payload: { bbox: Bbox; geometry: Json; label: string }) => void;
}) {
  const { t } = useI18n();
  const display = previewOverlay?.bbox ?? boundBbox ?? bbox;
  const center = centerFromBbox(display);
  const [tilesReady, setTilesReady] = useState(false);
  const tileLayer = mapTileLayer(providerKey);
  const [attributionOpen, setAttributionOpen] = useState(false);
  const [attributionHovered, setAttributionHovered] = useState(false);
  const attributionRef = useRef<HTMLDivElement | null>(null);
  const attributionButtonRef = useRef<HTMLButtonElement | null>(null);
  const [htmlDragActive, setHtmlDragActive] = useState(false);
  const [nativeDragActive, setNativeDragActive] = useState(false);
  const [dropBusy, setDropBusy] = useState(false);
  const [dropError, setDropError] = useState<string | null>(null);
  const dragDepthRef = useRef(0);

  const cleanedAttribution = useMemo(
    () => (tileLayer.attribution || "").replaceAll("&copy;", "©").replaceAll("&amp;", "&").trim(),
    [tileLayer.attribution],
  );
  const attributionCreditsText = cleanedAttribution
    ? `Leaflet · ${cleanedAttribution}`
    : "Leaflet";

  // 地图来源展开后 4 秒自动收起；鼠标停留时不收
  useEffect(() => {
    if (!attributionOpen || attributionHovered) return;
    const timer = window.setTimeout(() => setAttributionOpen(false), 4000);
    return () => window.clearTimeout(timer);
  }, [attributionOpen, attributionHovered]);

  useEffect(() => {
    if (!attributionOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAttributionOpen(false);
    };
    const onPointerDown = (e: MouseEvent | TouchEvent) => {
      if (
        attributionRef.current &&
        !attributionRef.current.contains(e.target as Node) &&
        attributionButtonRef.current &&
        !attributionButtonRef.current.contains(e.target as Node)
      ) {
        setAttributionOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [attributionOpen]);

  const applyImportResult = useCallback(
    (res: Awaited<ReturnType<typeof importVectorFromNativePaths>>) => {
      if (!res.ok) {
        setDropError(res.error);
        return;
      }
      setDropError(null);
      onGeometryImport?.({
        bbox: res.bbox,
        geometry: res.geometry as Json,
        label: res.label,
      });
    },
    [onGeometryImport],
  );

  useNativeVectorDropTarget({
    enabled: drawActive && Boolean(onGeometryImport),
    priority: 20,
    onHoverChange: setNativeDragActive,
    onPathsDropped: (paths) => {
      setDropBusy(true);
      void importVectorFromNativePaths(paths, { persistWorkingAoi: false })
        .then(applyImportResult)
        .finally(() => setDropBusy(false));
    },
  });

  const modeHint = useMemo(() => {
    if (!drawActive) return "只读范围";
    if (mode === "rect") return "拖拽绘制矩形";
    if (mode === "polygon") return "点击加点，双击完成";
    return "单击设置中心点";
  }, [drawActive, mode]);

  return (
    <div
      className="relative h-full w-full overflow-hidden rounded-lg border bg-[linear-gradient(0deg,rgba(13,148,136,0.08)_1px,transparent_1px),linear-gradient(90deg,rgba(13,148,136,0.08)_1px,transparent_1px)] bg-[size:32px_32px]"
      onDragEnter={(e) => {
        if (!drawActive || !onGeometryImport) return;
        dragDepthRef.current += 1;
        e.preventDefault();
        setHtmlDragActive(true);
      }}
      onDragOver={(e) => {
        if (!drawActive || !onGeometryImport) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
      }}
      onDragLeave={() => {
        if (!drawActive || !onGeometryImport) return;
        dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
        if (dragDepthRef.current === 0) setHtmlDragActive(false);
      }}
      onDrop={(e) => {
        if (!drawActive || !onGeometryImport) return;
        dragDepthRef.current = 0;
        setHtmlDragActive(false);
        const files = e.dataTransfer?.files;
        if (!files || files.length === 0) return;
        e.preventDefault();
        e.stopPropagation();
        setDropBusy(true);
        void importVectorFromHtml5Files(files, { persistWorkingAoi: false })
          .then(applyImportResult)
          .finally(() => setDropBusy(false));
      }}
    >
      <MapContainer
        center={center}
        zoom={10}
        scrollWheelZoom
        zoomControl={false}
        attributionControl={false}
        className={"relative z-0 isolate h-full w-full " + (drawActive ? "cursor-crosshair" : "")}
        style={{ minHeight, background: "transparent" }}
      >
        <TileLayer
          url={tileLayer.url}
          subdomains={tileLayer.subdomains}
          attribution={tileLayer.attribution}
          updateWhenIdle
          keepBuffer={4}
          eventHandlers={{
            load: () => setTilesReady(true),
          }}
        />
        <FitToBbox bbox={display} />
        <ZoomControl position="topright" />
        {previewOverlay?.url && previewOverlay?.bbox && (
          <Pane name="aoi-draw-preview-pane" style={{ zIndex: 620, pointerEvents: "none" }}>
            <PreviewImageOverlay
              identity={previewOverlay.identity}
              url={previewOverlay.url}
              bounds={boundsFromBbox(previewOverlay.bbox)}
              renderImage={({ url, bounds, opacity, className, onLoad, onError }) => (
                <ImageOverlay
                  pane="aoi-draw-preview-pane"
                  url={url}
                  bounds={bounds}
                  opacity={opacity}
                  className={className}
                  zIndex={620}
                  interactive={false}
                  eventHandlers={{ load: onLoad, error: onError }}
                />
              )}
            />
          </Pane>
        )}
        {footprintGeometries.map((geometry, index) => (
          <GeometryOverlay
            key={`footprint:${index}`}
            geometry={geometry}
            className="nisar-subset-footprint"
            color="#f59e0b"
            fillColor="#f59e0b"
            fillOpacity={0.05}
            dashArray="8 5"
          />
        ))}
        {previewOverlay?.geometry && (
          <GeometryOverlay
            geometry={previewOverlay.geometry}
            className="nisar-subset-preview-footprint"
            color="#06b6d4"
            fillColor="#06b6d4"
            fillOpacity={previewOverlay.url ? 0 : 0.14}
          />
        )}
        {requestedGeometry && (
          <GeometryOverlay
            geometry={requestedGeometry}
            className="nisar-subset-requested"
            color="#2563eb"
            fillColor="#2563eb"
            fillOpacity={previewOverlay?.url ? 0 : 0.08}
            dashArray="6 4"
          />
        )}
        {effectiveGeometries.map((geometry, index) => (
          <GeometryOverlay
            key={`effective:${index}`}
            geometry={geometry}
            className="nisar-subset-effective"
            color="#16a34a"
            fillColor="#22c55e"
            fillOpacity={0.18}
          />
        ))}
        {boundBbox && (
          <Rectangle
            bounds={boundsFromBbox(boundBbox)}
            pathOptions={{ color: "#0d9488", weight: 2.5, fillOpacity: 0.14 }}
          />
        )}
        <DrawLayer
          mode={mode}
          active={drawActive}
          onRect={(b) => onRectDraw?.(b)}
          onPolygon={(ring) => onPolygonDraw?.(ring)}
          onPoint={(lat, lng) => onPointDraw?.(lat, lng)}
        />
      </MapContainer>

      {(footprintGeometries.length > 0 || requestedGeometry || effectiveGeometries.length > 0 || previewOverlay) && (
        <div
          role="list"
          data-testid="aoi-draw-map-legend"
          aria-label="裁剪地图图层"
          className="pointer-events-none absolute bottom-3 left-3 z-[500] flex max-w-[calc(100%-3.5rem)] flex-wrap items-center gap-3 rounded-full border border-sp-line bg-sp-surface/95 px-3 py-1.5 text-[11px] font-medium text-sp-text shadow-subtle backdrop-blur-sm"
        >
          {footprintGeometries.length > 0 && (
            <span role="listitem" className="inline-flex items-center gap-1.5 text-sp-warning-text">
              <span className="h-2 w-2 rounded-full bg-[#f59e0b]" />
              影像覆盖
            </span>
          )}
          {previewOverlay && (
            <span role="listitem" className="inline-flex items-center gap-1.5 text-cyan-600 dark:text-cyan-400">
              <span className="h-2 w-2 rounded-full bg-[#06b6d4]" />
              当前预览影像框
            </span>
          )}
          {requestedGeometry && (
            <span role="listitem" className="inline-flex items-center gap-1.5 text-sp-accent">
              <span className="h-2 w-2 rounded-full bg-[#2563eb]" />
              请求范围
            </span>
          )}
          {effectiveGeometries.length > 0 && (
            <span role="listitem" className="inline-flex items-center gap-1.5 text-sp-success-text">
              <span className="h-2 w-2 rounded-full bg-[#16a34a]" />
              实际相交范围
            </span>
          )}
        </div>
      )}

      <div
        ref={attributionRef}
        onMouseEnter={() => setAttributionHovered(true)}
        onMouseLeave={() => setAttributionHovered(false)}
        onMouseDown={(e) => e.stopPropagation()}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
        className="pointer-events-auto absolute bottom-3 right-3 z-[520] flex max-w-[calc(100%-1.5rem)] items-center justify-end gap-2 select-none"
      >
        {attributionOpen && (
          <div
            data-testid="aoi-draw-map-attribution-popover"
            className="sp-popover-blur-in origin-right flex h-7 min-w-0 max-w-[20rem] items-center gap-1 rounded-full border border-sp-line bg-sp-surface/95 px-2.5 text-xs text-sp-text-secondary shadow-overlay backdrop-blur-sm select-none"
            title={attributionCreditsText}
          >
            <a
              href="https://leafletjs.com"
              target="_blank"
              rel="noopener noreferrer"
              className="shrink-0 text-sp-text hover:underline cursor-pointer"
              onClick={(e) => e.stopPropagation()}
            >
              Leaflet
            </a>
            {cleanedAttribution && (
              <span className="min-w-0 truncate"> · {cleanedAttribution}</span>
            )}
          </div>
        )}
        <button
          ref={attributionButtonRef}
          type="button"
          data-testid="aoi-draw-map-attribution-btn"
          title={t("panels.map.mapAttribution")}
          aria-label={t("panels.map.mapAttribution")}
          aria-expanded={attributionOpen}
          onClick={(e) => {
            e.stopPropagation();
            setAttributionOpen((prev) => !prev);
          }}
          className={cn(
            "pointer-events-auto flex h-7 w-7 cursor-pointer items-center justify-center rounded-full border border-sp-line bg-sp-surface/95 text-sp-text-secondary shadow-overlay backdrop-blur-sm transition-colors duration-fast ease-sp hover:bg-sp-surface hover:text-sp-text",
            attributionOpen && "bg-sp-surface text-sp-text",
          )}
        >
          <Info className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="pointer-events-none absolute left-3 top-3 z-[500] rounded-sp-md border border-sp-line bg-sp-surface/95 px-3 py-1.5 text-xs shadow-subtle backdrop-blur-sm">
        <div className="font-semibold text-sp-text">裁剪空间视图 · {modeHint}</div>
        <div className="mt-0.5 font-mono text-xs text-sp-text-muted">
          {formatSignedLon(display.west)} · {formatSignedLat(display.south)} · {formatSignedLon(display.east)} · {formatSignedLat(display.north)}
        </div>
      </div>

      {(htmlDragActive || nativeDragActive || dropBusy) && (
        <div
          data-testid="aoi-draw-map-vector-drop-overlay"
          className="pointer-events-none absolute inset-0 z-[800] flex items-center justify-center bg-sp-surface/75 backdrop-blur-xs border-2 border-dashed border-sp-accent rounded-lg p-4 text-center"
        >
          <div className="flex flex-col items-center gap-1.5 text-xs font-medium text-sp-text">
            <Upload className="h-5 w-5 text-sp-accent" />
            <span>{dropBusy ? "正在解析矢量研究区边界…" : "释放以导入矢量研究区 (.shp / .zip / .geojson / .kml)"}</span>
          </div>
        </div>
      )}

      {dropError && !htmlDragActive && !nativeDragActive && (
        <div className="pointer-events-auto absolute bottom-3 left-3 right-3 z-[800] flex items-center justify-between gap-2 rounded-sp-md border border-sp-danger-text/30 bg-sp-surface/95 px-3 py-1.5 text-xs text-sp-danger-text shadow-subtle">
          <span className="truncate">{dropError}</span>
          <button
            type="button"
            onClick={() => setDropError(null)}
            className="shrink-0 text-sp-text-muted hover:text-sp-text cursor-pointer"
          >
            关闭
          </button>
        </div>
      )}
    </div>
  );
}
