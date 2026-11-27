import assert from "node:assert/strict";
import test from "node:test";
import { loadAccountSessionStatuses, parseAccountSessionStatuses } from "./accountSessions.ts";

test("account status parser accepts only fixed secret-free provider and state pairs", () => {
  const status = [
    { provider: "cdse", state: "login_required" },
    { provider: "usgs_m2m", state: "credentials_missing" },
    { provider: "earthdata", state: "service_unavailable" },
    { provider: "bhoonidhi", state: "logged_in" },
    { provider: "mpc", state: "not_required" },
    { provider: "opentopography", state: "logged_in" },
    { provider: "gacos", state: "credentials_missing" },
  ];
  assert.deepEqual(parseAccountSessionStatuses(status), status);
  assert.throws(() => parseAccountSessionStatuses(status.map((entry) => entry.provider === "usgs_m2m" ? { ...entry, provider: "usgs" } : entry)));
  assert.throws(() => parseAccountSessionStatuses([...status.slice(0, 6), { ...status[6], token: "fixture-secret" }]));
  assert.throws(() => parseAccountSessionStatuses([...status.slice(0, 6), { provider: "cdse", state: "logged_in" }]));
});

test("account bridge invokes only the fixed secret-free status command", async () => {
  const called: string[] = [];
  const statuses = [
    { provider: "cdse", state: "login_required" },
    { provider: "usgs_m2m", state: "credentials_missing" },
    { provider: "earthdata", state: "login_required" },
    { provider: "bhoonidhi", state: "service_unavailable" },
    { provider: "mpc", state: "not_required" },
    { provider: "opentopography", state: "credentials_missing" },
    { provider: "gacos", state: "not_required" },
  ];
  assert.deepEqual(await loadAccountSessionStatuses(async (command, payload) => {
    called.push(command);
    assert.equal(payload, undefined);
    return statuses;
  }), statuses);
  assert.deepEqual(called, ["application_account_session_statuses"]);
});
