import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, render } from "@/test-utils/render-harness";
import { GacosPlanIssues } from "./GacosPlanIssues";

test.afterEach(() => cleanup());

test("native planning failures show a specific correction and the original code", () => {
  const view = render(<GacosPlanIssues report={{ has_errors: true, issues: [
    { code: "GACOS_TIME_REQUIRED", severity: "error" },
  ] }} />);
  assert.match(view.getByRole("alert").textContent ?? "", /UTC 小时和分钟/);
  assert.ok(view.getByText("GACOS_TIME_REQUIRED"));
});

test("an empty report does not invent success while a detail-less block stays visible", () => {
  const view = render(<GacosPlanIssues report={{ has_errors: false, issues: [] }} />);
  assert.equal(view.queryByRole("status"), null);
  view.rerender(<GacosPlanIssues report={{ has_errors: true, issues: [] }} />);
  assert.match(view.getByRole("alert").textContent ?? "", /没有返回可展示的具体错误项/);
});
