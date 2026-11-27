import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { JSDOM } from "jsdom";
import { clearTaskDetailTestBridge, registerTaskDetailTestBridge } from "@/lib/taskDetailTestBridge";
const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://127.0.0.1/" });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node, MutationObserver: dom.window.MutationObserver, getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true });
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });
const { FrozenProductPreview } = await import("./FrozenProductPreview");
const { cleanup, render, screen, act } = await import("@testing-library/react");
const userEvent = (await import("@testing-library/user-event")).default;
test.afterEach(() => { cleanup(); clearTaskDetailTestBridge(); });

test("frozen history viewer omits original-open and does not advertise a no-op save action", async () => {
  const commands: string[] = [];
  register(async (command) => { commands.push(command); return resolved("old-product"); });
  render(<FrozenProductPreview batchId="historical-batch" itemId="old-product" productLabel="Historical image" onClose={() => undefined} />);
  await screen.findByRole("img", { name: "Historical image 预览" });
  assert.equal(screen.queryByRole("button", { name: "打开原图" }), null);
  const button = screen.getByRole("button", { name: "保存" }) as HTMLButtonElement;
  assert.equal(button.disabled, true);
  assert.match(button.title, /当前入口未提供/);
  await userEvent.click(button);
  assert.deepEqual(commands, ["application_preview_asset"]);
});
function resolved(itemId: string) {
  return { product_key: itemId, descriptor: { preview_kind: "quicklook", source_asset_keys: [`${itemId}::browse`],
    render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1", media_type: "image/jpeg", credential_requirement: "none",
    cache_policy: "session", provenance: "ASF official Browse" }, source_asset_role: "browse", source_asset_key: `${itemId}::browse`,
    media_type: "image/jpeg", data_url: "data:image/jpeg;base64,AQID", byte_length: 3 };
}
function register(invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>) {
  registerTaskDetailTestBridge({ invoke, getCurrentWindow: () => ({ minimize: async () => undefined,
    toggleMaximize: async () => undefined, close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }),
    startResizeDragging: async () => undefined }), open: async () => null });
}
test("frozen history preview retries the exact source identity without a current workbench", async () => {
  const requests: Record<string, unknown>[] = [];
  register(async (command, args) => {
    assert.equal(command, "application_preview_asset");
    requests.push(args!.request as Record<string, unknown>);
    if (requests.length === 1) throw { code: "preview_timeout", message_key: "preview_timeout" };
    return resolved("old-product");
  });
  render(<FrozenProductPreview batchId="historical-batch" itemId="old-product" productLabel="Historical image" onClose={() => undefined} />);
  await userEvent.click(await screen.findByRole("button", { name: /重试/ }));
  await screen.findByRole("img", { name: "Historical image 预览" });
  assert.equal(requests.length, 2);
  assert.deepEqual(requests.map(({batch_id,item_id})=>({batch_id,item_id})), [
    {batch_id:"historical-batch",item_id:"old-product"}, {batch_id:"historical-batch",item_id:"old-product"}]);
  assert.notEqual(requests[0]!.request_id, requests[1]!.request_id);
});
test("provider no-preview remains unavailable instead of a frozen-batch mismatch", async () => {
  register(async () => { throw { code: "provider_no_preview", message_key: "provider_no_preview" }; });
  render(<FrozenProductPreview batchId="historical-batch" itemId="no-browse-product" productLabel="No browse" onClose={() => undefined} />);
  await screen.findByText("无预览图");
  assert.ok(screen.getByText("Provider 没有提供可验证的预览资产。"));
  assert.equal(screen.queryByRole("img"), null);
});
test("changing the frozen source cancels the previous request and ignores its late response", async () => {
  let finishOld!: (value: unknown) => void;
  const old = new Promise((resolve) => { finishOld = resolve; });
  const requests: Record<string, unknown>[] = [];
  const cancelled: string[] = [];
  register(async (command, args) => {
    if (command === "application_cancel_request") { cancelled.push(String(args!.requestId)); return { request_id: args!.requestId, cancelled: true }; }
    assert.equal(command, "application_preview_asset");
    const request = args!.request as Record<string, unknown>;
    requests.push(request);
    return request.batch_id === "old-batch" ? old : resolved("new-product");
  });
  const view = render(<FrozenProductPreview batchId="old-batch" itemId="old-product" productLabel="Old" onClose={() => undefined} />);
  await screen.findByText("加载中");
  view.rerender(<FrozenProductPreview batchId="new-batch" itemId="new-product" productLabel="New" onClose={() => undefined} />);
  await screen.findByRole("img", { name: "New 预览" });
  await act(async () => { finishOld(resolved("old-product")); await old; });
  assert.ok(screen.getByRole("img", { name: "New 预览" }));
  assert.equal(screen.queryByRole("img", { name: "Old 预览" }), null);
  assert.deepEqual(cancelled, [requests[0]!.request_id]);
});
test("compactOverlay renders monochrome styling across loading, failed retry, and available states without blue accent tokens", async () => {
  let attempt = 0;
  let resolveSecond!: (value: unknown) => void;
  const second = new Promise((resolve) => { resolveSecond = resolve; });
  register(async () => {
    attempt += 1;
    if (attempt === 1) throw { code: "preview_timeout", message_key: "preview_timeout" };
    return second;
  });
  const view = render(<FrozenProductPreview batchId="nisar-batch" itemId="nisar-item" productLabel="NISAR L2" compactOverlay onClose={() => undefined} />);
  const retryButton = await screen.findByRole("button", { name: /重试/ });
  const bar = view.container.querySelector("[data-subset-footprint-preview-bar]") as HTMLElement;
  assert.ok(bar);
  assert.equal(bar.innerHTML.includes("sp-accent"), false);
  await userEvent.click(retryButton);
  await screen.findByText("加载快视图中…");
  assert.equal(bar.innerHTML.includes("sp-accent"), false);
  await act(async () => { resolveSecond(resolved("nisar-item")); await second; });
  await screen.findByText("已叠加至影像框");
  assert.equal(bar.innerHTML.includes("sp-accent"), false);
});
