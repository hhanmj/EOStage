import assert from "node:assert/strict";
import test from "node:test";
import { formatByteSize } from "./formatByteSize.ts";

test("sizes use decimal units consistently from bytes through multi-GB NISAR products", () => {
  for (const [value, label] of [
    [0, "0 B"], [999, "999 B"], [1000, "1 KB"],
    [999999, "1000 KB"], [1000000, "1 MB"], [605000000, "605 MB"],
    [999999999, "1000 MB"], [1000000000, "1.0 GB"],
    [15000000000, "15.0 GB"], [29070721024, "29.1 GB"],
  ] as const) assert.equal(formatByteSize(value), label);
});

test("unknown sizes stay distinct from a measured zero", () => {
  for (const value of [null, undefined, NaN, Infinity, -1]) {
    assert.equal(formatByteSize(value), "—");
    assert.equal(formatByteSize(value, "总量未知"), "总量未知");
  }
});
