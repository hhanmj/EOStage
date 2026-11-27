type JsonObject = Record<string, unknown>;

function objectValue(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

/**
 * Return the direct polygonal geometry used by the working-AOI and catalogue
 * contracts. Administrative providers may wrap that geometry in one Feature,
 * but collection wrappers are deliberately not merged implicitly.
 */
export function canonicalAdministrativeAoiGeometry(value: unknown): JsonObject | null {
  const record = objectValue(value);
  if (!record) return null;
  const type = typeof record.type === "string" ? record.type : "";
  if (type === "Polygon" || type === "MultiPolygon") return record;
  if (type !== "Feature") return null;

  const geometry = objectValue(record.geometry);
  if (!geometry) return null;
  const geometryType = typeof geometry.type === "string" ? geometry.type : "";
  return geometryType === "Polygon" || geometryType === "MultiPolygon"
    ? geometry
    : null;
}
