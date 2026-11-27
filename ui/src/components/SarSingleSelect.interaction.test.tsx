import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";

import { cleanup, render, setupUser } from "../test-utils/render-harness";
import { SarSingleSelect } from "./SarSingleSelect";

const OPTIONS = [
  { value: "R", label: "右视（R）" },
  { value: "L", label: "左视（L）" },
];

function Harness({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState<string | null>(null);
  return (
    <div>
      <SarSingleSelect
        ariaLabel="视向"
        value={value}
        options={OPTIONS}
        allLabel="全部"
        disabled={disabled}
        onChange={setValue}
      />
      <button type="button">外部</button>
      <output data-testid="selected-value">{value ?? "ALL"}</output>
    </div>
  );
}

test("SarSingleSelect opens, selects R/L/all, and closes outside", async () => {
  const user = setupUser();
  const view = render(<Harness />);
  const trigger = view.getByRole("button", { name: "视向：全部" });

  await user.click(trigger);
  assert.equal(trigger.getAttribute("aria-expanded"), "true");
  await user.click(view.getByRole("option", { name: "右视（R）" }));
  assert.equal(view.getByTestId("selected-value").textContent, "R");
  assert.equal(trigger.getAttribute("aria-expanded"), "false");

  await user.click(trigger);
  await user.click(view.getByRole("option", { name: "左视（L）" }));
  assert.equal(view.getByTestId("selected-value").textContent, "L");

  await user.click(trigger);
  await user.click(view.getByRole("option", { name: "全部" }));
  assert.equal(view.getByTestId("selected-value").textContent, "ALL");

  await user.click(trigger);
  await user.click(view.getByRole("button", { name: "外部" }));
  assert.equal(trigger.getAttribute("aria-expanded"), "false");
  cleanup();
});

test("SarSingleSelect supports keyboard focus and selection", async () => {
  const user = setupUser();
  const view = render(<Harness />);
  const trigger = view.getByRole("button", { name: "视向：全部" });

  trigger.focus();
  await user.keyboard("{ArrowDown}");
  assert.equal(trigger.getAttribute("aria-expanded"), "true");
  await user.keyboard("{ArrowDown}{Enter}");
  assert.equal(view.getByTestId("selected-value").textContent, "R");

  await user.click(trigger);
  await user.keyboard("{Escape}");
  assert.equal(trigger.getAttribute("aria-expanded"), "false");
  assert.equal(document.activeElement, trigger);
  cleanup();
});

test("SarSingleSelect disabled state cannot open", async () => {
  const user = setupUser();
  const view = render(<Harness disabled />);
  const trigger = view.getByRole("button", { name: "视向：全部" });

  assert.equal((trigger as HTMLButtonElement).disabled, true);
  await user.click(trigger);
  assert.equal(view.queryByRole("listbox"), null);
  cleanup();
});
