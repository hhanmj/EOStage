/**
 * Shared native render-copy correspondence.
 *
 * The authoritative geometry may carry additional ordinates, but the map
 * contract is strictly two-dimensional. Canonical copies are generated once
 * for the bounded native world range and render polygons are resolved by an
 * exact key lookup; there is no tolerance or candidate scan in this path.
 */

export type RenderCopyPosition = readonly number[];
export type RenderCopyRing = readonly RenderCopyPosition[];
export type RenderCopyPolygon = readonly RenderCopyRing[];
export type RenderCopyPolygons = readonly RenderCopyPolygon[];

const EXPECTED_WORLD_SHIFTS = [-2, -1, 0, 1, 2] as const;

type ExpectedCopy = {
  canonicalIndex: number;
  worldShift: number;
} | null;

function structureSignature(polygon: RenderCopyPolygon): string {
  return `${polygon.length}|${polygon.map((ring) => ring.length).join(",")}`;
}

function exactNumber(value: number): string {
  return Object.is(value, -0) ? "-0" : value.toString();
}

function polygonKey(polygon: RenderCopyPolygon): string {
  return JSON.stringify([
    structureSignature(polygon),
    polygon.map((ring) => ring.map((position) => [
      exactNumber(position[0]!),
      exactNumber(position[1]!),
    ])),
  ]);
}

function expectedPolygonCopy(
  polygon: RenderCopyPolygon,
  worldShift: number,
): RenderCopyPolygon {
  return polygon.map((ring) => {
    let previousRawLongitude: number | null = null;
    let ringWorldShift = 0;
    return ring.map((position) => {
      const rawLongitude = position[0]!;
      if (previousRawLongitude !== null) {
        const rawDelta = rawLongitude - previousRawLongitude;
        const explicitFullWorldEdge =
          previousRawLongitude === -180 && rawLongitude === 180 ||
          previousRawLongitude === 180 && rawLongitude === -180;
        if (!explicitFullWorldEdge && rawDelta > 180) ringWorldShift -= 1;
        else if (!explicitFullWorldEdge && rawDelta < -180) ringWorldShift += 1;
      }
      previousRawLongitude = rawLongitude;
      // Core's producer computes source longitude plus one integer world index
      // in one f64 operation. Ring unwrapping contributes to that index for a
      // seam-crossing component; Z/M ordinates are deliberately not copied.
      return [rawLongitude + (worldShift + ringWorldShift) * 360, position[1]!];
    });
  });
}

function renderPolygonKey(polygon: RenderCopyPolygon): string {
  return polygonKey(polygon);
}

/**
 * Matches native render polygons against exact expected canonical copies.
 *
 * In strict mode every canonical polygon must be represented at least once.
 * Partial mode is used for scene render subsets: every emitted render polygon
 * must still match exactly, but far/unemitted canonical polygons may remain
 * unmatched. Each canonical/world pair is consumable only once and each
 * canonical polygon may contribute at most two copies.
 */
export function matchRenderCopyParity(
  canonicalPolygons: RenderCopyPolygons,
  renderPolygons: RenderCopyPolygons,
  requireEveryCanonical: boolean,
): boolean {
  if (
    canonicalPolygons.length === 0 ||
    renderPolygons.length === 0 ||
    renderPolygons.length > canonicalPolygons.length * 2 ||
    requireEveryCanonical && renderPolygons.length < canonicalPolygons.length
  ) return false;

  const expected = new Map<string, ExpectedCopy>();
  canonicalPolygons.forEach((polygon, canonicalIndex) => {
    for (const worldShift of EXPECTED_WORLD_SHIFTS) {
      const key = polygonKey(expectedPolygonCopy(polygon, worldShift));
      if (expected.has(key)) {
        expected.set(key, null);
      } else {
        expected.set(key, { canonicalIndex, worldShift });
      }
    }
  });

  const consumed = canonicalPolygons.map(() => new Set<number>());
  const matched = canonicalPolygons.map(() => 0);
  for (const renderPolygon of renderPolygons) {
    const assigned = expected.get(renderPolygonKey(renderPolygon));
    if (
      assigned === undefined ||
      assigned === null ||
      consumed[assigned.canonicalIndex]!.has(assigned.worldShift) ||
      consumed[assigned.canonicalIndex]!.size >= 2
    ) return false;
    consumed[assigned.canonicalIndex]!.add(assigned.worldShift);
    matched[assigned.canonicalIndex] = matched[assigned.canonicalIndex]! + 1;
  }
  return requireEveryCanonical ? matched.every((count) => count >= 1 && count <= 2) : true;
}
