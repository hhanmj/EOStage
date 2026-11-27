import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, render } from "@/test-utils/render-harness";
import { HoverHintLabel } from "./HoverHintLabel";

test.afterEach(() => cleanup());

test("settings hints use intrinsic width with bounded normal wrapping", () => {
  const view = render(<HoverHintLabel hint="密钥仅写入操作系统原生受保护凭据服务" focusable>信息</HoverHintLabel>);
  const tooltip = view.getByRole("tooltip");
  assert.ok(tooltip.classList.contains("w-max"));
  assert.ok(tooltip.classList.contains("max-w-[min(32rem,calc(100vw-2rem))]"));
  assert.ok(tooltip.classList.contains("whitespace-normal"));
  assert.equal(tooltip.classList.contains("whitespace-nowrap"), false);
});
