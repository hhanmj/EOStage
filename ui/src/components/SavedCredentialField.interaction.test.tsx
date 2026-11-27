import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { cleanup, render, setupUser } from "@/test-utils/render-harness";
import { SavedCredentialField } from "./SavedCredentialField";
import type { DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";
test.afterEach(() => cleanup());
const props = { label: "fixture", field: "bhoonidhi_password" as const, identity: "one" };

test("saved value stays a masked input placeholder until explicit reveal; hide destroys it and reads again", async () => {
  let reads = 0;
  const user = setupUser();
  const view = render(<SavedCredentialField {...props} onReveal={async () => {
    reads++; return { ok: true, value: "fixture-display" };
  }} />);
  const input = view.getByLabelText("fixture 已保存值") as HTMLInputElement;
  assert.equal(reads, 0);
  assert.equal(input.readOnly, true);
  assert.equal(input.type, "password");
  assert.equal(input.value, "");
  assert.equal(input.placeholder, "••••••••");
  await user.click(view.getByRole("button"));
  assert.equal((view.getByLabelText("fixture 已保存值") as HTMLInputElement).value, "fixture-display");
  assert.equal((view.getByLabelText("fixture 已保存值") as HTMLInputElement).type, "text");
  await user.click(view.getByRole("button"));
  assert.equal((view.getByLabelText("fixture 已保存值") as HTMLInputElement).value, "");
  assert.equal((view.getByLabelText("fixture 已保存值") as HTMLInputElement).type, "password");
  await user.click(view.getByRole("button"));
  assert.equal(reads, 2);
});

test("late results are discarded after hide, account change and unmount", async () => {
  const user = setupUser();
  for (const action of ["hide", "account", "unmount"]) {
    let resolve!: (value: DesktopCredentialRevealResult) => void;
    const onReveal = () => new Promise<DesktopCredentialRevealResult>((done) => { resolve = done; });
    const view = render(<SavedCredentialField {...props} onReveal={onReveal} />);
    await user.click(view.getByRole("button"));
    if (action === "hide") await user.click(view.getByRole("button"));
    if (action === "account") view.rerender(<SavedCredentialField {...props} identity="two" onReveal={onReveal} />);
    if (action === "unmount") view.unmount();
    await act(async () => { resolve({ ok: true, value: "late-fixture" }); });
    const input = view.queryByLabelText("fixture 已保存值") as HTMLInputElement | null;
    assert.notEqual(input?.value, "late-fixture");
    cleanup();
  }
});

test("missing saved material and failures never show raw errors", async () => {
  const user = setupUser();
  for (const response of [null, "throw"] as const) {
    const view = render(<SavedCredentialField {...props} onReveal={async () => {
      if (response === "throw") throw new Error("fixture-sensitive-error");
      return { ok: true, value: null };
    }} />);
    await user.click(view.getByRole("button"));
    assert.ok(await view.findByRole("alert"));
    assert.equal(view.queryByText("fixture-sensitive-error"), null);
    const input = view.getByLabelText("fixture 已保存值") as HTMLInputElement;
    assert.equal(input.value, "");
    assert.equal(input.type, "password");
    cleanup();
  }
});
