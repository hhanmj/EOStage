import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";

import { cleanup, fireEvent, render, setupUser } from "../test-utils/render-harness";
import { SarChoiceTabs } from "./SarChoiceTabs";

const CHOICES = [
  { value: "RSLC", label: "RSLC" },
  { value: "GSLC", label: "GSLC" },
  { value: "GCOV", label: "GCOV" },
];

function MultiChoiceHarness() {
  const [value, setValue] = useState("RSLC");
  return (
    <>
      <SarChoiceTabs label="产品" value={value} choices={CHOICES} onChange={setValue} />
      <output data-testid="selected-value">{value}</output>
    </>
  );
}

test("SarChoiceTabs renders a sole legal choice as selected inert product truth instead of unavailable grey", () => {
  let changes = 0;
  const view = render(
    <SarChoiceTabs
      label="处理级别"
      value="L1.1"
      choices={[{ value: "L1.1", label: "L1.1" }]}
      onChange={() => { changes += 1; }}
    />,
  );
  const tab = view.getByRole("tab", { name: "L1.1" });

  assert.equal(tab.getAttribute("aria-selected"), "true");
  assert.equal(tab.getAttribute("aria-disabled"), null);
  assert.notEqual(tab.tagName, "BUTTON");
  assert.equal(tab.getAttribute("tabindex"), null);
  assert.match(tab.className, /bg-primary/);
  assert.doesNotMatch(tab.className, /disabled:|text-muted-foreground/);
  fireEvent.click(tab);
  assert.equal(changes, 0);
  assert.equal((tab as HTMLElement).tabIndex, -1);
  cleanup();
});

test("SarChoiceTabs switches multiple legal values with pointer and keyboard ARIA tabs", async () => {
  const user = setupUser();
  const view = render(<MultiChoiceHarness />);
  const tablist = view.getByRole("tablist", { name: "产品" });
  const rslc = view.getByRole("tab", { name: "RSLC" });

  assert.equal(tablist.getAttribute("aria-orientation"), "horizontal");
  assert.equal(rslc.getAttribute("aria-selected"), "true");
  await user.click(view.getByRole("tab", { name: "GSLC" }));
  assert.equal(view.getByTestId("selected-value").textContent, "GSLC");

  view.getByRole("tab", { name: "GSLC" }).focus();
  await user.keyboard("{ArrowRight}");
  assert.equal(view.getByTestId("selected-value").textContent, "GCOV");
  assert.equal(document.activeElement, view.getByRole("tab", { name: "GCOV" }));

  await user.keyboard("{Home}");
  assert.equal(view.getByTestId("selected-value").textContent, "RSLC");
  assert.equal(document.activeElement, rslc);
  cleanup();
});
