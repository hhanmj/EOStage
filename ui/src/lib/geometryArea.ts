const EARTH_RADIUS_METRES = 6_371_008.8;
const SQUARE_METRES_PER_SQUARE_KILOMETRE = 1_000_000;

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function radians(value: number) {
  return value * Math.PI / 180;
}

function normalizedLongitudeDelta(from: number, to: number) {
  let delta = radians(to - from);
  while (delta > Math.PI) delta -= Math.PI * 2;
  while (delta < -Math.PI) delta += Math.PI * 2;
  return delta;
}

function ringAreaSquareMetres(value: unknown): number {
  if (!Array.isArray(value) || value.length < 3) return 0;
  let sum = 0;
  for (let index = 0; index < value.length; index += 1) {
    const current = value[index];
    const next = value[(index + 1) % value.length];
    if (!Array.isArray(current) || !Array.isArray(next)) continue;
    const lon1 = Number(current[0]);
    const lat1 = Number(current[1]);
    const lon2 = Number(next[0]);
    const lat2 = Number(next[1]);
    if (![lon1, lat1, lon2, lat2].every(Number.isFinite)) continue;
    sum += normalizedLongitudeDelta(lon1, lon2)
      * (2 + Math.sin(radians(lat1)) + Math.sin(radians(lat2)));
  }
  return Math.abs(sum * EARTH_RADIUS_METRES * EARTH_RADIUS_METRES / 2);
}

function polygonAreaSquareMetres(value: unknown): number {
  if (!Array.isArray(value) || value.length === 0) return 0;
  const outer = ringAreaSquareMetres(value[0]);
  const holes = value.slice(1).reduce((sum, ring) => sum + ringAreaSquareMetres(ring), 0);
  return Math.max(0, outer - holes);
}

function geometryAreaSquareMetres(value: unknown): number {
  const geometry = asRecord(value);
  if (!geometry) return 0;
  const type = geometry.type;
  if (type === "Feature") return geometryAreaSquareMetres(geometry.geometry);
  if (type === "FeatureCollection" && Array.isArray(geometry.features)) {
    return geometry.features.reduce((sum, feature) => sum + geometryAreaSquareMetres(feature), 0);
  }
  if (type === "GeometryCollection" && Array.isArray(geometry.geometries)) {
    return geometry.geometries.reduce((sum, item) => sum + geometryAreaSquareMetres(item), 0);
  }
  if (type === "Polygon") return polygonAreaSquareMetres(geometry.coordinates);
  if (type === "MultiPolygon" && Array.isArray(geometry.coordinates)) {
    return geometry.coordinates.reduce((sum, polygon) => sum + polygonAreaSquareMetres(polygon), 0);
  }
  return 0;
}

/** Stable geometry-derived area; never approximates an administrative polygon from its bbox. */
export function geometryAreaKm2(value: unknown): number {
  return geometryAreaSquareMetres(value) / SQUARE_METRES_PER_SQUARE_KILOMETRE;
}
