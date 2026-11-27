import assert from "node:assert/strict";
import test from "node:test";
import { gacosPlanIssues } from "./gacosPlanIssues.ts";

test("native GACOS issues retain their codes and explain the corrective input", () => {
  const issues = gacosPlanIssues({ has_errors: true, issues: [
    { code: "GACOS_BBOX_TOO_LARGE", severity: "error" },
    { code: "GACOS_TIME_CONFLICT", severity: "error" },
    { code: "GACOS_TIME_REQUIRED", severity: "error" },
  ] });
  assert.equal(issues.length, 3);
  assert.match(issues[0].action, /AOI/);
  assert.match(issues[1].action, /UTC/);
  assert.match(issues[2].action, /小时和分钟/);
  assert.equal(issues[2].code, "GACOS_TIME_REQUIRED");
});

test("blocked reports without details remain explicitly blocked", () => {
  const issues = gacosPlanIssues({ has_errors: true, issues: [] });
  assert.equal(issues[0].code, "GACOS_PLAN_BLOCKED");
  assert.match(issues[0].action, /暂不能提交/);
});

test("a warning-only report does not gain an invented blocking error", () => {
  const issues = gacosPlanIssues({ has_errors: false, issues: [{ code: "GACOS_NOTICE", severity: "warning", message: "检查时间" }] });
  assert.equal(issues.length, 1);
  assert.equal(issues[0].severity, "warning");
});

test("blocking flag is not lost when all reported issues are only warnings", () => {
  const issues = gacosPlanIssues({ has_errors: true, issues: [{ code: "GACOS_NOTICE", severity: "warning" }] });
  assert.equal(issues[0].code, "GACOS_PLAN_BLOCKED");
});

test("malformed report values do not crash presentation", () => {
  for (const value of [null, false, [], "report"]) assert.deepEqual(gacosPlanIssues(value), []);
  assert.equal(gacosPlanIssues({ has_errors: true, issues: [null, false, "bad"] })[0].severity, "error");
});

test("unknown safe messages remain visible without dumping raw diagnostic objects", () => {
  const source = { has_errors: true, issues: [{ code: "GACOS_NEW_ISSUE", message: "当前请求范围无效", details: { privateField: "not-for-display" } }] };
  const before = JSON.stringify(source);
  const issues = gacosPlanIssues(source);
  assert.equal(issues[0].message, "当前请求范围无效");
  assert.doesNotMatch(JSON.stringify(issues), /not-for-display/);
  assert.equal(JSON.stringify(source), before);
});

test("untrusted locator text is not echoed in an unknown issue", () => {
  const [issue] = gacosPlanIssues({ has_errors: true, issues: [{ code: "GACOS_UNKNOWN", message: "https://example.invalid/private" }] });
  assert.doesNotMatch(issue.message, /example\.invalid/);
  assert.equal(issue.code, "GACOS_UNKNOWN");
});

test("identical issues are deduplicated and long safe text is bounded", () => {
  const item = { code: "GACOS_OTHER", message: "x".repeat(600) };
  const issues = gacosPlanIssues({ has_errors: true, issues: [item, item] });
  assert.equal(issues.length, 1);
  assert.equal(issues[0].message.length, 501);
});
