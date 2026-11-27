import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, render, setupUser } from "@/test-utils/render-harness";
import { CredentialActionBar } from "./CredentialActionBar";

test.afterEach(() => cleanup());

test("credential actions share equal-width compact controls with a responsive minimum width", async () => {
  const user = setupUser();
  let clicked = "";
  const view = render(
    <CredentialActionBar
      id="fixture"
      actions={[
        { key: "refresh", label: "刷新状态", onClick: () => { clicked = "refresh"; } },
        { key: "check", label: "检测登录", onClick: () => { clicked = "check"; } },
        { key: "clear", label: "清除凭证", onClick: () => { clicked = "clear"; } },
      ]}
    />,
  );
  const bar = view.container.querySelector('[data-credential-action-bar="fixture"]');
  assert.ok(bar);
  assert.ok(bar.classList.contains("grid-cols-[repeat(auto-fit,minmax(min(100%,6rem),1fr))]"));
  for (const label of ["刷新状态", "检测登录", "清除凭证"]) {
    const button = view.getByRole("button", { name: label });
    assert.match(button.className, /h-8/);
    assert.match(button.className, /w-full/);
  }
  await user.click(view.getByRole("button", { name: "清除凭证" }));
  assert.equal(clicked, "clear");
});

test("credential action bar preserves every action rather than truncating after four", () => {
  for (const count of [1, 2, 4, 5]) {
    const view = render(
      <CredentialActionBar
        id={`fixture-${count}`}
        actions={Array.from({ length: count }, (_, index) => ({ key: String(index), label: `动作${index + 1}` }))}
      />,
    );
    const bar = view.container.querySelector(`[data-credential-action-bar="fixture-${count}"]`);
    assert.ok(bar);
    assert.equal(view.getAllByRole("button").length, count);
    assert.ok(bar.classList.contains("grid-cols-[repeat(auto-fit,minmax(min(100%,6rem),1fr))]"));
    cleanup();
  }
});

test("busy and disabled actions cannot submit or invoke callbacks", async () => {
  const user = setupUser();
  let calls = 0;
  const view = render(<CredentialActionBar id="busy" actions={[
    { key: "refresh", label: "刷新状态", busy: true, onClick: () => { calls++; } },
    { key: "clear", label: "清除凭证", disabled: true, onClick: () => { calls++; } },
  ]} />);
  const refresh = view.getByRole("button", { name: "刷新状态" });
  assert.equal(refresh.getAttribute("aria-busy"), "true");
  await user.click(refresh);
  await user.click(view.getByRole("button", { name: "清除凭证" }));
  assert.equal(calls, 0);
});
