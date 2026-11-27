export type MapProviderCrs = "EPSG:3857" | "GCJ-02" | string;

export type MapProviderProjectionBbox = {
  west: number;
  east: number;
  south: number;
  north: number;
  crs?: string;
};

const PI = Math.PI;
const GCJ_A = 6378245.0;
const GCJ_EE = 0.006693421622965943;

function outsideChina(longitude: number, latitude: number): boolean {
  return longitude < 72.004 || longitude > 137.8347 || latitude < 0.8293 || latitude > 55.8271;
}

function transformLatitude(longitude: number, latitude: number): number {
  let value = -100 + 2 * longitude + 3 * latitude + 0.2 * latitude * latitude;
  value += 0.1 * longitude * latitude + 0.2 * Math.sqrt(Math.abs(longitude));
  value += (20 * Math.sin(6 * longitude * PI) + 20 * Math.sin(2 * longitude * PI)) * 2 / 3;
  value += (20 * Math.sin(latitude * PI) + 40 * Math.sin(latitude / 3 * PI)) * 2 / 3;
  value += (160 * Math.sin(latitude / 12 * PI) + 320 * Math.sin(latitude * PI / 30)) * 2 / 3;
  return value;
}

function transformLongitude(longitude: number, latitude: number): number {
  let value = 300 + longitude + 2 * latitude + 0.1 * longitude * longitude;
  value += 0.1 * longitude * latitude + 0.1 * Math.sqrt(Math.abs(longitude));
  value += (20 * Math.sin(6 * longitude * PI) + 20 * Math.sin(2 * longitude * PI)) * 2 / 3;
  value += (20 * Math.sin(longitude * PI) + 40 * Math.sin(longitude / 3 * PI)) * 2 / 3;
  value += (150 * Math.sin(longitude / 12 * PI) + 300 * Math.sin(longitude / 30 * PI)) * 2 / 3;
  return value;
}

function wgs84ToGcj02(longitude: number, latitude: number): [number, number] {
  if (outsideChina(longitude, latitude)) return [longitude, latitude];
  let latitudeDelta = transformLatitude(longitude - 105, latitude - 35);
  let longitudeDelta = transformLongitude(longitude - 105, latitude - 35);
  const radianLatitude = latitude / 180 * PI;
  let magic = Math.sin(radianLatitude);
  magic = 1 - GCJ_EE * magic * magic;
  const sqrtMagic = Math.sqrt(magic);
  latitudeDelta = latitudeDelta * 180 / ((GCJ_A * (1 - GCJ_EE)) / (magic * sqrtMagic) * PI);
  longitudeDelta = longitudeDelta * 180 / (GCJ_A / sqrtMagic * Math.cos(radianLatitude) * PI);
  return [longitude + longitudeDelta, latitude + latitudeDelta];
}

function cloneJson<T>(value: T): T {
  if (Array.isArray(value)) return value.map((item) => cloneJson(item)) as T;
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, item]) => [key, cloneJson(item)]),
    ) as T;
  }
  return value;
}

function projectCoordinates(value: unknown): unknown {
  if (!Array.isArray(value)) return cloneJson(value);
  if (
    value.length >= 2 &&
    typeof value[0] === "number" && Number.isFinite(value[0]) &&
    typeof value[1] === "number" && Number.isFinite(value[1])
  ) {
    const [longitude, latitude] = wgs84ToGcj02(value[0], value[1]);
    return [longitude, latitude, ...value.slice(2).map((item) => cloneJson(item))];
  }
  return value.map((item) => projectCoordinates(item));
}

function projectGeojsonValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map((item) => projectGeojsonValue(item));
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([key, item]) => [
      key,
      key === "coordinates" ? projectCoordinates(item) : projectGeojsonValue(item),
    ]),
  );
}

export function projectGeometryForMapProvider<T>(geometry: T, providerCrs: MapProviderCrs): T {
  if (providerCrs !== "GCJ-02") return cloneJson(geometry);
  return projectGeojsonValue(geometry) as T;
}

export function projectBboxForMapProvider<T extends MapProviderProjectionBbox>(
  bbox: T,
  providerCrs: MapProviderCrs,
): T {
  if (providerCrs !== "GCJ-02") return cloneJson(bbox);
  const canonicalCorners: Array<[number, number]> = [
    [bbox.west, bbox.south],
    [bbox.west, bbox.north],
    [bbox.east, bbox.south],
    [bbox.east, bbox.north],
  ];
  const projectedCorners = canonicalCorners.map(([longitude, latitude]) =>
    wgs84ToGcj02(longitude, latitude));
  const moved = projectedCorners.some(([longitude, latitude], index) =>
    longitude !== canonicalCorners[index]![0] || latitude !== canonicalCorners[index]![1]);
  if (!moved) return cloneJson(bbox);
  return {
    ...cloneJson(bbox),
    west: Math.min(...projectedCorners.map(([longitude]) => longitude)),
    east: Math.max(...projectedCorners.map(([longitude]) => longitude)),
    south: Math.min(...projectedCorners.map(([, latitude]) => latitude)),
    north: Math.max(...projectedCorners.map(([, latitude]) => latitude)),
    crs: "GCJ-02",
  };
}
