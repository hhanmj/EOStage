import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { cleanup, render, setupUser } from "../test-utils/render-harness";
import { DemSourceSelect } from "./DemSourceSelect";
import { resolvedDemProvider } from "../lib/demSources";

function DemSelectHarness() {
  const [dataset, setDataset] = useState("COP30");
  const provider = resolvedDemProvider(dataset);

  return (
    <div>
      <DemSourceSelect dataset={dataset} onChange={setDataset} />
      <output data-testid="selected-value">
        {provider}:{dataset}
      </output>
    </div>
  );
}

test("DemSourceSelect exposes only product choice while provider routing stays internal", async () => {
  const user = setupUser();
  const view = render(<DemSelectHarness />);

  const select = view.getByLabelText("DEM 产品与分辨率") as HTMLSelectElement;
  assert.equal(select.value, "COP30");
  assert.equal(view.getByTestId("selected-value").textContent, "COPERNICUS_AWS:COP30");

  await user.selectOptions(select, "SRTM_GL1");
  assert.equal(select.value, "SRTM_GL1");
  assert.equal(view.getByTestId("selected-value").textContent, "OPENTOPOGRAPHY:SRTM_GL1");

  cleanup();
});
