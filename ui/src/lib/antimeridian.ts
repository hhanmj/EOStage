import type { Bbox, Json } from "./bridge.ts";

/**
 * Render-only counterpart of Core's bounded longitude envelope.
 *
 * The native envelope remains the authority for request planning and is
 * consumed from the optional CoreRestore display field when available. This
 * helper is only the legacy/browser fallback: it mirrors the shared
 * Hubei/New Zealand fixture contract, admits an explicit full-world edge, and
 * fails closed for exact-180, wide, or uncertain topology. It never rewrites
 * stored geometry or produces request payloads.
 */
export type WrappedAoiEnvelope = {
  south: number;
  north: number;
  crossesAntimeridian: boolean;
  displayWest: number;
  displayEast: number;
  requestWindows: readonly Bbox[];
};

type Position = [number, number, ...number[]];
type UnwrappedPosition = { longitude: number; latitude: number; ordinates: number[] };
type ParsedRing = {
  raw: Position[];
  unwrapped: UnwrappedPosition[];
  west: number;
  east: number;
  center: number;
};

type PolygonCoordinates = unknown[][][];

const MAX_LEGACY_GEOMETRY_BYTES = 8 * 1024 * 1024;
const UTF8_ENCODER = new TextEncoder();

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function finitePosition(value: unknown): Position | null {
  if (
    !Array.isArray(value)
    || value.length < 2
    || value.some((ordinate) => typeof ordinate !== "number" || !Number.isFinite(ordinate))
  ) {
    return null;
  }
  const longitude = value[0] as number;
  const latitude = value[1] as number;
  if (longitude < -180 || longitude > 180 || latitude < -90 || latitude > 90) return null;
  return value as Position;
}

function parseRing(value: unknown): ParsedRing | null {
  if (!Array.isArray(value) || value.length < 4) return null;
  const raw = value.map(finitePosition);
  if (raw.some((position) => position === null)) return null;
  const positions = raw as Position[];
  const first = positions[0]!;
  const last = positions[positions.length - 1]!;
  if (
    first.length !== last.length
    || first.some((ordinate, index) => ordinate !== last[index])
  ) return null;
  const distinct = new Set(positions.slice(0, -1).map(([longitude, latitude]) => `${longitude},${latitude}`));
  if (distinct.size < 3) return null;

  const unwrapped: UnwrappedPosition[] = [{
    longitude: first[0],
    latitude: first[1],
    ordinates: [...first],
  }];
  for (let index = 1; index < positions.length; index += 1) {
    const previous = unwrapped[index - 1]!;
    const current = positions[index]!;
    let longitude = current[0];
    const previousRaw = positions[index - 1]!;
    const rawDelta = longitude - previousRaw[0];
    if (rawDelta !== 360 && rawDelta !== -360) {
      while (longitude - previous.longitude > 180) longitude -= 360;
      while (longitude - previous.longitude < -180) longitude += 360;
    }
    unwrapped.push({ longitude, latitude: current[1], ordinates: [...current] });
  }
  const closing = unwrapped[unwrapped.length - 1]!;
  const explicitFullWorldEdge = positions.some((position, index) => {
    const next = positions[index + 1];
    return next !== undefined && Math.abs(next[0] - position[0]) === 360;
  });
  if (closing.longitude !== unwrapped[0]!.longitude || closing.latitude !== unwrapped[0]!.latitude) {
    if (!explicitFullWorldEdge || closing.latitude !== unwrapped[0]!.latitude) return null;
    closing.longitude = unwrapped[0]!.longitude;
  }
  const area = unwrapped.slice(0, -1).reduce((sum, point, index) => {
    const next = unwrapped[index + 1]!;
    return sum + point.longitude * next.latitude - next.longitude * point.latitude;
  }, 0);
  if (area === 0) return null;
  let west = Infinity;
  let east = -Infinity;
  for (const point of unwrapped) {
    west = Math.min(west, point.longitude);
    east = Math.max(east, point.longitude);
  }
  return {
    raw: positions,
    unwrapped,
    west,
    east,
    center: (west + east) / 2,
  };
}

function polygonCoordinates(value: unknown): PolygonCoordinates | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  return value as PolygonCoordinates;
}

function polygonParts(value: unknown): PolygonCoordinates[] {
  const pending: unknown[] = [value];
  const parts: PolygonCoordinates[] = [];
  while (pending.length) {
    const geometry = record(pending.pop());
    if (!geometry) continue;
    const type = geometry.type;
    if (type === "Polygon") {
      const coordinates = polygonCoordinates(geometry.coordinates);
      if (coordinates) parts.push(coordinates);
    } else if (type === "MultiPolygon" && Array.isArray(geometry.coordinates)) {
      for (const polygon of geometry.coordinates) {
        const coordinates = polygonCoordinates(polygon);
        if (coordinates) parts.push(coordinates);
      }
    } else if (type === "Feature") {
      pending.push(geometry.geometry);
    } else if (type === "FeatureCollection" && Array.isArray(geometry.features)) {
      for (const feature of geometry.features) pending.push(feature);
    } else if (type === "GeometryCollection" && Array.isArray(geometry.geometries)) {
      for (const item of geometry.geometries) pending.push(item);
    }
  }
  return parts;
}

function allRings(value: unknown): unknown[] {
  return polygonParts(value).flatMap((polygon) => polygon);
}

function hasExplicitFullWorldEdge(value: unknown): boolean {
  return allRings(value).some((rawRing) => {
    if (!Array.isArray(rawRing)) return false;
    const positions = rawRing.map(finitePosition);
    if (positions.some((position) => position === null)) return false;
    const valid = positions as Position[];
    return valid.some((position, index) => {
      const next = valid[index + 1];
      return next !== undefined && Math.abs(next[0] - position[0]) === 360;
    });
  });
}

function parsePolygons(value: unknown): { polygons: ParsedRing[][]; south: number; north: number } | null {
  const parts = polygonParts(value);
  if (!parts.length) return null;
  const polygons: ParsedRing[][] = [];
  let south = Infinity;
  let north = -Infinity;
  for (const part of parts) {
    if (!part.length) return null;
    const rings = part.map(parseRing);
    if (rings.some((ring) => ring === null)) return null;
    const parsed = rings as ParsedRing[];
    for (const ring of parsed) {
      for (const position of ring.raw) {
        south = Math.min(south, position[1]);
        north = Math.max(north, position[1]);
      }
    }
    polygons.push(parsed);
  }
  if (!Number.isFinite(south) || !Number.isFinite(north) || south >= north) return null;
  return { polygons, south, north };
}

function canonicalLongitude(value: number): number {
  // Polygon coordinates arrive in the canonical geographic world already.
  // Preserve those exact source doubles: modulo arithmetic can move a decimal
  // endpoint by one ulp, after which the strict render-domain parity check
  // rejects the projection that produced its own envelope. Only unwrapped
  // adjacent-world endpoints need an integer-world shift.
  let normalized = value;
  if (normalized < -180 || normalized >= 180) {
    const worlds = Math.floor((normalized + 180) / 360);
    normalized -= worlds * 360;
    if (normalized >= 180) normalized -= 360;
    if (normalized < -180) normalized += 360;
  }
  return normalized === 0 ? 0 : normalized;
}

/** Canonicalize a Leaflet world-copy longitude at the user-authority boundary. */
export function canonicalLeafletLongitude(value: number): number | null {
  if (!Number.isFinite(value)) return null;
  const canonical = canonicalLongitude(value);
  return Number(canonical.toFixed(12));
}

export type CanonicalDrawRectangle =
  | { kind: "bbox"; bbox: Bbox }
  | { kind: "polygon"; ring: [number, number][] };

function finiteDrawPosition(value: readonly [number, number]): boolean {
  return Number.isFinite(value[0]) && Number.isFinite(value[1]) && value[1] >= -90 && value[1] <= 90;
}

/**
 * Convert one locally continuous Leaflet sketch to canonical EPSG:4326.
 * Adjacency is unwrapped first; modulo is applied only after one shared world
 * has been selected, so a dateline ring retains its two-window semantics.
 */
export function canonicalizeLeafletPolygonRing(
  rawPoints: readonly (readonly [number, number])[],
): [number, number][] | null {
  if (rawPoints.length < 3 || rawPoints.some((point) => !finiteDrawPosition(point))) return null;
  const points = rawPoints.slice();
  const firstRaw = points[0]!;
  const lastRaw = points.at(-1)!;
  if (firstRaw[0] === lastRaw[0] && firstRaw[1] === lastRaw[1]) points.pop();
  if (points.length < 3) return null;

  const unwrapped: [number, number][] = [[points[0]![0], points[0]![1]]];
  for (let index = 1; index < points.length; index += 1) {
    const point = points[index]!;
    const previous = unwrapped[index - 1]!;
    let longitude = point[0];
    while (longitude - previous[0] > 180) longitude -= 360;
    while (longitude - previous[0] < -180) longitude += 360;
    unwrapped.push([longitude, point[1]]);
  }
  const longitudes = unwrapped.map(([longitude]) => longitude);
  const west = Math.min(...longitudes);
  const east = Math.max(...longitudes);
  if (!Number.isFinite(west) || !Number.isFinite(east) || east - west >= 360) return null;
  const center = (west + east) / 2;
  const world = Math.floor((center + 180) / 360);
  const ring = unwrapped.map(([longitude, latitude]) => {
    const canonical = canonicalLeafletLongitude(longitude - world * 360);
    return canonical === null ? null : [canonical, latitude] as [number, number];
  });
  if (ring.some((point) => point === null)) return null;
  const canonicalRing = ring as [number, number][];
  const distinct = new Set(canonicalRing.map(([longitude, latitude]) => `${longitude},${latitude}`));
  if (distinct.size < 3) return null;
  canonicalRing.push([...canonicalRing[0]!] as [number, number]);
  return canonicalRing;
}

export function canonicalizeLeafletRectangle(
  start: readonly [number, number],
  end: readonly [number, number],
): CanonicalDrawRectangle | null {
  if (!finiteDrawPosition(start) || !finiteDrawPosition(end)) return null;
  const south = Math.min(start[1], end[1]);
  const north = Math.max(start[1], end[1]);
  if (north - south <= 1e-5) return null;
  let endLongitude = end[0];
  while (endLongitude - start[0] > 180) endLongitude -= 360;
  while (endLongitude - start[0] < -180) endLongitude += 360;
  const west = Math.min(start[0], endLongitude);
  const east = Math.max(start[0], endLongitude);
  if (east - west <= 1e-5 || east - west > 180) return null;
  const ring = canonicalizeLeafletPolygonRing([
    [west, south], [east, south], [east, north], [west, north],
  ]);
  if (!ring) return null;
  const ringLongitudes = ring.slice(0, -1).map(([longitude]) => longitude);
  const canonicalWest = Math.min(...ringLongitudes);
  const canonicalEast = Math.max(...ringLongitudes);
  const crossesAntimeridian = ring.some((point, index) => {
    const next = ring[index + 1];
    return next !== undefined && Math.abs(next[0] - point[0]) > 180;
  });
  if (crossesAntimeridian) return { kind: "polygon", ring };
  return {
    kind: "bbox",
    bbox: { west: canonicalWest, east: canonicalEast, south, north, crs: "EPSG:4326" },
  };
}

function uniqueSorted(values: number[]): number[] {
  return Array.from(new Set(values.map(canonicalLongitude))).sort((left, right) => left - right);
}

function displayInterval(polygons: ParsedRing[][]): [number, number] | null {
  const boundaries: number[] = [];
  for (const polygon of polygons) {
    const exterior = polygon[0];
    if (!exterior) return null;
    const alignedRings = polygon.map((ring) => {
      const shift = Math.round((exterior.center - ring.center) / 360);
      return {
        west: ring.west + shift * 360,
        east: ring.east + shift * 360,
      };
    });
    let west = Infinity;
    let east = -Infinity;
    for (const ring of alignedRings) {
      west = Math.min(west, ring.west);
      east = Math.max(east, ring.east);
    }
    if (!(east > west) || east - west > 360) return null;
    boundaries.push(west, east);
  }
  const points = uniqueSorted(boundaries);
  if (points.length < 2) return null;
  let largestGap = -Infinity;
  let largestGapIndex = -1;
  for (let index = 0; index < points.length; index += 1) {
    const nextIndex = (index + 1) % points.length;
    const next = nextIndex === 0 ? points[0]! + 360 : points[nextIndex]!;
    const gap = next - points[index]!;
    if (gap > largestGap) {
      largestGap = gap;
      largestGapIndex = index;
    }
  }
  if (largestGapIndex < 0) return null;
  const startIndex = (largestGapIndex + 1) % points.length;
  // `points` is already canonical in [-180, 180). When the largest gap is
  // the wraparound gap, the local interval is simply [first, last]. Do not
  // add and subtract a world from both endpoints: that can move an ordinary
  // provider decimal extremum by one ulp and make the strict render parity
  // check reject its own geometry.
  const west = points[startIndex]!;
  const east = points[largestGapIndex]!
    + (largestGapIndex < startIndex ? 360 : 0);
  const width = east - west;
  return width > 0 && width < 360 ? [west, east] : null;
}

function requestWindows(west: number, east: number, south: number, north: number): Bbox[] | null {
  if (east <= 180) {
    return [{ west, south, east, north, crs: "EPSG:4326" }];
  }
  const secondEast = east - 360;
  if (west >= 180 || secondEast <= -180) return null;
  return [
    { west, south, east: 180, north, crs: "EPSG:4326" },
    { west: -180, south, east: secondEast, north, crs: "EPSG:4326" },
  ];
}

export function deriveWrappedAoiEnvelope(geometry: Json): WrappedAoiEnvelope | null {
  let serializedBytes: number;
  try {
    const serialized = JSON.stringify(geometry);
    if (typeof serialized !== "string") return null;
    serializedBytes = UTF8_ENCODER.encode(serialized).byteLength;
  } catch {
    return null;
  }
  if (serializedBytes > MAX_LEGACY_GEOMETRY_BYTES) return null;
  const parsed = parsePolygons(geometry);
  if (!parsed) return null;
  if (hasExplicitFullWorldEdge(geometry)) {
    return {
      south: parsed.south,
      north: parsed.north,
      crossesAntimeridian: false,
      displayWest: -180,
      displayEast: 180,
      requestWindows: [{ west: -180, south: parsed.south, east: 180, north: parsed.north, crs: "EPSG:4326" }],
    };
  }
  const interval = displayInterval(parsed.polygons);
  if (!interval) return null;
  // The native implementation owns exact-180 tie-breaking and wide-domain
  // topology. Without its derived envelope, rotating either case to an
  // arbitrary periodic copy is not provably safe, so leave it on the legacy
  // raw path. The explicit full-world edge above is the only wide fallback
  // that is unambiguous from the bounded GeoJSON wire alone.
  if (interval[1] - interval[0] >= 180) return null;
  const windows = requestWindows(interval[0], interval[1], parsed.south, parsed.north);
  if (!windows || windows.length < 1 || windows.length > 2) return null;
  const envelope: WrappedAoiEnvelope = {
    south: parsed.south,
    north: parsed.north,
    crossesAntimeridian: windows.length === 2,
    displayWest: interval[0],
    displayEast: interval[1],
    requestWindows: windows,
  };
  return shiftPolygonalGeojsonToDisplayDomain(geometry, envelope) ? envelope : null;
}

export function continuousDisplayBbox(envelope: WrappedAoiEnvelope): Bbox {
  return {
    west: envelope.displayWest,
    south: envelope.south,
    east: envelope.displayEast,
    north: envelope.north,
    crs: "EPSG:4326",
  };
}

/**
 * Render-only DEM envelope in the AOI's continuous periodic display domain.
 * Acquisition still uses the canonical request windows owned by Core; this
 * rectangle exists only so a canonical-wide antimeridian bbox is not hidden.
 */
export function expandedDemDisplayBbox(
  envelope: WrappedAoiEnvelope,
  bufferDegrees: number,
): Bbox {
  const buffer = Number.isFinite(bufferDegrees) && bufferDegrees > 0 ? bufferDegrees : 0;
  return {
    west: envelope.displayWest - buffer,
    south: Math.max(-90, envelope.south - buffer),
    east: envelope.displayEast + buffer,
    north: Math.min(90, envelope.north + buffer),
    crs: "EPSG:4326",
  };
}

function cloneJson(value: unknown): unknown {
  if (!value || typeof value !== "object") return value;
  const root: unknown = Array.isArray(value) ? [] : {};
  const pending: Array<{ source: object; target: object }> = [{ source: value, target: root as object }];
  while (pending.length) {
    const { source, target } = pending.pop()!;
    if (Array.isArray(source) && Array.isArray(target)) {
      for (const item of source) {
        if (item && typeof item === "object") {
          const child: unknown = Array.isArray(item) ? [] : {};
          target.push(child);
          pending.push({ source: item, target: child as object });
        } else {
          target.push(item);
        }
      }
    } else if (!Array.isArray(source) && !Array.isArray(target)) {
      const targetRecord = target as Record<string, unknown>;
      for (const [key, item] of Object.entries(source)) {
        if (item && typeof item === "object") {
          const child: unknown = Array.isArray(item) ? [] : {};
          targetRecord[key] = child;
          pending.push({ source: item, target: child as object });
        } else {
          targetRecord[key] = item;
        }
      }
    }
  }
  return root;
}

function chooseWorldShift(west: number, east: number, envelope: WrappedAoiEnvelope): number {
  const lower = Math.ceil((envelope.displayWest - west) / 360);
  const upper = Math.floor((envelope.displayEast - east) / 360);
  if (lower <= upper) {
    if (lower <= 0 && upper >= 0) return 0;
    return Math.abs(lower) < Math.abs(upper) ? lower : upper;
  }
  return Math.round(((envelope.displayWest + envelope.displayEast) / 2 - (west + east) / 2) / 360);
}

function projectedPolygonCoordinates(value: unknown, envelope: WrappedAoiEnvelope): PolygonCoordinates | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  const rings = value.map(parseRing);
  if (rings.some((ring) => ring === null)) return null;
  const parsed = rings as ParsedRing[];
  const exterior = parsed[0]!;
  const componentRings = parsed.map((ring) => {
    const relativeShift = Math.round((exterior.center - ring.center) / 360);
    return ring.unwrapped.map((point) => ({
      ...point,
      longitude: point.longitude + relativeShift * 360,
    }));
  });
  let west = Infinity;
  let east = -Infinity;
  for (const ring of componentRings) {
    for (const point of ring) {
      west = Math.min(west, point.longitude);
      east = Math.max(east, point.longitude);
    }
  }
  const shift = chooseWorldShift(west, east, envelope);
  return componentRings.map((ring) => ring.map((point) => {
    const longitude = point.longitude + shift * 360;
    return [longitude, point.latitude, ...point.ordinates.slice(2)];
  }));
}

function shiftGeometryInPlace(value: unknown, envelope: WrappedAoiEnvelope): boolean {
  const pending: unknown[] = [value];
  let visited = false;
  while (pending.length) {
    const geometry = record(pending.pop());
    if (!geometry) return false;
    visited = true;
    if (geometry.type === "Polygon") {
      const coordinates = projectedPolygonCoordinates(geometry.coordinates, envelope);
      if (!coordinates) return false;
      geometry.coordinates = coordinates;
    } else if (geometry.type === "MultiPolygon" && Array.isArray(geometry.coordinates)) {
      const polygons = geometry.coordinates.map((polygon) => projectedPolygonCoordinates(polygon, envelope));
      if (polygons.some((polygon) => polygon === null)) return false;
      geometry.coordinates = polygons;
    } else if (geometry.type === "Feature") {
      pending.push(geometry.geometry);
    } else if (geometry.type === "FeatureCollection" && Array.isArray(geometry.features)) {
      for (const feature of geometry.features) pending.push(feature);
    } else if (geometry.type === "GeometryCollection" && Array.isArray(geometry.geometries)) {
      for (const item of geometry.geometries) pending.push(item);
    } else {
      return false;
    }
  }
  return visited;
}

function projectedCoordinates(value: unknown): [number, number][] | null {
  const positions: [number, number][] = [];
  for (const polygon of polygonParts(value)) {
    for (const rawRing of polygon) {
      if (!Array.isArray(rawRing)) return null;
      for (const rawPosition of rawRing) {
        if (
          !Array.isArray(rawPosition)
          || rawPosition.length < 2
          || typeof rawPosition[0] !== "number"
          || typeof rawPosition[1] !== "number"
          || !Number.isFinite(rawPosition[0])
          || !Number.isFinite(rawPosition[1])
          || rawPosition[1] < -90
          || rawPosition[1] > 90
        ) return null;
        positions.push([rawPosition[0], rawPosition[1]]);
      }
    }
  }
  return positions.length > 0 ? positions : null;
}

export function shiftPolygonalGeojsonToLongitudeReference(
  geometry: Json,
  envelope: WrappedAoiEnvelope,
): Json | null {
  const width = envelope.displayEast - envelope.displayWest;
  if (
    !Number.isFinite(envelope.displayWest)
    || !Number.isFinite(envelope.displayEast)
    || !Number.isFinite(width)
    || width <= 0
    || width > 360
  ) return null;
  const copy = cloneJson(geometry);
  if (!shiftGeometryInPlace(copy, envelope)) return null;
  const positions = projectedCoordinates(copy);
  if (!positions) return null;
  return copy as Json;
}

export function shiftPolygonalGeojsonToDisplayDomain(
  geometry: Json,
  envelope: WrappedAoiEnvelope,
): Json | null {
  const copy = shiftPolygonalGeojsonToLongitudeReference(geometry, envelope);
  if (!copy) return null;
  const positions = projectedCoordinates(copy);
  if (
    !positions
    || positions.some(([longitude]) => longitude < envelope.displayWest || longitude > envelope.displayEast)
  ) return null;
  return copy;
}

/** Render-only BBox projection for a Preview whose exact footprint is not on the wire. */
export function shiftBboxToDisplayDomain(bbox: Bbox | null | undefined, envelope: WrappedAoiEnvelope): Bbox | null {
  if (
    !bbox
    || !Number.isFinite(bbox.west)
    || !Number.isFinite(bbox.east)
    || !Number.isFinite(bbox.south)
    || !Number.isFinite(bbox.north)
    || bbox.west >= bbox.east
    || bbox.south >= bbox.north
  ) return null;
  const width = bbox.east - bbox.west;
  // A bbox wider than a half-world is not a proof of a local footprint. In
  // particular, canonical [-179, 179] must never become a 358-degree overlay.
  if (width > 180) return null;
  const envelopeCenter = (envelope.displayWest + envelope.displayEast) / 2;
  const candidates = [-2, -1, 0, 1, 2].map((shift) => {
    const west = bbox.west + shift * 360;
    const east = bbox.east + shift * 360;
    const overlap = Math.max(0, Math.min(east, envelope.displayEast) - Math.max(west, envelope.displayWest));
    const distance = Math.abs((west + east) / 2 - envelopeCenter);
    return { west, east, overlap, distance, shift };
  });
  candidates.sort((left, right) =>
    right.overlap - left.overlap ||
    left.distance - right.distance ||
    Math.abs(left.shift) - Math.abs(right.shift),
  );
  const { west, east } = candidates[0]!;
  return {
    west,
    south: bbox.south,
    east,
    north: bbox.north,
    crs: "EPSG:4326",
  };
}
