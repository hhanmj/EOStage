import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import type { PreviewRuntimeState } from "@/lib/catalogPreview";
import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import { MapPreviewStatus } from "./MapPreviewStatus";

test.afterEach(() => cleanup());

function preview(status: PreviewRuntimeState["status"]): PreviewRuntimeState {
  return { status, product_key: "scene-1", descriptor: null, url: null, message: "影像服务连接失败" };
}

test("map status keeps cancel, retry, and close wired without creating another image", () => {
  const calls: string[] = [];
  const props = { activeProductKey: "scene-1", onCancel: () => calls.push("cancel"),
    onRetry: () => calls.push("retry"), onClose: () => calls.push("close") };
  const view = render(<MapPreviewStatus {...props} preview={preview("loading")} />);
  assert.equal(view.getByLabelText("地图影像预览").getAttribute("data-preview-status"), "loading");
  fireEvent.click(view.getByRole("button", { name: "取消地图预览" }));
  view.rerender(<MapPreviewStatus {...props} preview={preview("failed")} />);
  assert.match(view.getByRole("alert").textContent ?? "", /连接失败/);
  fireEvent.click(view.getByRole("button", { name: "重试地图预览" }));
  view.rerender(<MapPreviewStatus {...props} preview={preview("available")} />);
  assert.ok(view.queryByRole("button", { name: "取消地图预览" }) === null);
  assert.ok(view.queryByRole("img") === null);
  fireEvent.click(view.getByRole("button", { name: "关闭地图预览" }));
  assert.deepEqual(calls, ["cancel", "retry", "close"]);
});

test("inactive and mismatched product status never leaks into the current map", () => {
  const noop = () => undefined;
  const props = { activeProductKey: "scene-1", onCancel: noop, onRetry: noop, onClose: noop };
  const view = render(<MapPreviewStatus {...props} preview={preview("not_requested")} />);
  for (const status of ["not_requested", "stale"] as const) {
    view.rerender(<MapPreviewStatus {...props} preview={preview(status)} />);
    assert.ok(view.queryByLabelText("地图影像预览") === null);
  }
  view.rerender(<MapPreviewStatus {...props} activeProductKey="scene-2" preview={preview("failed")} />);
  assert.ok(view.queryByRole("alert") === null);
  view.rerender(<MapPreviewStatus {...props} preview={preview("cancelled")} />);
  assert.match(view.getByRole("status").textContent ?? "", /已取消/);
  assert.ok(view.getByRole("button", { name: "重试地图预览" }));
});
