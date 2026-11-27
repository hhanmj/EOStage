import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";

import { cleanup, render, setupUser } from "../test-utils/render-harness";
import { FilterDateRange } from "./FilterDateRange";

function Harness({ initialStart = "", initialEnd = "" }) {
  const [start, setStart] = useState(initialStart);
  const [end, setEnd] = useState(initialEnd);
  return (
    <div>
      <FilterDateRange
        start={start}
        end={end}
        onChange={(next) => {
          setStart(next.start);
          setEnd(next.end);
        }}
        now={new Date("2026-08-25T12:00:00Z")}
      />
      <button type="button">外部</button>
      <output data-testid="range">{start}|{end}</output>
    </div>
  );
}

test("FilterDateRange applies quick ranges and clears to unbounded", async () => {
  const user = setupUser();
  const view = render(<Harness />);

  await user.click(view.getByRole("button", { name: "日期范围：全部时间" }));
  await user.click(view.getByRole("button", { name: "最近 1 个月" }));
  assert.equal(view.getByTestId("range").textContent, "2026-07-25|2026-08-25");

  await user.click(view.getByRole("button", { name: /日期范围：2026\/07\/25/ }));
  await user.click(view.getByRole("button", { name: "全部时间" }));
  assert.equal(view.getByTestId("range").textContent, "|");
  cleanup();
});

test("FilterDateRange selects a project-owned calendar day and closes outside", async () => {
  const user = setupUser();
  const view = render(<Harness initialStart="2026-08-10" />);
  const trigger = view.getByRole("button", { name: /日期范围：2026\/08\/10/ });

  await user.click(trigger);
  await user.click(view.getByRole("button", { name: "选择 2026-08-15" }));
  assert.equal(view.getByTestId("range").textContent, "2026-08-10|2026-08-15");
  await user.click(view.getByRole("button", { name: "外部" }));
  assert.equal(trigger.getAttribute("aria-expanded"), "false");
  cleanup();
});

test("FilterDateRange exposes editable date inputs and explicit clear actions", async () => {
  const user = setupUser();
  const view = render(<Harness initialStart="2026-08-10" initialEnd="2026-08-20" />);

  await user.click(view.getByRole("button", { name: /日期范围：2026\/08\/10/ }));
  const startInput = view.getByRole("textbox", { name: "开始日期" });
  const endInput = view.getByRole("textbox", { name: "结束日期" });
  assert.equal(startInput.getAttribute("placeholder"), "yyyy-mm-dd");
  assert.equal(endInput.getAttribute("placeholder"), "yyyy-mm-dd");

  await user.click(view.getByRole("button", { name: "清空开始日期" }));
  assert.equal(view.getByTestId("range").textContent, "|2026-08-20");
  await user.click(view.getByRole("button", { name: "清空日期" }));
  assert.equal(view.getByTestId("range").textContent, "|");
  cleanup();
});
