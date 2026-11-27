import assert from "node:assert/strict";
import test from "node:test";

import React, { useState } from "react";

import { cleanup, fireEvent, render } from "@/test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import {
  GacosImportRequestSelector,
} from "./GacosImportRequestSelector";
import type { GacosImportRequestChoice } from "@/lib/gacosImportRequest";

const choices: GacosImportRequestChoice[] = [
  {
    key: "request|utc-1",
    taskId: "request",
    itemId: "utc-1",
    state: "manual_verification_required",
    label: "2026-09-01 · UTC 01:02 · 范围 110.0000, 20.0000, 111.0000, 21.0000",
  },
  {
    key: "request|utc-2",
    taskId: "request",
    itemId: "utc-2",
    state: "running",
    label: "2026-09-02 · UTC 03:04 · 范围 112.0000, 22.0000, 113.0000, 23.0000",
  },
];

function ControlledSelector({
  initialKey,
  values,
}: {
  initialKey: string;
  values: readonly GacosImportRequestChoice[];
}) {
  const [selectedKey, setSelectedKey] = useState(initialKey);
  return (
    <>
      <GacosImportRequestSelector
        choices={values}
        selectedKey={selectedKey}
        onSelectedKeyChange={setSelectedKey}
      />
      <div role="status" aria-label="selected request">{selectedKey}</div>
    </>
  );
}

test.afterEach(() => cleanup());

test("request picker clears a stale key and exposes the surviving UTC group", async () => {
  const view = render(<ControlledSelector initialKey="old-request|old-item" values={[choices[1]!]} />);

  await waitFor(() => assert.equal(
    view.getByRole("status", { name: "selected request" }).textContent,
    choices[1]!.key,
  ));
  assert.match(view.container.textContent ?? "", /2026-09-02 · UTC 03:04/);
});

test("request picker lets the user choose the exact multi-UTC result group", () => {
  const view = render(<ControlledSelector initialKey="" values={choices} />);
  const select = view.getByRole("combobox", { name: "结果所属的 GACOS request 项目" });

  fireEvent.change(select, { target: { value: choices[1]!.key } });
  assert.equal((select as HTMLSelectElement).value, choices[1]!.key);
  assert.equal(view.getByRole("status", { name: "selected request" }).textContent, choices[1]!.key);
});
