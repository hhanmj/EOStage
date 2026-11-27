import assert from "node:assert/strict";
import test from "node:test";

import { constrainMapTooltipPosition } from "./mapTooltipPosition.ts";

test("map tooltip is repositioned inside every viewport corner", () => {
  const viewport = { width: 800, height: 500 };
  const tooltip = { width: 320, height: 220 };
  for (const anchor of [
    { x: 0, y: 0 },
    { x: 800, y: 0 },
    { x: 0, y: 500 },
    { x: 800, y: 500 },
  ]) {
    const position = constrainMapTooltipPosition(anchor, viewport, tooltip, 12);
    assert.ok(position.x >= 12);
    assert.ok(position.y >= 12);
    assert.ok(position.x + tooltip.width <= viewport.width - 12);
    assert.ok(position.y + tooltip.height <= viewport.height - 12);
  }
});

test("oversized tooltip is pinned to the inset origin without negative projection", () => {
  assert.deepEqual(
    constrainMapTooltipPosition(
      { x: 10, y: 10 },
      { width: 200, height: 120 },
      { width: 300, height: 200 },
      8,
    ),
    { x: 8, y: 8 },
  );
});
