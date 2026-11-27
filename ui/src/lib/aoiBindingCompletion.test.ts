import assert from "node:assert/strict";
import test from "node:test";

import { completeAoiBindingDecision } from "./aoiBindingCompletion.ts";

test("a newer search response is preserved when it wins before AOI bind success", () => {
  assert.deepEqual(
    completeAoiBindingDecision({
      succeeded: true,
      capturedAdminSearchGeneration: 4,
      currentAdminSearchGeneration: 5,
    }),
    { closeAoiTools: false, clearAdminSearch: false },
  );
});

test("an in-flight newer search is preserved when AOI bind succeeds first", () => {
  assert.deepEqual(
    completeAoiBindingDecision({
      succeeded: true,
      capturedAdminSearchGeneration: 8,
      currentAdminSearchGeneration: 9,
    }),
    { closeAoiTools: false, clearAdminSearch: false },
  );
});

test("without a newer search, successful AOI binding clears the old candidates", () => {
  assert.deepEqual(
    completeAoiBindingDecision({
      succeeded: true,
      capturedAdminSearchGeneration: 12,
      currentAdminSearchGeneration: 12,
    }),
    { closeAoiTools: false, clearAdminSearch: true },
  );
});

test("a failed AOI bind neither closes tools nor clears search state", () => {
  assert.deepEqual(
    completeAoiBindingDecision({
      succeeded: false,
      capturedAdminSearchGeneration: 2,
      currentAdminSearchGeneration: 2,
    }),
    { closeAoiTools: false, clearAdminSearch: false },
  );
});
