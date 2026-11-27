import assert from "node:assert/strict";
import test from "node:test";

import React from "react";

import type { PreviewRuntimeState } from "@/lib/catalogPreview";

import { cleanup, fireEvent, render, setupUser } from "../test-utils/render-harness";
import { testDom } from "../test-utils/setup-dom";
import { MapPreviewPanel } from "./MapPreviewPanel";

test.afterEach(() => cleanup());

test("image decode failure is visible and retries the same source only on request", async () => {
  let retried = 0;
  const view = render(<MapPreviewPanel {...commonProps({ onRetry: () => { retried += 1; } })} />);
  const original = view.getByRole("img");
  fireEvent.error(original);

  assert.equal(view.getByLabelText("当前产品预览").getAttribute("data-preview-status"), "failed");
  assert.ok(view.queryByRole("img") === null, "a failed image must be removed");
  assert.ok(view.queryByText("可用") === null, "failed display must not claim availability");
  assert.ok(view.getByText(/预览图显示失败/));
  assert.equal(retried, 0);
  await setupUser().click(view.getByRole("button", { name: "重试预览" }));
  const replacement = view.getByRole("img");
  assert.ok(replacement !== original, "retry must create a new image element");
  assert.equal(replacement.getAttribute("src"), original.getAttribute("src"));
  assert.equal(retried, 1);
  fireEvent.error(original);
  assert.ok(view.getByRole("img"));
});

test("a new URL for the same product has a distinct image and ignores old events", () => {
  const view = render(<MapPreviewPanel {...commonProps()} />);
  const oldImage = view.getByRole("img");
  view.rerender(<MapPreviewPanel {...commonProps({
    preview: preview("available", { url: "https://asf.alaska.edu/preview/replacement.jpg" }),
  })} />);
  const nextImage = view.getByRole("img");
  assert.ok(nextImage !== oldImage, "a changed source must remount the image");
  fireEvent.error(oldImage);
  fireEvent.load(oldImage);
  assert.equal(view.getByLabelText("当前产品预览").getAttribute("data-preview-status"), "available");
  assert.ok(view.getByRole("img") === nextImage, "stale events must leave the current image intact");
  fireEvent.error(nextImage);
  assert.ok(view.queryByRole("img") === null, "the current failed image must be removed");
  view.rerender(<MapPreviewPanel {...commonProps({
    activeProductKey: "product-2", productLabel: "Product 2",
    preview: preview("available", { product_key: "product-2" }),
  })} />);
  assert.ok(view.getByRole("img", { name: "Product 2 预览" }));
});

test("unapproved preview URLs never reach an image or expose their raw locator", () => {
  const urls = [
    "https://unreviewed.invalid/private-preview.jpg",
    "file:///C:/private-preview.png",
    "data:text/html;base64,PHNjcmlwdD4=",
    "data:image/svg+xml;base64,PHN2Zz4=",
    "data:image/png;base64,not-valid-base64!",
  ];
  for (const url of urls) {
    const view = render(<MapPreviewPanel {...commonProps({ preview: preview("available", { url }) })} />);
    assert.ok(view.queryByRole("img") === null, `unexpected image for ${url.split(":")[0]}`);
    assert.equal(view.getByLabelText("当前产品预览").getAttribute("data-preview-status"), "failed");
    assert.ok(view.getByText(/来源未通过预览安全校验/));
    assert.equal(view.container.innerHTML.includes(url), false);
    cleanup();
  }
});

test("missing action implementations are disabled while wired actions remain independent", async () => {
  const view = render(<MapPreviewPanel {...commonProps({
    onSave: undefined, onOpenSaveDirectory: undefined,
    savedPath: "fixture-output/preview.jpg",
  })} />);
  assert.equal(view.queryByRole("button", { name: "打开原图" }), null);
  for (const name of ["保存", /保存到/]) {
    const button = view.getByRole("button", { name }) as HTMLButtonElement;
    assert.equal(button.disabled, true);
    assert.match(button.title, /当前入口未提供/);
  }
  let saved = 0;
  let folderOpened = 0;
  view.rerender(<MapPreviewPanel {...commonProps({
    savedPath: "fixture-output/preview.jpg",
    onSave: () => { saved += 1; },
    onOpenSaveDirectory: () => { folderOpened += 1; },
  })} />);
  const user = setupUser();
  await user.click(view.getByRole("button", { name: "保存" }));
  await user.click(view.getByRole("button", { name: /保存到/ }));
  assert.deepEqual([saved, folderOpened], [1, 1]);
});

function preview(
  status: PreviewRuntimeState["status"],
  overrides: Partial<PreviewRuntimeState> = {},
): PreviewRuntimeState {
  return {
    status,
    product_key: "product-1",
    descriptor: null,
    url: status === "available" ? "https://asf.alaska.edu/preview/product-1.jpg" : null,
    message: status === "failed" ? "预览加载失败。" : "",
    ...overrides,
  };
}

function commonProps(overrides: Partial<Parameters<typeof MapPreviewPanel>[0]> = {}) {
  return {
    preview: preview("available"),
    activeProductKey: "product-1",
    productLabel: "Product 1",
    onClose: () => undefined,
    onCancel: () => undefined,
    onSave: () => undefined,
    onOpenSaveDirectory: () => undefined,
    ...overrides,
  };
}

test("shared preview viewport zooms around the wheel cursor and pans the image", () => {
  const view = render(<MapPreviewPanel {...commonProps()} />);
  const viewport = view.getByLabelText("预览视口");
  const image = view.getByRole("img", { name: "Product 1 预览" });

  fireEvent.wheel(viewport, { deltaY: -100, clientX: 120, clientY: 80 });
  assert.equal(image.getAttribute("data-preview-zoom"), "1.1");
  assert.equal(image.getAttribute("data-preview-offset-x"), "-12");
  assert.equal(image.getAttribute("data-preview-offset-y"), "-8");

  fireEvent.pointerDown(viewport, {
    button: 0,
    pointerId: 7,
    clientX: 120,
    clientY: 80,
  });
  fireEvent.pointerMove(viewport, {
    pointerId: 7,
    clientX: 145,
    clientY: 68,
  });
  fireEvent.pointerUp(viewport, { pointerId: 7, clientX: 145, clientY: 68 });
  assert.equal(image.getAttribute("data-preview-offset-x"), "13");
  assert.equal(image.getAttribute("data-preview-offset-y"), "-20");
});

test("fit, 100 percent, and reset controls restore a predictable viewport", async () => {
  const view = render(<MapPreviewPanel {...commonProps()} />);
  const user = setupUser();
  const image = view.getByRole("img", { name: "Product 1 预览" });

  await user.click(view.getByRole("button", { name: "放大预览" }));
  assert.ok(view.getByText("125%"));
  await user.click(view.getByRole("button", { name: "适合窗口" }));
  assert.ok(view.getByText("适合"));
  assert.equal(image.getAttribute("data-preview-offset-x"), "0");

  await user.click(view.getByRole("button", { name: "原始尺寸" }));
  assert.ok(view.getByText("100%"));
  await user.click(view.getByRole("button", { name: "重置视图" }));
  assert.ok(view.getByText("适合"));
  assert.equal(image.getAttribute("data-preview-zoom"), "1");
  assert.equal(image.getAttribute("data-preview-offset-y"), "0");
});

test("loading can be cancelled and typed failures can be retried", async () => {
  let cancelled = 0;
  const loading = render(
    <MapPreviewPanel
      {...commonProps({
        preview: preview("loading", { message: "正在加载预览。" }),
        onCancel: () => { cancelled += 1; },
      })}
    />,
  );
  const user = setupUser();
  await user.click(loading.getByRole("button", { name: "取消" }));
  assert.equal(cancelled, 1);

  cleanup();
  let retried = 0;
  const failed = render(
    <MapPreviewPanel
      {...commonProps({
        preview: preview("failed"),
        onRetry: () => { retried += 1; },
      })}
    />,
  );
  await user.click(failed.getByRole("button", { name: "重试预览" }));
  assert.equal(retried, 1);

  cleanup();
  const cancelledPreview = render(
    <MapPreviewPanel
      {...commonProps({
        preview: preview("cancelled", { message: "预览加载已取消。" }),
        onRetry: () => { retried += 1; },
      })}
    />,
  );
  await user.click(cancelledPreview.getByRole("button", { name: "重试预览" }));
  assert.equal(retried, 2);
});

test("fitted image keeps its natural pixel under the first wheel anchor", () => {
  const view = render(<MapPreviewPanel {...commonProps()} />);
  const viewport = view.getByLabelText("预览视口");
  const image = view.getByRole("img", { name: "Product 1 预览" });
  Object.defineProperties(viewport, {
    clientWidth: { configurable: true, value: 400 },
    clientHeight: { configurable: true, value: 300 },
  });
  Object.defineProperties(image, {
    naturalWidth: { configurable: true, value: 4000 },
    naturalHeight: { configurable: true, value: 2000 },
  });
  fireEvent.load(image);
  fireEvent.wheel(viewport, { deltaY: -100, clientX: 120, clientY: 80 });

  const zoom = Number(image.getAttribute("data-preview-zoom"));
  const offsetX = Number(image.getAttribute("data-preview-offset-x"));
  const offsetY = Number(image.getAttribute("data-preview-offset-y"));
  assert.ok(Math.abs(zoom - 0.11) < 1e-12);
  const imagePoint = { x: 1200, y: 300 };
  assert.ok(Math.abs(offsetX + imagePoint.x * zoom - 120) < 1e-9);
  assert.ok(Math.abs(offsetY + imagePoint.y * zoom - 80) < 1e-9);
});

test("changing product identity resets the shared viewport transform", async () => {
  const view = render(<MapPreviewPanel {...commonProps()} />);
  const user = setupUser();
  const previousImage = view.getByRole("img", { name: "Product 1 预览" });
  await user.click(view.getByRole("button", { name: "原始尺寸" }));
  fireEvent.pointerDown(view.getByLabelText("预览视口"), {
    button: 0,
    pointerId: 1,
    clientX: 10,
    clientY: 10,
  });
  fireEvent.pointerMove(view.getByLabelText("预览视口"), {
    pointerId: 1,
    clientX: 30,
    clientY: 40,
  });
  fireEvent.pointerUp(view.getByLabelText("预览视口"), { pointerId: 1 });

  view.rerender(
    <MapPreviewPanel
      {...commonProps({
        activeProductKey: "product-2",
        preview: preview("available", { product_key: "product-2" }),
        productLabel: "Product 2",
      })}
    />,
  );
  assert.ok(view.getByText("适合"));
  const nextImage = view.getByRole("img", { name: "Product 2 预览" });
  assert.notEqual(nextImage, previousImage);
  assert.equal(nextImage.getAttribute("data-preview-zoom"), "1");
  assert.equal(nextImage.getAttribute("data-preview-offset-x"), "0");
});


test("cached image identity changes keep natural dimensions available for fit", () => {
  const imagePrototype = testDom.window.HTMLImageElement.prototype;
  const priorComplete = Object.getOwnPropertyDescriptor(imagePrototype, "complete");
  const priorNaturalWidth = Object.getOwnPropertyDescriptor(imagePrototype, "naturalWidth");
  const priorNaturalHeight = Object.getOwnPropertyDescriptor(imagePrototype, "naturalHeight");
  Object.defineProperty(imagePrototype, "complete", {
    configurable: true,
    get: () => true,
  });
  Object.defineProperty(imagePrototype, "naturalWidth", {
    configurable: true,
    get: () => 4000,
  });
  Object.defineProperty(imagePrototype, "naturalHeight", {
    configurable: true,
    get: () => 2000,
  });
  try {
    const view = render(<MapPreviewPanel {...commonProps()} />);
    assert.equal(view.getByRole("img", { name: "Product 1 预览" }).style.width, "4000px");
    view.rerender(
      <MapPreviewPanel
        {...commonProps({
          activeProductKey: "product-2",
          preview: preview("available", { product_key: "product-2" }),
          productLabel: "Product 2",
        })}
      />,
    );
    assert.equal(view.getByRole("img", { name: "Product 2 预览" }).style.width, "4000px");
  } finally {
    if (priorComplete) Object.defineProperty(imagePrototype, "complete", priorComplete);
    else delete (imagePrototype as { complete?: unknown }).complete;
    if (priorNaturalWidth) Object.defineProperty(imagePrototype, "naturalWidth", priorNaturalWidth);
    else delete (imagePrototype as { naturalWidth?: unknown }).naturalWidth;
    if (priorNaturalHeight) Object.defineProperty(imagePrototype, "naturalHeight", priorNaturalHeight);
    else delete (imagePrototype as { naturalHeight?: unknown }).naturalHeight;
  }
});
