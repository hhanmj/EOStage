import assert from "node:assert/strict";
import test from "node:test";

import React from "react";

import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import { NisarBandSelector } from "./NisarBandSelector";

test("integrating S-band stays visible but cannot be selected by pointer", () => {
  const changes: string[] = [];
  const view = render(
    <NisarBandSelector
      value="L"
      sBandState="integrating"
      onChange={(value) => changes.push(value)}
    />,
  );

  assert.ok(view.getByRole("radiogroup", { name: "NISAR 雷达波段" }));
  assert.equal(view.getByRole("radio", { name: /L-band · ASF\/NASA/ }).getAttribute("aria-checked"), "true");
  fireEvent.click(view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }));

  assert.deepEqual(changes, []);
  assert.equal(
    (view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }) as HTMLButtonElement).disabled,
    true,
  );
  assert.ok(view.getByText("正在接入"));
  assert.match(view.getByText(/已保留范围和日期/).textContent ?? "", /已清除不兼容条件/);
  cleanup();
});

test("admitted S-band selects the Bhoonidhi route while retaining the shared selector", () => {
  const changes: Array<"L" | "S"> = [];
  const view = render(
    <NisarBandSelector
      value="L"
      sBandState="available"
      onChange={(value) => changes.push(value)}
    />,
  );

  const sBand = view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }) as HTMLButtonElement;
  assert.equal(sBand.disabled, false);
  fireEvent.click(sBand);
  assert.deepEqual(changes, ["S"]);
  cleanup();
});

test("selected S-band remains visibly disabled when restoring a pre-admission value", () => {
  const view = render(
    <NisarBandSelector value="S" sBandState="integrating" onChange={() => undefined} />,
  );

  assert.equal(
    view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }).getAttribute("aria-checked"),
    "true",
  );
  assert.equal(
    (view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }) as HTMLButtonElement).disabled,
    true,
  );
  assert.ok(view.getByText(/S-band 正在接入/));
  cleanup();
});

test("band selector moves focus with arrow keys and keeps a roving tab stop", () => {
  let value: "L" | "S" = "L";
  const view = render(<NisarBandSelector value={value} sBandState="available" onChange={(next) => { value = next; }} />);
  const lBand = view.getByRole("radio", { name: /L-band · ASF\/NASA/ });
  lBand.focus();
  assert.equal(lBand.getAttribute("tabindex"), "0");
  assert.equal(view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }).getAttribute("tabindex"), "-1");

  fireEvent.keyDown(lBand, { key: "ArrowRight" });
  view.rerender(<NisarBandSelector value={value} sBandState="available" onChange={(next) => { value = next; }} />);
  const sBand = view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ });
  assert.equal(document.activeElement, sBand);
  assert.equal(sBand.getAttribute("tabindex"), "0");

  fireEvent.keyDown(sBand, { key: "Home" });
  view.rerender(<NisarBandSelector value={value} sBandState="available" onChange={(next) => { value = next; }} />);
  assert.equal(document.activeElement, view.getByRole("radio", { name: /L-band · ASF\/NASA/ }));
  const rerenderedLBand = view.getByRole("radio", { name: /L-band · ASF\/NASA/ });
  fireEvent.keyDown(rerenderedLBand, { key: "End" });
  view.rerender(<NisarBandSelector value={value} sBandState="available" onChange={(next) => { value = next; }} />);
  assert.equal(document.activeElement, view.getByRole("radio", { name: /S-band · Bhoonidhi\/NRSC/ }));
  cleanup();
});

test("disabled band selector ignores keyboard changes", () => {
  let changes = 0;
  const view = render(
    <NisarBandSelector value="L" sBandState="available" disabled onChange={() => { changes += 1; }} />,
  );
  fireEvent.keyDown(view.getByRole("radio", { name: /L-band · ASF\/NASA/ }), { key: "ArrowRight" });
  assert.equal(changes, 0);
  cleanup();
});

test("integrating S-band cannot be selected with arrow or End keys", () => {
  let changes = 0;
  const view = render(
    <NisarBandSelector value="L" sBandState="integrating" onChange={() => { changes += 1; }} />,
  );
  const lBand = view.getByRole("radio", { name: /L-band · ASF\/NASA/ });
  lBand.focus();
  fireEvent.keyDown(lBand, { key: "ArrowRight" });
  fireEvent.keyDown(lBand, { key: "End" });
  assert.equal(changes, 0);
  assert.equal(document.activeElement, lBand);
  cleanup();
});
