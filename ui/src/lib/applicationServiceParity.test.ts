import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

type ReadonlyServiceSnapshot = {
  readiness: {
    contract: string;
    status: string;
    store_schema_version: number | null;
  };
  restore: {
    workbench_batch: unknown;
    workbench: unknown;
    tasks: unknown[];
    runtimes: unknown[];
  };
};

test("canonical Application Service fixture has no transport envelope fields", () => {
  const fixturePath = new URL(
    "../../../tests/fixtures/v3_contracts/application_service_restore.json",
    import.meta.url,
  );
  const snapshot = JSON.parse(
    readFileSync(fixturePath, "utf8"),
  ) as ReadonlyServiceSnapshot;

  assert.deepEqual(Object.keys(snapshot).sort(), ["readiness", "restore"]);
  assert.equal(snapshot.readiness.contract, "application_service_readonly_v1");
  assert.equal(snapshot.readiness.status, "liveness_only");
  assert.equal(snapshot.restore.tasks.length, 0);
  assert.equal(snapshot.restore.runtimes.length, 0);

  const serialized = JSON.stringify(snapshot).toLowerCase();
  for (const forbidden of ["request_id", "protocol_major", "protocol_minor", "terminal"]) {
    assert.equal(serialized.includes(forbidden), false, forbidden);
  }
});
