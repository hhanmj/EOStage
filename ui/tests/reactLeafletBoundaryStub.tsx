import { createElement, type ReactNode } from "react";

type Capture = {
  fitBounds: unknown[];
  mapEvents: Array<Record<string, (event: unknown) => void>>;
  tileLayers: Array<{
    url: string | null;
    bounds: unknown;
    eventHandlers: Record<string, (event?: unknown) => void>;
  }>;
};

const capture: Capture = { fitBounds: [], mapEvents: [], tileLayers: [] };
const map = {
  fitBounds(bounds: unknown) {
    capture.fitBounds.push(bounds);
  },
  getCenter() {
    return { lat: 0, lng: 0 };
  },
  getZoom() {
    return 10;
  },
  getContainer() {
    return { getBoundingClientRect: () => ({ width: 900, height: 600 }) };
  },
  invalidateSize() {},
  zoomIn() {},
  zoomOut() {},
  dragging: { enabled: () => true, enable() {}, disable() {} },
  doubleClickZoom: { enabled: () => true, enable() {}, disable() {} },
};

export function resetBoundaryCapture() {
  capture.fitBounds.length = 0;
  capture.mapEvents.length = 0;
  capture.tileLayers.length = 0;
}

export function boundaryCapture() {
  return capture;
}

export function MapContainer({
  children,
  scrollWheelZoom: _scrollWheelZoom,
  zoomControl: _zoomControl,
  attributionControl: _attributionControl,
  ...props
}: { children?: ReactNode; [key: string]: unknown }) {
  return createElement("div", { "data-map-container": "true", ...props }, children);
}

export function useMap() {
  return map;
}

export function useMapEvents(events: Record<string, (event: unknown) => void>) {
  capture.mapEvents.push(events);
  return null;
}

function primitive(
  name: string,
  props: Record<string, unknown>,
) {
  const attributes: Record<string, string> = { "data-map-primitive": name };
  if (props.positions) attributes["data-positions"] = JSON.stringify(props.positions);
  if (props.bounds) attributes["data-bounds"] = JSON.stringify(props.bounds);
  if (typeof props.url === "string") attributes["data-url"] = props.url;
  if (typeof props.zIndex === "number") attributes["data-z-index"] = String(props.zIndex);
  if (typeof props.attribution === "string") attributes["data-attribution"] = props.attribution;
  const pathOptions = props.pathOptions && typeof props.pathOptions === "object"
    ? props.pathOptions as Record<string, unknown>
    : null;
  if (typeof pathOptions?.className === "string") attributes["data-class-name"] = pathOptions.className;
  const eventHandlers = props.eventHandlers && typeof props.eventHandlers === "object"
    ? props.eventHandlers as Record<string, unknown>
    : null;
  const onClick = typeof eventHandlers?.click === "function"
    ? () => (eventHandlers.click as (event: unknown) => void)({
        originalEvent: { preventDefault() {}, stopPropagation() {} },
      })
    : undefined;
  return createElement("div", { ...attributes, onClick }, props.children as ReactNode);
}

export function Pane(props: Record<string, unknown>) {
  const style = props.style && typeof props.style === "object"
    ? props.style as Record<string, unknown>
    : {};
  return createElement("div", {
    "data-map-primitive": "pane",
    "data-pane-name": typeof props.name === "string" ? props.name : "",
    "data-pane-z-index": String(style.zIndex ?? ""),
  }, props.children as ReactNode);
}

export function Polygon(props: Record<string, unknown>) {
  return primitive("polygon", props);
}

export function Rectangle(props: Record<string, unknown>) {
  return primitive("rectangle", props);
}

export function TileLayer(props: Record<string, unknown>) {
  const eventHandlers = props.eventHandlers && typeof props.eventHandlers === "object"
    ? props.eventHandlers as Record<string, (event?: unknown) => void>
    : {};
  capture.tileLayers.push({
    url: typeof props.url === "string" ? props.url : null,
    bounds: props.bounds ?? null,
    eventHandlers,
  });
  return primitive("tile", props);
}

export function AttributionControl(props: Record<string, unknown>) {
  return primitive("attribution", props);
}

export function ImageOverlay(props: Record<string, unknown>) {
  return primitive("image-overlay", props);
}

export function Polyline(props: Record<string, unknown>) {
  return primitive("polyline", props);
}

export function CircleMarker(props: Record<string, unknown>) {
  return primitive("circle-marker", props);
}

export function Tooltip(props: Record<string, unknown>) {
  return primitive("tooltip", props);
}

export const __boundaryMap = map;
