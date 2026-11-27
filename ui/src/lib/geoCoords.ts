/**
 * geoCoords.ts
 * 经纬度坐标轴格式化工具：
 * 严格按经纬度符号区分东西半球 (E / W) 与南北半球 (N / S)，杜绝东经/北纬误标为 W / S。
 */

export function formatSignedLon(value: number, digits = 4): string {
  if (!Number.isFinite(value)) return "—";
  const hemi = value >= 0 ? "E" : "W";
  return `${Math.abs(value).toFixed(digits)}°${hemi}`;
}

export function formatSignedLat(value: number, digits = 4): string {
  if (!Number.isFinite(value)) return "—";
  const hemi = value >= 0 ? "N" : "S";
  return `${Math.abs(value).toFixed(digits)}°${hemi}`;
}

export function formatSignedBboxSummary(
  bbox: { west: number; south: number; east: number; north: number },
  digits = 4,
  separator = " · ",
): string {
  return [
    formatSignedLon(bbox.west, digits),
    formatSignedLat(bbox.south, digits),
    formatSignedLon(bbox.east, digits),
    formatSignedLat(bbox.north, digits),
  ].join(separator);
}
