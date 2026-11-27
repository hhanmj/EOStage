/**
 * Bounded GeoJSON geometry admission for authoritative canonical data and
 * native render-only DTOs.
 *
 * Canonical GeoJSON is allowed to carry RFC `bbox` and arbitrary foreign
 * members. Render DTOs are deliberately narrower and accept only `type` and
 * `coordinates`; callers must retain the original canonical object separately.
 */

export type GeoJsonGeometryObject = Record<string, unknown>;

const UTF8_ENCODER = new TextEncoder();
const INVALID_SNAPSHOT = Symbol("invalid-geojson-snapshot");

type JsonSnapshot = null | string | boolean | number | JsonSnapshot[] | { [key: string]: JsonSnapshot };
type SnapshotResult = JsonSnapshot | typeof INVALID_SNAPSHOT;

function plainObject(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  try {
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
  } catch {
    return false;
  }
}

function plainArray(value: unknown): value is unknown[] {
  if (!Array.isArray(value)) return false;
  try {
    const prototype = Object.getPrototypeOf(value);
    return prototype === Array.prototype || prototype === null;
  } catch {
    return false;
  }
}

function ownDescriptors(value: object): {
  keys: readonly (string | symbol)[];
  descriptors: PropertyDescriptorMap;
} | null {
  try {
    return {
      keys: Reflect.ownKeys(value),
      descriptors: Object.getOwnPropertyDescriptors(value),
    };
  } catch {
    return null;
  }
}

function descriptorFor(descriptors: PropertyDescriptorMap, key: string): PropertyDescriptor | null {
  return Object.prototype.hasOwnProperty.call(descriptors, key) ? descriptors[key]! : null;
}

function dataDescriptor(
  descriptor: PropertyDescriptor | null,
  enumerable: boolean,
): descriptor is PropertyDescriptor & { value: unknown } {
  return !!descriptor && descriptor.enumerable === enumerable && "value" in descriptor;
}

function prototypesAreUnpolluted(): boolean {
  return !Object.prototype.hasOwnProperty.call(Array.prototype, "toJSON") &&
    !Object.prototype.hasOwnProperty.call(Object.prototype, "toJSON");
}

function snapshotValue(value: unknown, active: WeakSet<object>): SnapshotResult {
  if (value === null) return null;
  if (typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") return Number.isFinite(value) ? value : INVALID_SNAPSHOT;
  if (typeof value !== "object" || active.has(value)) return INVALID_SNAPSHOT;
  const inspected = ownDescriptors(value);
  if (!inspected || inspected.keys.some((key) => typeof key !== "string")) return INVALID_SNAPSHOT;
  active.add(value);
  try {
    if (Array.isArray(value)) {
      if (!plainArray(value) || !prototypesAreUnpolluted()) return INVALID_SNAPSHOT;
      const lengthDescriptor = descriptorFor(inspected.descriptors, "length");
      if (
        !dataDescriptor(lengthDescriptor, false) ||
        lengthDescriptor.configurable ||
        !lengthDescriptor.writable ||
        typeof lengthDescriptor.value !== "number" ||
        !Number.isSafeInteger(lengthDescriptor.value) ||
        lengthDescriptor.value < 0 ||
        inspected.keys.length !== lengthDescriptor.value + 1 ||
        !inspected.keys.includes("length")
      ) return INVALID_SNAPSHOT;
      const length = lengthDescriptor.value;
      const elementDescriptors: PropertyDescriptor[] = [];
      for (let index = 0; index < length; index += 1) {
        const descriptor = descriptorFor(inspected.descriptors, String(index));
        if (!dataDescriptor(descriptor, true)) return INVALID_SNAPSHOT;
        elementDescriptors.push(descriptor);
      }
      if (inspected.keys.some((key) => key !== "length" &&
          (typeof key !== "string" || !/^\d+$/.test(key) || Number(key) >= length))) return INVALID_SNAPSHOT;
      const snapshot: JsonSnapshot[] = [];
      for (const descriptor of elementDescriptors) {
        const child = snapshotValue(descriptor.value, active);
        if (child === INVALID_SNAPSHOT) return INVALID_SNAPSHOT;
        snapshot.push(child);
      }
      return snapshot;
    }
    if (!plainObject(value) || !prototypesAreUnpolluted()) return INVALID_SNAPSHOT;
    for (const key of inspected.keys) {
      if (typeof key !== "string") return INVALID_SNAPSHOT;
      const descriptor = descriptorFor(inspected.descriptors, key);
      if (!dataDescriptor(descriptor, true)) return INVALID_SNAPSHOT;
    }
    const snapshot: { [key: string]: JsonSnapshot } = {};
    for (const key of inspected.keys as readonly string[]) {
      const descriptor = descriptorFor(inspected.descriptors, key)! as PropertyDescriptor & { value: unknown };
      const child = snapshotValue(descriptor.value, active);
      if (child === INVALID_SNAPSHOT) return INVALID_SNAPSHOT;
      Object.defineProperty(snapshot, key, {
        configurable: true,
        enumerable: true,
        value: child,
        writable: true,
      });
    }
    return snapshot;
  } finally {
    active.delete(value);
  }
}

function boundedPlainJsonObject(value: unknown, maximumBytes: number): GeoJsonGeometryObject | null {
  if (!plainObject(value)) return null;
  const snapshot = snapshotValue(value, new WeakSet<object>());
  if (snapshot === INVALID_SNAPSHOT || snapshot === null || typeof snapshot !== "object" || Array.isArray(snapshot)) return null;
  try {
    const serialized = JSON.stringify(snapshot);
    if (typeof serialized !== "string" || UTF8_ENCODER.encode(serialized).byteLength > maximumBytes) return null;
  } catch {
    return null;
  }
  return snapshot as GeoJsonGeometryObject;
}

function hasOwn(value: GeoJsonGeometryObject, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function parseRing(value: unknown, maximumLongitude: number): boolean {
  if (!Array.isArray(value) || value.length < 4) return false;
  const first = value[0];
  const last = value[value.length - 1];
  if (
    !Array.isArray(first) ||
    !Array.isArray(last) ||
    first.length < 2 ||
    last.length < 2 ||
    first[0] !== last[0] ||
    first[1] !== last[1]
  ) return false;
  const distinct = new Set<string>();
  for (let index = 0; index < value.length; index += 1) {
    const position = value[index];
    if (!Array.isArray(position) || position.length < 2) return false;
    if (position.some((ordinate) => typeof ordinate !== "number" || !Number.isFinite(ordinate))) return false;
    if (
      position[0]! < -maximumLongitude ||
      position[0]! > maximumLongitude ||
      position[1]! < -90 ||
      position[1]! > 90
    ) return false;
    if (index < value.length - 1) distinct.add(`${position[0]},${position[1]}`);
  }
  return distinct.size >= 3;
}

function parsePolygon(value: unknown, maximumLongitude: number): boolean {
  if (!Array.isArray(value) || value.length === 0) return false;
  return value.every((ring) => parseRing(ring, maximumLongitude));
}

function parseCoordinates(value: GeoJsonGeometryObject, maximumLongitude: number): boolean {
  if (value.type === "Polygon") return parsePolygon(value.coordinates, maximumLongitude);
  if (value.type === "MultiPolygon" && Array.isArray(value.coordinates) && value.coordinates.length > 0) {
    return value.coordinates.every((polygon) => parsePolygon(polygon, maximumLongitude));
  }
  return false;
}

function parseGeometry(
  value: unknown,
  maximumBytes: number,
  maximumLongitude: number,
  strictRenderKeys: boolean,
): GeoJsonGeometryObject | null {
  const source = boundedPlainJsonObject(value, maximumBytes);
  const keys = source ? Reflect.ownKeys(source) : [];
  if (
    !source ||
    !hasOwn(source, "type") ||
    !hasOwn(source, "coordinates") ||
    strictRenderKeys && (keys.length !== 2 || keys.some((key) => key !== "type" && key !== "coordinates")) ||
    !parseCoordinates(source, maximumLongitude)
  ) return null;
  return source;
}

export function parseCanonicalGeometry(value: unknown, maximumBytes: number): GeoJsonGeometryObject | null {
  return parseGeometry(value, maximumBytes, 180, false);
}

export function parseRenderGeometry(value: unknown, maximumBytes: number, maximumLongitude: number): GeoJsonGeometryObject | null {
  return parseGeometry(value, maximumBytes, maximumLongitude, true);
}
