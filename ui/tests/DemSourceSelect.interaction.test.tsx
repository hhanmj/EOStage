import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

import { DemSourceSelect } from "../src/components/DemSourceSelect.tsx";
import { resolvedDemProvider } from "../src/lib/demSources.ts";

test("rendered Download DEM source keeps COP90 unique while resolving its provider internally", async () => {
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const observations: Array<[string, string]> = [];
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => {
    root.render(
      <DemSourceSelect
        dataset="COP30"
        onChange={(dataset) => observations.push([dataset, resolvedDemProvider(dataset) ?? ""])}
      />,
    );
  });
  const select = dom.window.document.querySelector("select") as HTMLSelectElement;
  const optionValues = Array.from(select.options).map((option) => option.value);
  assert.equal(optionValues.filter((value) => value === "COP90").length, 1);
  assert.equal(optionValues.some((value) => value.includes(":")), false);
  await act(async () => {
    select.value = "COP90";
    select.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
  });
  assert.deepEqual(observations, [["COP90", "COPERNICUS_AWS"]]);
  await act(async () => root.unmount());
});
