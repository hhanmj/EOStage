import assert from "node:assert/strict";
import test from "node:test";

import React, { useState } from "react";

import type { AoiPreviewOk } from "@/lib/bridge";
import {
  initialGacosRequestDraft,
  type GacosRequestDraft,
  type GacosSentinelBatchReference,
} from "@/lib/gacosRequestDraft";
import { cleanup, fireEvent, render, setupUser } from "@/test-utils/render-harness";
import { GacosRequestInputs } from "./GacosRequestInputs";

const polygonA = {
  type: "Polygon",
  coordinates: [[[110, 20], [111, 20], [111, 21], [110, 20]]],
};
const polygonB = {
  type: "Polygon",
  coordinates: [[[112, 22], [113, 22], [113, 23], [112, 22]]],
};

const latestBatch: GacosSentinelBatchReference = {
  batchId: "sentinel-batch-1",
  label: "Sentinel-1 最新完整检索",
  totalCount: 4,
};

const preview: AoiPreviewOk = {
  ok: true,
  path: "F:/fixtures/boundary.geojson",
  file_name: "boundary.geojson",
  total_features: 2,
  fields: ["name"],
  features: [
    { id: "a", index: 1, source_index: 0, name: "河谷", properties: { name: "河谷" } },
    { id: "b", index: 2, source_index: 1, name: "山地", properties: { name: "山地" } },
  ],
  geojson: {
    type: "FeatureCollection",
    features: [
      { type: "Feature", id: "a", properties: { name: "河谷" }, geometry: polygonA },
      { type: "Feature", id: "b", properties: { name: "山地" }, geometry: polygonB },
    ],
  },
};

function Harness({
  initialValue,
  initialLatest = latestBatch,
  globalName = "全局 AOI",
}: {
  initialValue?: GacosRequestDraft;
  initialLatest?: GacosSentinelBatchReference | null;
  globalName?: string;
}) {
  const [value, setValue] = useState<GacosRequestDraft>(() => structuredClone(initialValue ?? initialGacosRequestDraft));
  const [latest, setLatest] = useState<GacosSentinelBatchReference | null>(() => structuredClone(initialLatest));
  const [globalAoiName] = useState(globalName);
  return (
    <>
      <GacosRequestInputs
        value={value}
        onChange={setValue}
        busy={false}
        latestSentinelBatch={latest}
        onImportCurrentAoi={async () => ({ name: "当前 AOI 快照", geometry: structuredClone(polygonA) })}
        onImportAoiFile={async () => structuredClone(preview)}
      />
      <output data-testid="draft-json">{JSON.stringify(value)}</output>
      <output data-testid="global-aoi">{globalAoiName}</output>
      <button
        type="button"
        onClick={() => setLatest({ batchId: "sentinel-batch-2", label: "后来更新的结果", totalCount: 99 })}
      >
        更新最新结果引用
      </button>
    </>
  );
}

function readDraft(view: ReturnType<typeof render>): GacosRequestDraft {
  return JSON.parse(view.getByTestId("draft-json").textContent ?? "null") as GacosRequestDraft;
}

test.afterEach(() => cleanup());

test("online source is frozen only by the explicit latest-results click", async () => {
  const user = setupUser();
  const view = render(<Harness />);

  await user.click(view.getByRole("button", { name: "导入最新 Sentinel-1 结果（全部 4 景）" }));
  const captured = readDraft(view).sourceBatch;
  assert.deepEqual(captured, latestBatch);

  await user.click(view.getByRole("button", { name: "更新最新结果引用" }));
  assert.deepEqual(readDraft(view).sourceBatch, latestBatch);
  cleanup();
});

test("manual mode remains usable without any Sentinel-1 prop", async () => {
  const user = setupUser();
  const view = render(<Harness initialLatest={null} initialValue={{
    ...structuredClone(initialGacosRequestDraft),
    mode: "manual_dates",
    datesText: "20260901",
    hour: "1",
    minute: "2",
    aoi: { name: "当前 AOI", geometry: structuredClone(polygonA) },
  }} />);

  assert.equal(view.queryByRole("button", { name: /导入最新 Sentinel-1/ }), null);
  assert.match(view.getByText("输入完整，可以生成请求预览").textContent ?? "", /输入完整/);
  await user.click(view.getByRole("radio", { name: "Binary grid" }));
  assert.equal(readDraft(view).outputFormat, "binary");
  cleanup();
});

test("manual date and UTC fields surface actionable validation errors", async () => {
  const user = setupUser();
  const view = render(<Harness initialLatest={null} initialValue={{
    ...structuredClone(initialGacosRequestDraft),
    mode: "manual_dates",
    aoi: { name: "当前 AOI", geometry: structuredClone(polygonA) },
  }} />);

  await user.type(view.getByRole("textbox", { name: "GACOS 日期（每行 YYYYMMDD）" }), "20260230");
  fireEvent.change(view.getByRole("combobox", { name: "UTC 小时" }), { target: { value: "24" } });
  assert.match(view.getByRole("alert").textContent ?? "", /有效的日历日期|UTC 分钟/);
  cleanup();
});

test("local AOI feature selection combines selected geometries without changing global AOI", async () => {
  const user = setupUser();
  const view = render(<Harness initialLatest={null} />);

  await user.click(view.getByRole("button", { name: "导入本地 AOI" }));
  await user.click(view.getByRole("checkbox", { name: "河谷 AOI 要素" }));
  await user.click(view.getByRole("checkbox", { name: "山地 AOI 要素" }));
  await user.click(view.getByRole("button", { name: /应用所选 AOI（2）/ }));

  const next = readDraft(view);
  assert.equal(next.aoi?.name, "boundary.geojson");
  assert.equal(next.aoi?.geometry.type, "MultiPolygon");
  assert.equal(view.getByTestId("global-aoi").textContent, "全局 AOI");
  cleanup();
});

test("collapsing the input summary never hides blocking errors or changes the draft", async () => {
  const user = setupUser();
  const view = render(<Harness initialLatest={null} />);
  const details = view.getByLabelText("提交前预览") as HTMLDetailsElement;
  const before = readDraft(view);
  assert.equal(details.open, false);
  assert.equal(view.getByRole("alert").closest("details"), null);
  await user.click(details.querySelector("summary")!);
  assert.equal(details.open, true);
  assert.deepEqual(readDraft(view), before);
  await user.click(details.querySelector("summary")!);
  assert.equal(details.open, false);
  assert.deepEqual(readDraft(view), before);
  assert.ok(view.getByRole("alert"));
});

test("busy GACOS inputs retain their values and disable import and scientific choices", () => {
  const draft = structuredClone(initialGacosRequestDraft);
  const view = render(<GacosRequestInputs value={draft} onChange={() => assert.fail("busy inputs must not publish")}
    busy latestSentinelBatch={latestBatch}
    onImportCurrentAoi={async () => ({ name: "当前 AOI", geometry: polygonA })}
    onImportAoiFile={async () => null} />);
  for (const name of ["导入当前 AOI", "导入本地 AOI"]) {
    assert.equal((view.getByRole("button", { name }) as HTMLButtonElement).disabled, true);
  }
  assert.equal((view.getByRole("combobox", { name: "UTC 小时" }) as HTMLSelectElement).disabled, true);
  assert.equal((view.getByRole("radio", { name: "Binary grid" }) as HTMLInputElement).disabled, true);
});
