import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";

import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import { SchemaConditionFields, SchemaFacetFields, type SchemaMultiSelectRenderer } from "./SchemaFilterFields";
import {
  initialSearchFilterDraft,
  searchFilterSchema,
  type SearchFilterDraftValue,
} from "../lib/searchFilterSchema";

const S2 = searchFilterSchema("search.optical.sentinel-2.l2a.v1");
const LANDSAT = searchFilterSchema("search.optical.landsat.c2-l2.v1");

test("condition fields render the schema's date range and numeric field and report draft changes", () => {
  const changes: Array<[string, SearchFilterDraftValue]> = [];
  function Harness() {
    const [draft, setDraft] = useState(initialSearchFilterDraft(S2));
    return (
      <SchemaConditionFields
        schema={S2}
        draft={draft}
        onDraftChange={(filterId, value) => {
          changes.push([filterId, value]);
          setDraft((previous) => ({ ...previous, [filterId]: value }));
        }}
        disabled={false}
        labelPrefix="Sentinel-2"
        dateHelpSuffix="单次最多冻结 100 景。"
      />
    );
  }
  const view = render(<Harness />);
  try {
    assert.ok(view.getByText("日期条件"));
    assert.ok(view.getByText("日期留空表示全部时间；单次最多冻结 100 景。"));
    assert.ok(view.getByRole("button", { name: "日期范围：全部时间" }));
    assert.ok(view.getByText("最大云量（%）"));
    const cloud = view.getByRole("textbox", { name: "Sentinel-2 最大云量" });
    assert.equal(cloud.getAttribute("placeholder"), "留空不限制");
    fireEvent.change(cloud, { target: { value: "20" } });
    assert.deepEqual(changes.at(-1), ["cloud_cover", "20"]);
    assert.equal((view.getByRole("textbox", { name: "Sentinel-2 最大云量" }) as HTMLInputElement).value, "20");
  } finally {
    cleanup();
  }
});

test("facet fields hand every facet to the shared menu with empty-selection rules applied", () => {
  const calls: Parameters<SchemaMultiSelectRenderer>[0][] = [];
  const changes: Array<[string, SearchFilterDraftValue]> = [];
  const renderMultiSelect: SchemaMultiSelectRenderer = (args) => {
    calls.push(args);
    return <span>{args.ariaLabel}：{args.values.join("/") || "全部"}</span>;
  };
  const view = render(
    <SchemaFacetFields
      schema={LANDSAT}
      draft={initialSearchFilterDraft(LANDSAT)}
      onDraftChange={(filterId, value) => changes.push([filterId, value])}
      disabled={false}
      labelPrefix="Landsat Collection 2"
      renderMultiSelect={renderMultiSelect}
    />,
  );
  try {
    assert.deepEqual(
      [...view.container.querySelectorAll("[data-search-filter-group]")].map((node) => node.getAttribute("data-search-filter-group")),
      ["platform", "instrument", "product_level", "product_type"],
    );
    assert.deepEqual(calls.map((call) => call.ariaLabel), [
      "Landsat Collection 2 平台", "Landsat Collection 2 载荷族", "Landsat Collection 2 产品级别", "Landsat Collection 2 产品类型",
    ]);
    // All selected by default shows as "全部"; fixed facts carry their single option.
    // Contract provenance: contracts/v3/search-filter-schemas.v1.json (search.optical.landsat.c2-l2.v1)
    // admits Landsat 4/5/7/8/9 platforms with unified sensor family "tm_etm_oli_tirs".
    assert.deepEqual(calls[0].values, []);
    assert.deepEqual(calls[1].filter.options.map((option) => option.value), ["tm_etm_oli_tirs"]);
    // Clearing a restore_all multi-select restores every option; fixed facts ignore changes.
    calls[0].onChange([]);
    calls[3].onChange(["l2sr"]);
    calls[1].onChange(["anything"]);
    assert.deepEqual(changes, [
      ["platform", ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"]],
      ["product_type", ["l2sr"]],
    ]);
  } finally {
    cleanup();
  }
});

test("disabled facets and conditions are disabled together", () => {
  const view = render(
    <div>
      <SchemaFacetFields
        schema={S2}
        draft={initialSearchFilterDraft(S2)}
        onDraftChange={() => undefined}
        disabled
        labelPrefix="Sentinel-2"
        renderMultiSelect={({ ariaLabel, disabled }) => <button type="button" disabled={disabled}>{ariaLabel}</button>}
      />
      <SchemaConditionFields schema={S2} draft={initialSearchFilterDraft(S2)} onDraftChange={() => undefined} disabled labelPrefix="Sentinel-2" />
    </div>,
  );
  try {
    assert.equal(view.getByRole("button", { name: "Sentinel-2 平台" }).hasAttribute("disabled"), true);
    assert.equal(view.getByRole("textbox", { name: "Sentinel-2 最大云量" }).hasAttribute("disabled"), true);
  } finally {
    cleanup();
  }
});
