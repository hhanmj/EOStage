import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import test from "node:test";

import type { SarSourceCapability } from "./generated/v3-contracts.ts";
import {
  presentSarSources,
  sarSourceDisplayLabel,
} from "./sarSourcePresentation.ts";

const fixture = JSON.parse(
  readFileSync(new URL("../../../contracts/v3/asf-capabilities.v1.json", import.meta.url), "utf8"),
) as { sources: SarSourceCapability[] };

test("SAR source presentation keeps contract identities while applying the approved order", () => {
  const presented = presentSarSources(fixture.sources);

  assert.deepEqual(
    presented.slice(0, 4).map((item) => item.selector),
    ["NISAR", "SENTINEL-1", "ALOS-2", "ALOS PALSAR"],
  );
  // The ASF-removed legacy missions are retired and no longer appear here.
  assert.ok(!presented.some((item) => item.selector === "JERS-1"));
  assert.ok(!presented.some((item) => item.selector === "ERS"));
  assert.ok(!presented.some((item) => item.selector === "RADARSAT-1"));
  assert.deepEqual(
    [...presented.map((item) => item.selector)].sort(),
    [...fixture.sources.map((item) => item.selector)].sort(),
  );
});

test("SAR selector uses concise product-owned labels without mutating backend display names", () => {
  const labels = new Map(
    fixture.sources.map((item) => [item.selector, sarSourceDisplayLabel(item)]),
  );

  assert.equal(labels.get("NISAR"), "NISAR");
  assert.equal(labels.get("SENTINEL-1"), "Sentinel-1");
  assert.equal(labels.get("ALOS-2"), "ALOS-2");
  assert.equal(labels.get("ALOS PALSAR"), "ALOS PALSAR");
  assert.equal(labels.get("SEASAT"), "SEASAT");
  assert.equal(
    fixture.sources.find((item) => item.selector === "ALOS-2")?.display_name,
    "ALOS-2 PALSAR-2 ScanSAR L1.1",
  );
});

test("unknown future sources retain stable contract order and display name", () => {
  const unknownA = { ...fixture.sources[0], selector: "FUTURE-A", display_name: "Future A" };
  const unknownB = { ...fixture.sources[0], selector: "FUTURE-B", display_name: "Future B" };
  const presented = presentSarSources([unknownA, unknownB]);

  assert.deepEqual(presented.map((item) => item.selector), ["FUTURE-A", "FUTURE-B"]);
  assert.equal(sarSourceDisplayLabel(unknownB), "Future B");
});
