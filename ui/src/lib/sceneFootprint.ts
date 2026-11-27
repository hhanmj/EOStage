import type { Bbox, Json } from "./bridge.ts";
import { parseCanonicalGeometry, parseRenderGeometry } from "./geojsonGeometry.ts";
import { matchRenderCopyParity } from "./renderCopyParity.ts";

export type SceneFootprintRenderModel = {
  /** The frozen GeoJSON object is the authoritative rendering input. */
  geojson: Json;
  /** Derived only from the validated authoritative geometry. */
  bbox: Bbox & { crs: "EPSG:4326" };
  /** Leaflet positions, retaining distinct Polygon and MultiPolygon parts. */
  polygons: [number, number][][][];
};

type Coordinate = [number, number];

type Position = {
  horizontal: Coordinate;
};

const MAX_CANONICAL_GEOMETRY_BYTES = 8 * 1024 * 1024;
const MAX_RENDER_GEOMETRY_BYTES = 17 * 1024 * 1024;
const MAX_PROJECTED_RENDER_LONGITUDE = 900;
const MAX_RENDER_POLYGON_WIDTH = 360;
const MAX_SELF_INTERSECTION_PAIRS = 250_000;

type SelfIntersectionBudget = {
  remaining: number;
  exhausted: boolean;
};

function position(value: unknown, allowDisplayLongitude = false): Position | null {
  if (
    !Array.isArray(value)
    || value.length < 2
    || value.some((ordinate) => typeof ordinate !== "number" || !Number.isFinite(ordinate))
    || (!allowDisplayLongitude && (value[0]! < -180 || value[0]! > 180))
    || (allowDisplayLongitude && (value[0]! < -MAX_PROJECTED_RENDER_LONGITUDE || value[0]! > MAX_PROJECTED_RENDER_LONGITUDE))
    || value[1]! < -90
    || value[1]! > 90
  ) return null;
  return { horizontal: [value[0]!, value[1]!] };
}

function ring(
  value: unknown,
  allowDisplayLongitude: boolean,
  budget: SelfIntersectionBudget,
): Coordinate[] | null {
  if (!Array.isArray(value) || value.length < 4) return null;
  const positions = value.map((item) => position(item, allowDisplayLongitude));
  if (positions.some((item) => item === null)) return null;
  const valid = positions as Position[];
  const first = valid[0]!;
  const last = valid[valid.length - 1]!;
  if (
    first.horizontal[0] !== last.horizontal[0]
    || first.horizontal[1] !== last.horizontal[1]
  ) return null;
  const coordinates = valid.map((item) => item.horizontal);
  if (new Set(coordinates.slice(0, -1).map(([longitude, latitude]) => `${longitude},${latitude}`)).size < 3) {
    return null;
  }
  const unwrapped: Coordinate[] = [coordinates[0]!];
  for (let index = 1; index < coordinates.length; index += 1) {
    const [previousLongitude] = unwrapped[index - 1]!;
    const [previousRawLongitude] = coordinates[index - 1]!;
    let longitude = coordinates[index]![0];
    if (!isBitExactFullWorldEdge(previousRawLongitude, longitude)) {
      while (longitude - previousLongitude > 180) longitude -= 360;
      while (longitude - previousLongitude < -180) longitude += 360;
    }
    unwrapped.push([longitude, coordinates[index]![1]]);
  }
  if (ringSelfIntersects(unwrapped, budget)) return null;
  const signedDoubleArea = unwrapped.slice(0, -1).reduce(
    (sum, [longitude, latitude], index) => {
      const [nextLongitude, nextLatitude] = unwrapped[index + 1]!;
      return sum + longitude * nextLatitude - nextLongitude * latitude;
    },
    0,
  );
  return signedDoubleArea === 0 ? null : coordinates;
}

function isBitExactFullWorldEdge(first: number, second: number): boolean {
  const delta = second - first;
  if (delta !== 360 && delta !== -360) return false;
  // Only exact world copies of the canonical -180/180 endpoints qualify.
  // This admits [-180,180], [180,540], and their reverse/adjacent copies,
  // while rejecting near-global pseudo edges such as [181,539].
  return Number.isInteger((first + 180) / 360);
}

function orientation(a: Coordinate, b: Coordinate, c: Coordinate): number {
  return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
}

function onSegment(a: Coordinate, b: Coordinate, point: Coordinate): boolean {
  return point[0] >= Math.min(a[0], b[0]) && point[0] <= Math.max(a[0], b[0]) &&
    point[1] >= Math.min(a[1], b[1]) && point[1] <= Math.max(a[1], b[1]);
}

function segmentsIntersect(a: Coordinate, b: Coordinate, c: Coordinate, d: Coordinate): boolean {
  const abC = orientation(a, b, c);
  const abD = orientation(a, b, d);
  const cdA = orientation(c, d, a);
  const cdB = orientation(c, d, b);
  if (abC === 0 && onSegment(a, b, c)) return true;
  if (abD === 0 && onSegment(a, b, d)) return true;
  if (cdA === 0 && onSegment(c, d, a)) return true;
  if (cdB === 0 && onSegment(c, d, b)) return true;
  return (abC > 0) !== (abD > 0) && (cdA > 0) !== (cdB > 0);
}

function ringSelfIntersects(coordinates: Coordinate[], budget: SelfIntersectionBudget): boolean {
  const edgeCount = coordinates.length - 1;
  for (let left = 0; left < edgeCount; left += 1) {
    const leftStart = coordinates[left]!;
    const leftEnd = coordinates[left + 1]!;
    for (let right = left + 1; right < edgeCount; right += 1) {
      if (right === left + 1 || left === 0 && right === edgeCount - 1) continue;
      if (budget.remaining === 0) {
        budget.exhausted = true;
        return true;
      }
      budget.remaining -= 1;
      if (segmentsIntersect(leftStart, leftEnd, coordinates[right]!, coordinates[right + 1]!)) return true;
    }
  }
  return false;
}

function polygon(
  value: unknown,
  allowDisplayLongitude: boolean,
  budget: SelfIntersectionBudget,
): Coordinate[][] | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  const rings = value.map((item) => ring(item, allowDisplayLongitude, budget));
  return rings.some((item) => item === null) ? null : rings as Coordinate[][];
}

function polygonCoordinates(
  value: unknown,
  allowDisplayLongitude: boolean,
  budget: SelfIntersectionBudget,
): Coordinate[][][] | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const geometry = value as Record<string, unknown>;
  if (geometry.type === "Polygon") {
    const single = polygon(geometry.coordinates, allowDisplayLongitude, budget);
    return single ? [single] : null;
  }
  if (geometry.type !== "MultiPolygon" || !Array.isArray(geometry.coordinates) || geometry.coordinates.length === 0) {
    return null;
  }
  const polygons = geometry.coordinates.map((item) => polygon(item, allowDisplayLongitude, budget));
  return polygons.some((item) => item === null) ? null : polygons as Coordinate[][][];
}

function polygonWithinRenderWidth(polygon: Coordinate[][]): boolean {
  let west = Infinity;
  let east = -Infinity;
  for (const ringCoordinates of polygon) {
    for (const [longitude] of ringCoordinates) {
      west = Math.min(west, longitude);
      east = Math.max(east, longitude);
    }
  }
  return Number.isFinite(west) && Number.isFinite(east) && west < east && east - west <= MAX_RENDER_POLYGON_WIDTH;
}

/**
 * Validates the only frozen scene geometry forms that map rendering accepts.
 * No bbox-only geometry is manufactured: absent or malformed authority is not
 * admissible to the scene footprint layer.
 */
export function sceneFootprintRenderModel(value: unknown, renderedGeometry: unknown = value): SceneFootprintRenderModel | null {
  const canonicalGeometry = parseCanonicalGeometry(value, MAX_CANONICAL_GEOMETRY_BYTES);
  const renderGeometry = renderedGeometry === value
    ? canonicalGeometry
    : parseRenderGeometry(renderedGeometry, MAX_RENDER_GEOMETRY_BYTES, MAX_PROJECTED_RENDER_LONGITUDE);
  if (!canonicalGeometry || !renderGeometry) return null;
  const selfIntersectionBudget: SelfIntersectionBudget = {
    remaining: MAX_SELF_INTERSECTION_PAIRS,
    exhausted: false,
  };
  const authoritativeCoordinates = polygonCoordinates(canonicalGeometry, false, selfIntersectionBudget);
  const coordinates = renderedGeometry === value
    ? authoritativeCoordinates
    : polygonCoordinates(renderGeometry, true, selfIntersectionBudget);
  if (
    !authoritativeCoordinates ||
    !coordinates ||
    selfIntersectionBudget.exhausted ||
    !value ||
    typeof value !== "object" ||
    Array.isArray(value)
  ) return null;
  if (renderedGeometry !== value && !coordinates.every(polygonWithinRenderWidth)) return null;
  const authoritativeFullWorld = geometryHasBitExactFullWorldEdge(authoritativeCoordinates);
  const renderedFullWorld = geometryHasBitExactFullWorldEdge(coordinates);
  if (renderedGeometry !== value && authoritativeFullWorld !== renderedFullWorld) return null;
  // Native scene pages may contain one or two adjacent periodic copies of a
  // seam component. Every emitted copy must still be an exact canonical
  // integer-world copy; only far canonical components may be omitted.
  if (renderedGeometry !== value && !matchRenderCopyParity(authoritativeCoordinates, coordinates, false)) return null;
  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;
  for (const polygon of coordinates) {
    for (const ringCoordinates of polygon) {
      for (const [longitude, latitude] of ringCoordinates) {
        west = Math.min(west, longitude);
        south = Math.min(south, latitude);
        east = Math.max(east, longitude);
        north = Math.max(north, latitude);
      }
    }
  }
  const bbox: Bbox & { crs: "EPSG:4326" } = {
    west,
    south,
    east,
    north,
    crs: "EPSG:4326",
  };
  if (!(bbox.west < bbox.east && bbox.south < bbox.north)) return null;
  return {
    geojson: value as Json,
    bbox,
    polygons: coordinates.map((item) => item.map((points) => points.map(([longitude, latitude]) => [latitude, longitude]))),
  };
}

function geometryHasBitExactFullWorldEdge(polygons: Coordinate[][][]): boolean {
  return polygons.some((polygon) => polygon.some((ringCoordinates) => ringCoordinates.some((position, index) => {
    const next = ringCoordinates[index + 1];
    return next !== undefined && isBitExactFullWorldEdge(position[0], next[0]);
  })));
}
