import assert from "node:assert/strict";
import test from "node:test";

import { canonicalDemIdentityTimestamp } from "./demIdentityTimestamp.ts";

test("DEM identity timestamps share one canonical UTC representation", () => {
  for (const [input, expected] of [
    ["2026-08-17T09:10:11.000Z", "2026-08-17T09:10:11Z"],
    ["2026-08-17T09:10:11.010Z", "2026-08-17T09:10:11.01Z"],
    ["2026-08-17T09:10:11.120Z", "2026-08-17T09:10:11.12Z"],
    ["2026-08-17T09:10:11.123Z", "2026-08-17T09:10:11.123Z"],
  ] as const) {
    assert.equal(canonicalDemIdentityTimestamp(input), expected);
    assert.equal(canonicalDemIdentityTimestamp(expected), expected);
  }
  for (const invalid of [
    "2026-08-17T09:10:11.1234Z",
    "2026-08-17T09:10:11+00:00",
    "2026-08-17T09:10:11Z\n",
    "2026-02-30T09:10:11Z",
  ]) {
    assert.equal(canonicalDemIdentityTimestamp(invalid), null);
  }
});
