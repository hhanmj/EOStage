import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { screen } from "@testing-library/react";
import { SpSelect, type SpSelectOption } from "./SpSelect";

test.afterEach(() => {
  cleanup();
});

const OPTIONS: SpSelectOption[] = [
  { value: "sar", label: "数据检索" },
  { value: "dem", label: "DEM 规划" },
  { value: "orbit", label: "精密轨道" },
  { value: "convert", label: "高程转换", disabled: true },
];

test("SpSelect renders with selected label, opens dropdown, and triggers onChange", () => {
  function SelectHarness() {
    const [val, setVal] = useState("sar");
    return (
      <SpSelect
        data-testid="mode-select"
        value={val}
        onChange={setVal}
        options={OPTIONS}
        aria-label="模式选择"
      />
    );
  }

  render(<SelectHarness />);

  const trigger = screen.getByTestId("mode-select");
  assert.strictEqual(trigger.textContent?.includes("数据检索"), true);
  assert.strictEqual(trigger.getAttribute("aria-expanded"), "false");

  // 点击展开
  fireEvent.click(trigger);
  assert.strictEqual(trigger.getAttribute("aria-expanded"), "true");
  const listbox = screen.getByRole("listbox");
  assert.ok(listbox);

  // 点击选择 DEM 规划
  const demOption = screen.getByText("DEM 规划");
  fireEvent.click(demOption);

  // 展开关闭，值更新为 DEM 规划
  assert.strictEqual(trigger.textContent?.includes("DEM 规划"), true);
  assert.strictEqual(trigger.getAttribute("aria-expanded"), "false");
});

test("SpSelect supports keyboard navigation (ArrowDown, Enter, Escape)", () => {
  let selected = "sar";
  const { rerender } = render(
    <SpSelect
      data-testid="keyboard-select"
      value={selected}
      onChange={(v) => {
        selected = v;
      }}
      options={OPTIONS}
      aria-label="模式选择"
    />
  );

  const trigger = screen.getByTestId("keyboard-select");

  // 按下回车打开
  fireEvent.keyDown(trigger, { key: "Enter" });
  assert.strictEqual(trigger.getAttribute("aria-expanded"), "true");

  // 下移一项并回车选中
  fireEvent.keyDown(trigger, { key: "ArrowDown" });
  fireEvent.keyDown(trigger, { key: "Enter" });

  assert.strictEqual(selected, "dem");
  rerender(
    <SpSelect
      data-testid="keyboard-select"
      value={selected}
      onChange={(v) => {
        selected = v;
      }}
      options={OPTIONS}
      aria-label="模式选择"
    />
  );
  assert.strictEqual(trigger.textContent?.includes("DEM 规划"), true);

  // 再次打开后按 Escape 关闭
  fireEvent.keyDown(trigger, { key: " " });
  assert.strictEqual(trigger.getAttribute("aria-expanded"), "true");
  fireEvent.keyDown(trigger, { key: "Escape" });
  assert.strictEqual(trigger.getAttribute("aria-expanded"), "false");
});

test("SpSelect widens popover with min-w-full w-max max-w-[360px] and exposes full title tooltip", () => {
  render(
    <SpSelect
      data-testid="wide-select"
      value="cog"
      onChange={() => {}}
      options={[
        { value: "cog", label: "COG 云优化 GeoTIFF (.tif)", description: "Cloud Optimized GeoTIFF" },
        { value: "mbtiles", label: "MBTiles 离线瓦片库 (.mbtiles)" },
      ]}
      aria-label="导出格式"
    />
  );

  fireEvent.click(screen.getByTestId("wide-select"));
  const listbox = screen.getByRole("listbox", { name: "导出格式" });
  assert.equal(listbox.className.includes("min-w-full"), true);
  assert.equal(listbox.className.includes("w-max"), true);
  assert.equal(listbox.className.includes("max-w-[360px]"), true);
  const firstOpt = screen.getByRole("option", { name: /COG 云优化/ });
  assert.ok(firstOpt.getAttribute("title")?.includes("COG 云优化 GeoTIFF (.tif)"));
});
