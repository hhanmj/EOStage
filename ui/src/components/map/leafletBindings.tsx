/**
 * 自主研发的 Leaflet 绑定层（W5-3d）
 * 严格基于 Leaflet 1.9 原生 API 与 React 标准 Hooks 实现。
 * 遵循 GPL-3.0-or-later 协议，严禁引入任何第三方不兼容绑定代码与内部标识。
 */

import React, {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import L, {
  type ControlPosition,
  type Direction,
  type Evented,
  type LatLngBoundsExpression,
  type LatLngExpression,
  type Layer,
  type LeafletEventHandlerFnMap,
  type Map as LeafletMap,
  type MapOptions,
  type PathOptions,
  type TileLayerOptions,
  type TooltipOptions,
} from "leaflet";

export interface MapBindingState {
  map: LeafletMap;
  pane?: string;
  parentLayer?: any;
}

export const MapScopeContext = createContext<MapBindingState | null>(null);

/**
 * 辅助工具：过滤掉对象中值为 undefined 的属性，避免覆盖 Leaflet 原型默认值
 */
function cleanOptions<T extends Record<string, any>>(opts: T): T {
  const result: any = {};
  for (const key of Object.keys(opts)) {
    if (opts[key] !== undefined) {
      result[key] = opts[key];
    }
  }
  return result as T;
}

/**
 * 辅助 hook：安全绑定 Leaflet 事件并在回调变化或卸载时维护监听
 */
function useLeafletEvents(target: Evented | null, handlers?: LeafletEventHandlerFnMap) {
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  useEffect(() => {
    if (!target || !handlersRef.current) return;
    const active: Array<{ event: string; fn: (e: any) => void }> = [];
    for (const eventName of Object.keys(handlersRef.current)) {
      const listener = (event: any) => {
        const fn = (handlersRef.current as Record<string, any>)?.[eventName];
        if (typeof fn === "function") {
          fn(event);
        }
      };
      target.on(eventName, listener);
      active.push({ event: eventName, fn: listener });
    }
    return () => {
      for (const { event, fn } of active) {
        target.off(event, fn);
      }
    };
  }, [target, handlers ? Object.keys(handlers).sort().join(",") : ""]);
}

export interface MapContainerProps {
  center?: LatLngExpression;
  zoom?: number;
  scrollWheelZoom?: boolean;
  zoomControl?: boolean;
  attributionControl?: boolean;
  className?: string;
  style?: CSSProperties;
  children?: ReactNode;
  [key: string]: unknown;
}

/**
 * MapContainer：负责挂载并持有 L.map 实例，仅在 map 创建后渲染子组件
 */
export function MapContainer({
  center,
  zoom,
  scrollWheelZoom,
  zoomControl,
  attributionControl,
  className,
  style,
  children,
  ...rest
}: MapContainerProps): React.JSX.Element {
  const containerRef = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<LeafletMap | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const options: MapOptions = cleanOptions({
      center,
      zoom,
      scrollWheelZoom,
      zoomControl: zoomControl ?? true,
      attributionControl: attributionControl ?? true,
      ...(rest as MapOptions),
    });

    const instance = L.map(containerRef.current, options);
    setMap(instance);

    return () => {
      instance.remove();
      setMap(null);
    };
  }, []);

  return (
    <div className={className} style={style}>
      <div ref={containerRef} className="h-full w-full">
        {map ? (
          <MapScopeContext.Provider value={{ map }}>
            {children}
          </MapScopeContext.Provider>
        ) : null}
      </div>
    </div>
  );
}

/**
 * 获取当前 MapContainer 上下文中的 LeafletMap 实例
 */
export function useMap(): LeafletMap {
  const ctx = useContext(MapScopeContext);
  if (!ctx || !ctx.map) {
    throw new Error("useMap must be used within a MapContainer");
  }
  return ctx.map;
}

/**
 * 订阅地图事件并保持最新回调
 */
export function useMapEvents(handlers: LeafletEventHandlerFnMap): LeafletMap {
  const map = useMap();
  useLeafletEvents(map, handlers);
  return map;
}

export interface TileLayerProps {
  url: string;
  attribution?: string;
  bounds?: LatLngBoundsExpression;
  detectRetina?: boolean;
  eventHandlers?: LeafletEventHandlerFnMap;
  keepBuffer?: number;
  maxNativeZoom?: number;
  maxZoom?: number;
  opacity?: number;
  referrerPolicy?: ReferrerPolicy | boolean | string;
  subdomains?: string | string[];
  tileSize?: number;
  updateWhenIdle?: boolean;
  updateWhenZooming?: boolean;
  zIndex?: number;
  zoomOffset?: number;
  pane?: string;
}

function stableJsonKey(value: unknown): string {
  try {
    return JSON.stringify(value) ?? "";
  } catch {
    return String(value);
  }
}

function scheduleAfterZoomAnimation(
  map: LeafletMap | undefined,
  apply: () => void,
): (() => void) | undefined {
  if (!map) {
    apply();
    return undefined;
  }
  const mapInternal = map as LeafletMap & { _animatingZoom?: boolean };
  if (!mapInternal._animatingZoom) {
    apply();
    return undefined;
  }
  const onZoomEnd = () => {
    apply();
  };
  map.once("zoomend", onZoomEnd);
  return () => {
    map.off("zoomend", onZoomEnd);
  };
}

/**
 * TileLayer：创建并挂载切片图层，支持动态更新 url、opacity、zIndex
 */
export function TileLayer({
  url,
  attribution,
  bounds,
  detectRetina,
  eventHandlers,
  keepBuffer,
  maxNativeZoom,
  maxZoom,
  opacity,
  referrerPolicy,
  subdomains,
  tileSize,
  updateWhenIdle,
  updateWhenZooming,
  zIndex,
  zoomOffset,
  pane,
}: TileLayerProps): null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const currentPane = pane ?? ctx?.pane;
  const [layer, setLayer] = useState<L.TileLayer | null>(null);
  const lastUrlRef = useRef<string>(url);

  useEffect(() => {
    if (!map) return;

    const options: TileLayerOptions = cleanOptions({
      attribution,
      bounds,
      detectRetina,
      keepBuffer,
      maxNativeZoom,
      maxZoom,
      opacity,
      referrerPolicy: referrerPolicy as any,
      subdomains,
      tileSize,
      updateWhenIdle,
      updateWhenZooming,
      zIndex,
      zoomOffset,
      pane: currentPane,
    });

    lastUrlRef.current = url;
    const instance = L.tileLayer(url, options);
    const retryTimers = new Set<number>();
    const onTileError = (ev: L.TileErrorEvent) => {
      const tile = ev.tile as HTMLImageElement | undefined;
      if (!tile) return;
      const retries = Number(tile.dataset.retryCount || "0");
      if (retries >= 2) return;
      tile.dataset.retryCount = String(retries + 1);
      const rawSrc = tile.dataset.origSrc || tile.getAttribute("src") || tile.src;
      if (!rawSrc || rawSrc.startsWith("data:")) return;
      tile.dataset.origSrc = rawSrc;
      const sep = rawSrc.includes("?") ? "&" : "?";
      const timerId = window.setTimeout(() => {
        retryTimers.delete(timerId);
        if (!tile.isConnected) return;
        tile.src = `${rawSrc}${sep}_sp_retry=${retries + 1}`;
      }, 450 * (retries + 1));
      retryTimers.add(timerId);
    };
    instance.on("tileerror", onTileError);
    instance.addTo(map);
    setLayer(instance);

    return () => {
      for (const id of retryTimers) window.clearTimeout(id);
      retryTimers.clear();
      instance.off("tileerror", onTileError);
      instance.removeFrom(map);
      setLayer(null);
    };
  }, [map, currentPane]);

  useLeafletEvents(layer, eventHandlers);

  // 动态更新 url（仅在 url 实际变化时重设，避免无意义闪刷）
  useEffect(() => {
    if (layer && url && lastUrlRef.current !== url) {
      lastUrlRef.current = url;
      layer.setUrl(url);
    }
  }, [layer, url]);

  // 动态更新 opacity
  useEffect(() => {
    if (layer && typeof opacity === "number") {
      layer.setOpacity(opacity);
    }
  }, [layer, opacity]);

  // 动态更新 zIndex
  useEffect(() => {
    if (layer && typeof zIndex === "number") {
      layer.setZIndex(zIndex);
    }
  }, [layer, zIndex]);

  return null;
}

export interface ImageOverlayProps {
  url: string;
  bounds: LatLngBoundsExpression;
  className?: string;
  clipPath?: string | null;
  eventHandlers?: LeafletEventHandlerFnMap;
  interactive?: boolean;
  opacity?: number;
  pane?: string;
  zIndex?: number;
}

function applyImageElementClipPath(instance: L.ImageOverlay | null, clipPath?: string | null) {
  const el = instance?.getElement?.();
  if (!el) return;
  const normalized = clipPath || "";
  el.style.clipPath = normalized;
  (el.style as CSSStyleDeclaration & { webkitClipPath?: string }).webkitClipPath = normalized;
}

/**
 * ImageOverlay：创建并挂载栅格图层，支持更新 url、bounds、opacity、zIndex、clipPath
 */
export function ImageOverlay({
  url,
  bounds,
  className,
  clipPath,
  eventHandlers,
  interactive,
  opacity,
  pane,
  zIndex,
}: ImageOverlayProps): null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const currentPane = pane ?? ctx?.pane;
  const [layer, setLayer] = useState<L.ImageOverlay | null>(null);
  const clipPathRef = useRef<string | null | undefined>(clipPath);
  clipPathRef.current = clipPath;
  const eventHandlersRef = useRef<LeafletEventHandlerFnMap | undefined>(eventHandlers);
  eventHandlersRef.current = eventHandlers;
  const lastUrlRef = useRef<string>(url);
  const lastBoundsKeyRef = useRef<string>(stableJsonKey(bounds));
  const settledUrlRef = useRef<string | null>(null);

  useEffect(() => {
    if (!map) return;

    const options: L.ImageOverlayOptions = cleanOptions({
      className,
      interactive,
      opacity,
      zIndex,
      pane: currentPane,
    });
    lastUrlRef.current = url;
    settledUrlRef.current = null;
    lastBoundsKeyRef.current = stableJsonKey(bounds);
    const instance = L.imageOverlay(url, bounds, options);

    const dispatchLifecycle = (type: "add" | "load" | "error", ev?: unknown) => {
      if (type === "add" || type === "load") {
        applyImageElementClipPath(instance, clipPathRef.current);
      }
      if (type === "load" || type === "error") {
        if (settledUrlRef.current === `${type}:${lastUrlRef.current}`) return;
        settledUrlRef.current = `${type}:${lastUrlRef.current}`;
      }
      const fn = (eventHandlersRef.current as Record<string, ((event: any) => void) | undefined> | undefined)?.[type];
      fn?.(ev ?? { type, target: instance });
    };

    const onInternalAdd = (ev: unknown) => dispatchLifecycle("add", ev);
    const onInternalLoad = (ev: unknown) => dispatchLifecycle("load", ev);
    const onInternalError = (ev: unknown) => dispatchLifecycle("error", ev);
    instance.on("add", onInternalAdd);
    instance.on("load", onInternalLoad);
    instance.on("error", onInternalError);
    instance.addTo(map);
    applyImageElementClipPath(instance, clipPathRef.current);

    // 若浏览器命中缓存或 data URL 在 addTo 内同步就绪，立即补发 load，防止 opacity 卡在 0
    const imgEl = instance.getElement();
    if (imgEl && imgEl.complete && imgEl.naturalWidth > 0) {
      dispatchLifecycle("load");
    }
    setLayer(instance);

    return () => {
      instance.off("add", onInternalAdd);
      instance.off("load", onInternalLoad);
      instance.off("error", onInternalError);
      instance.removeFrom(map);
      setLayer(null);
    };
  }, [map, currentPane]);

  const delegatedHandlers = React.useMemo(() => {
    if (!eventHandlers) return undefined;
    const rest: LeafletEventHandlerFnMap = {};
    for (const key of Object.keys(eventHandlers)) {
      if (key !== "add" && key !== "load" && key !== "error") {
        (rest as Record<string, unknown>)[key] = (eventHandlers as Record<string, unknown>)[key];
      }
    }
    return Object.keys(rest).length > 0 ? rest : undefined;
  }, [eventHandlers]);

  useLeafletEvents(layer, delegatedHandlers);

  useEffect(() => {
    applyImageElementClipPath(layer, clipPath);
  }, [layer, clipPath]);

  useEffect(() => {
    if (layer && url && lastUrlRef.current !== url) {
      lastUrlRef.current = url;
      settledUrlRef.current = null;
      layer.setUrl(url);
      const imgEl = layer.getElement();
      if (imgEl && imgEl.complete && imgEl.naturalWidth > 0) {
        if (settledUrlRef.current !== `load:${url}`) {
          settledUrlRef.current = `load:${url}`;
          applyImageElementClipPath(layer, clipPathRef.current);
          const fn = (eventHandlersRef.current as Record<string, ((event: any) => void) | undefined> | undefined)?.load;
          fn?.({ type: "load", target: layer });
        }
      }
    }
  }, [layer, url]);

  useEffect(() => {
    if (!layer || !bounds) return;
    const nextKey = stableJsonKey(bounds);
    if (nextKey === lastBoundsKeyRef.current) return;
    lastBoundsKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setBounds(L.latLngBounds(bounds as any));
    });
  }, [layer, map, bounds]);

  useEffect(() => {
    if (layer && typeof opacity === "number") {
      layer.setOpacity(opacity);
    }
  }, [layer, opacity]);

  useEffect(() => {
    if (layer && typeof zIndex === "number") {
      layer.setZIndex(zIndex);
    }
  }, [layer, zIndex]);

  return null;
}

export interface PathComponentProps {
  pathOptions?: PathOptions;
  bubblingMouseEvents?: boolean;
  interactive?: boolean;
  pane?: string;
  eventHandlers?: LeafletEventHandlerFnMap;
  children?: ReactNode;
}

export interface PolygonProps extends PathComponentProps {
  positions: LatLngExpression[] | LatLngExpression[][] | LatLngExpression[][][];
}

/**
 * Polygon：多边形矢量图层
 */
export function Polygon({
  positions,
  pathOptions,
  bubblingMouseEvents,
  interactive,
  pane,
  eventHandlers,
  children,
}: PolygonProps): React.JSX.Element | null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const currentPane = pane ?? ctx?.pane;
  const [layer, setLayer] = useState<L.Polygon | null>(null);
  const lastPositionsKeyRef = useRef<string>(stableJsonKey(positions));
  const lastStyleKeyRef = useRef<string>(stableJsonKey(pathOptions));

  useEffect(() => {
    if (!map) return;

    const options: L.PolylineOptions = cleanOptions({
      bubblingMouseEvents,
      interactive,
      pane: currentPane,
      ...pathOptions,
    });
    lastPositionsKeyRef.current = stableJsonKey(positions);
    lastStyleKeyRef.current = stableJsonKey(pathOptions);
    const instance = L.polygon(positions, options);
    instance.addTo(map);
    setLayer(instance);

    return () => {
      instance.removeFrom(map);
      setLayer(null);
    };
  }, [map, currentPane]);

  useLeafletEvents(layer, eventHandlers);

  useEffect(() => {
    if (!layer || !positions) return;
    const nextKey = stableJsonKey(positions);
    if (nextKey === lastPositionsKeyRef.current) return;
    lastPositionsKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setLatLngs(positions);
    });
  }, [layer, map, positions]);

  useEffect(() => {
    if (!layer || !pathOptions) return;
    const nextKey = stableJsonKey(pathOptions);
    if (nextKey === lastStyleKeyRef.current) return;
    lastStyleKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setStyle(pathOptions);
    });
  }, [layer, map, pathOptions]);

  if (!layer || !children || !map) return null;

  return (
    <MapScopeContext.Provider value={{ map, pane: currentPane, parentLayer: layer }}>
      {children}
    </MapScopeContext.Provider>
  );
}

export interface PolylineProps extends PathComponentProps {
  positions: LatLngExpression[] | LatLngExpression[][];
}

/**
 * Polyline：折线矢量图层
 */
export function Polyline({
  positions,
  pathOptions,
  bubblingMouseEvents,
  interactive,
  pane,
  eventHandlers,
  children,
}: PolylineProps): React.JSX.Element | null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const currentPane = pane ?? ctx?.pane;
  const [layer, setLayer] = useState<L.Polyline | null>(null);
  const lastPositionsKeyRef = useRef<string>(stableJsonKey(positions));
  const lastStyleKeyRef = useRef<string>(stableJsonKey(pathOptions));

  useEffect(() => {
    if (!map) return;

    const options: L.PolylineOptions = cleanOptions({
      bubblingMouseEvents,
      interactive,
      pane: currentPane,
      ...pathOptions,
    });
    lastPositionsKeyRef.current = stableJsonKey(positions);
    lastStyleKeyRef.current = stableJsonKey(pathOptions);
    const instance = L.polyline(positions, options);
    instance.addTo(map);
    setLayer(instance);

    return () => {
      instance.removeFrom(map);
      setLayer(null);
    };
  }, [map, currentPane]);

  useLeafletEvents(layer, eventHandlers);

  useEffect(() => {
    if (!layer || !positions) return;
    const nextKey = stableJsonKey(positions);
    if (nextKey === lastPositionsKeyRef.current) return;
    lastPositionsKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setLatLngs(positions);
    });
  }, [layer, map, positions]);

  useEffect(() => {
    if (!layer || !pathOptions) return;
    const nextKey = stableJsonKey(pathOptions);
    if (nextKey === lastStyleKeyRef.current) return;
    lastStyleKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setStyle(pathOptions);
    });
  }, [layer, map, pathOptions]);

  if (!layer || !children || !map) return null;

  return (
    <MapScopeContext.Provider value={{ map, pane: currentPane, parentLayer: layer }}>
      {children}
    </MapScopeContext.Provider>
  );
}

export interface RectangleProps extends PathComponentProps {
  bounds: LatLngBoundsExpression;
}

/**
 * Rectangle：矩形矢量图层
 */
export function Rectangle({
  bounds,
  pathOptions,
  bubblingMouseEvents,
  interactive,
  pane,
  eventHandlers,
  children,
}: RectangleProps): React.JSX.Element | null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const currentPane = pane ?? ctx?.pane;
  const [layer, setLayer] = useState<L.Rectangle | null>(null);
  const lastBoundsKeyRef = useRef<string>(stableJsonKey(bounds));
  const lastStyleKeyRef = useRef<string>(stableJsonKey(pathOptions));

  useEffect(() => {
    if (!map) return;

    const options: L.PolylineOptions = cleanOptions({
      bubblingMouseEvents,
      interactive,
      pane: currentPane,
      ...pathOptions,
    });
    lastBoundsKeyRef.current = stableJsonKey(bounds);
    lastStyleKeyRef.current = stableJsonKey(pathOptions);
    const instance = L.rectangle(bounds, options);
    instance.addTo(map);
    setLayer(instance);

    return () => {
      instance.removeFrom(map);
      setLayer(null);
    };
  }, [map, currentPane]);

  useLeafletEvents(layer, eventHandlers);

  useEffect(() => {
    if (!layer || !bounds) return;
    const nextKey = stableJsonKey(bounds);
    if (nextKey === lastBoundsKeyRef.current) return;
    lastBoundsKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setBounds(bounds);
    });
  }, [layer, map, bounds]);

  useEffect(() => {
    if (!layer || !pathOptions) return;
    const nextKey = stableJsonKey(pathOptions);
    if (nextKey === lastStyleKeyRef.current) return;
    lastStyleKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setStyle(pathOptions);
    });
  }, [layer, map, pathOptions]);

  if (!layer || !children || !map) return null;

  return (
    <MapScopeContext.Provider value={{ map, pane: currentPane, parentLayer: layer }}>
      {children}
    </MapScopeContext.Provider>
  );
}

export interface CircleMarkerProps extends PathComponentProps {
  center: LatLngExpression;
  radius: number;
}

/**
 * CircleMarker：圆点矢量图层
 */
export function CircleMarker({
  center,
  radius,
  pathOptions,
  bubblingMouseEvents,
  interactive,
  pane,
  eventHandlers,
  children,
}: CircleMarkerProps): React.JSX.Element | null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const currentPane = pane ?? ctx?.pane;
  const [layer, setLayer] = useState<L.CircleMarker | null>(null);
  const lastCenterKeyRef = useRef<string>(stableJsonKey(center));
  const lastStyleKeyRef = useRef<string>(stableJsonKey(pathOptions));

  useEffect(() => {
    if (!map) return;

    const options: L.CircleMarkerOptions = cleanOptions({
      radius,
      bubblingMouseEvents,
      interactive,
      pane: currentPane,
      ...pathOptions,
    });
    lastCenterKeyRef.current = stableJsonKey(center);
    lastStyleKeyRef.current = stableJsonKey(pathOptions);
    const instance = L.circleMarker(center, options);
    instance.addTo(map);
    setLayer(instance);

    return () => {
      instance.removeFrom(map);
      setLayer(null);
    };
  }, [map, currentPane]);

  useLeafletEvents(layer, eventHandlers);

  useEffect(() => {
    if (!layer || !center) return;
    const nextKey = stableJsonKey(center);
    if (nextKey === lastCenterKeyRef.current) return;
    lastCenterKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setLatLng(center);
    });
  }, [layer, map, center]);

  useEffect(() => {
    if (layer && typeof radius === "number") {
      layer.setRadius(radius);
    }
  }, [layer, radius]);

  useEffect(() => {
    if (!layer || !pathOptions) return;
    const nextKey = stableJsonKey(pathOptions);
    if (nextKey === lastStyleKeyRef.current) return;
    lastStyleKeyRef.current = nextKey;
    return scheduleAfterZoomAnimation(map, () => {
      layer.setStyle(pathOptions);
    });
  }, [layer, map, pathOptions]);

  if (!layer || !children || !map) return null;

  return (
    <MapScopeContext.Provider value={{ map, pane: currentPane, parentLayer: layer }}>
      {children}
    </MapScopeContext.Provider>
  );
}

export interface PaneProps {
  name: string;
  style?: CSSProperties;
  children?: ReactNode;
}

/**
 * Pane：创建并管理独立地图面板，向下传递 pane 作用域
 */
export function Pane({ name, style, children }: PaneProps): React.JSX.Element | null {
  const ctx = useContext(MapScopeContext);
  const map = ctx?.map;
  const parentPane = ctx?.pane;
  const [paneCreated, setPaneCreated] = useState(false);

  useEffect(() => {
    if (!map) return;

    let paneEl = map.getPane(name);
    if (!paneEl) {
      paneEl = map.createPane(name, parentPane ? map.getPane(parentPane) : undefined);
    }

    if (paneEl && style) {
      Object.assign(paneEl.style, style);
    }

    setPaneCreated(true);

    return () => {
      if (paneEl) {
        paneEl.remove();
      }
      // Leaflet 私有属性清理：从 map 的 pane 注册表和渲染器缓存中注销，确保再次挂载时创建全新 Attached 的面板
      type MapInternals = {
        _panes?: Record<string, HTMLElement>;
        _paneRenderers?: Record<string, any>;
      };
      const internals = map as unknown as MapInternals;
      if (internals._panes && name in internals._panes) {
        delete internals._panes[name];
      }
      if (internals._paneRenderers && name in internals._paneRenderers) {
        const renderer = internals._paneRenderers[name];
        if (renderer && typeof renderer.remove === "function") {
          renderer.remove();
        }
        delete internals._paneRenderers[name];
      }
      setPaneCreated(false);
    };
  }, [map, name, parentPane]);

  if (!map || !paneCreated) return null;

  return (
    <MapScopeContext.Provider value={{ map, pane: name }}>
      {children}
    </MapScopeContext.Provider>
  );
}

export interface AttributionControlProps {
  position?: ControlPosition;
  prefix?: string | boolean;
}

/**
 * AttributionControl：地图版权信息控件
 */
export function AttributionControl({
  position,
  prefix,
}: AttributionControlProps): null {
  const map = useMap();

  useEffect(() => {
    const options = cleanOptions({ position, prefix });
    const control = L.control.attribution(options);
    control.addTo(map);

    return () => {
      control.remove();
    };
  }, [map, position, prefix]);

  return null;
}

export interface ZoomControlProps {
  position?: ControlPosition;
}

/**
 * ZoomControl：缩放按钮控件
 */
export function ZoomControl({ position }: ZoomControlProps): null {
  const map = useMap();

  useEffect(() => {
    const options = cleanOptions({ position });
    const control = L.control.zoom(options);
    control.addTo(map);

    return () => {
      control.remove();
    };
  }, [map, position]);

  return null;
}

export interface TooltipProps {
  children?: ReactNode;
  pane?: string;
  direction?: Direction;
  permanent?: boolean;
  sticky?: boolean;
  interactive?: boolean;
  opacity?: number;
  className?: string;
}

/**
 * Tooltip：绑定到父矢量图层的信息提示气泡
 */
export function Tooltip({
  children,
  pane,
  direction,
  permanent,
  sticky,
  interactive,
  opacity,
  className,
}: TooltipProps): React.JSX.Element | null {
  const ctx = useContext(MapScopeContext);
  const parentLayer = ctx?.parentLayer as Layer | undefined;
  const currentPane = pane ?? ctx?.pane;

  const containerRef = useRef<HTMLDivElement | null>(null);
  const isTextContent = typeof children === "string" || typeof children === "number";

  useEffect(() => {
    if (!parentLayer || !("bindTooltip" in parentLayer)) return;

    const options: TooltipOptions = cleanOptions({
      pane: currentPane,
      direction,
      permanent,
      sticky,
      interactive,
      opacity,
      className,
    });

    if (isTextContent) {
      (parentLayer as any).bindTooltip(String(children), options);
    } else {
      if (!containerRef.current) {
        containerRef.current = document.createElement("div");
      }
      (parentLayer as any).bindTooltip(containerRef.current, options);
    }

    return () => {
      if ("unbindTooltip" in parentLayer) {
        (parentLayer as any).unbindTooltip();
      }
    };
  }, [parentLayer, currentPane, direction, permanent, sticky, interactive, opacity, className]);

  useEffect(() => {
    if (!parentLayer || !("setTooltipContent" in parentLayer)) return;
    if (isTextContent) {
      (parentLayer as any).setTooltipContent(String(children));
    }
  }, [parentLayer, isTextContent, children]);

  if (!isTextContent && containerRef.current) {
    return createPortal(children, containerRef.current);
  }

  return null;
}
