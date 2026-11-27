import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { TaskPageDetail } from "../src/components/TaskPageDetail.tsx";

test("rendered task detail keeps unknown totals and terminal partial state honest", async () => {
  const dom = new JSDOM("<div id=\"root\"></div>");
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  const previousNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperties(globalThis, {
    window: { configurable: true, value: dom.window },
    document: { configurable: true, value: dom.window.document },
    navigator: { configurable: true, value: dom.window.navigator },
    IS_REACT_ACT_ENVIRONMENT: { configurable: true, value: true },
  });
  let root: ReturnType<typeof createRoot> | null = null;
  try {
    root = createRoot(dom.window.document.getElementById("root")!);
    await act(async () => {
      root?.render(
        <TaskPageDetail
          page={{
            taskId: "task-1",
            revision: 2,
            pageIndex: 0,
            pageSize: 100,
            totalCount: 1,
            items: [{
              itemId: "scene-1",
              displayName: "S1_PARTIAL",
              status: "cancelled",
              bytesCompleted: 55,
              totalBytes: null,
              percentage: null,
              bytesPerSecond: 0,
              destination: "C:\\downloads\\scene.zip.part",
              failure: { code: "CANCELLED", reasonZh: "用户取消", retryable: false },
              retryable: true,
              terminal: true,
            }],
          }}
          onPageIndexChange={() => undefined}
        />,
      );
    });
    const text = dom.window.document.body.textContent ?? "";
    assert.match(text, /55 B/);
    assert.match(text, /总量未知/);
    assert.match(text, /速率 0 B\/s/);
    assert.match(text, /C:\\downloads\\scene.zip.part/);
    assert.match(text, /原因 用户取消/);
    assert.match(text, /代码 CANCELLED/);
    assert.match(text, /失败不可重试/);
    assert.match(text, /项目可重试/);
  } finally {
    if (root) await act(async () => root?.unmount());
    for (const [key, descriptor] of Object.entries({
      window: previousWindow,
      document: previousDocument,
      navigator: previousNavigator,
    })) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete (globalThis as Record<string, unknown>)[key];
    }
    dom.window.close();
  }
});
