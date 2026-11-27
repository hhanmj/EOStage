import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { cleanup, fireEvent, render, setupUser } from "@/test-utils/render-harness";
import { ProviderCredentialField } from "./ProviderCredentialField";
import type { DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";

test.afterEach(() => cleanup());

test("pending reveal can be hidden and a late result never returns to the original field", async () => {
  const user = setupUser();
  let resolve!: (value: DesktopCredentialRevealResult) => void;
  const view = render(<ProviderCredentialField label="Key" value="" onChange={() => undefined} secret
    savedCredential={{ identity: "one", field: "opentopography_key", onReveal: () =>
      new Promise((done) => { resolve = done; }) }} />);
  const input = view.getByLabelText("Key") as HTMLInputElement;
  await user.click(view.getByRole("button", { name: "显示已保存的 Key" }));
  assert.equal(input.getAttribute("aria-busy"), "true");
  const hide = view.getByRole("button", { name: "隐藏已保存的 Key" }) as HTMLButtonElement;
  assert.equal(hide.disabled, false);
  await user.click(hide);
  await act(async () => { resolve({ ok: true, value: "late-fixture" }); });
  assert.equal(input.value, "");
  assert.equal(input.readOnly, false);
  assert.equal(input.getAttribute("aria-busy"), "false");
});

test("disabled, replaced draft, binding changes and unmount invalidate saved display requests", async () => {
  const user = setupUser();
  for (const change of ["disabled", "draft", "identity", "epoch", "field", "removed", "unmount"] as const) {
    let resolve!: (value: DesktopCredentialRevealResult) => void;
    const binding = { identity: "one", epoch: 0, field: "opentopography_key" as const,
      onReveal: () => new Promise<DesktopCredentialRevealResult>((done) => { resolve = done; }) };
    const props = { label: "Key", value: "", onChange: () => undefined, secret: true, savedCredential: binding };
    const view = render(<ProviderCredentialField {...props} />);
    await user.click(view.getByRole("button"));
    if (change === "unmount") view.unmount();
    else view.rerender(<ProviderCredentialField {...props}
      disabled={change === "disabled"}
      value={change === "draft" ? "replacement-fixture" : ""}
      savedCredential={change === "removed" ? undefined : { ...binding,
        identity: change === "identity" ? "two" : "one",
        epoch: change === "epoch" ? 1 : 0,
        field: change === "field" ? "earthdata_token" : binding.field }} />);
    await act(async () => { resolve({ ok: true, value: "late-fixture" }); });
    const input = view.queryByLabelText("Key") as HTMLInputElement | null;
    assert.notEqual(input?.value, "late-fixture", change);
    if (change === "draft") {
      assert.equal(input?.value, "replacement-fixture");
      // Clearing the draft must not resurrect a previously resolved saved value.
      view.rerender(<ProviderCredentialField {...props} />);
      assert.equal((view.getByLabelText("Key") as HTMLInputElement).value, "");
    }
    cleanup();
  }
});

test("saved display reports sanitized failures without copying material into drafts", async () => {
  const user = setupUser();
  for (const failure of ["missing", "rejected", "throws"] as const) {
    let writes = 0;
    const view = render(<ProviderCredentialField label="Key" value="" secret onChange={() => { writes++; }}
      savedCredential={{ identity: "one", field: "opentopography_key", onReveal: async () => {
        if (failure === "throws") throw new Error("sensitive-fixture-message");
        return { ok: failure === "missing", value: null, error: "sensitive-fixture-message" };
      } }} />);
    await user.click(view.getByRole("button"));
    assert.ok(view.getByRole("alert"));
    assert.equal(view.container.textContent?.includes("sensitive-fixture-message"), false);
    assert.equal((view.getByLabelText("Key") as HTMLInputElement).value, "");
    assert.equal(writes, 0);
    cleanup();
  }
});

test("saved material is revealed in the original field and destroyed again on hide", async () => {
  const user = setupUser();
  let reads = 0;
  const view = render(
    <ProviderCredentialField
      label="API Key"
      credentialLabel="OpenTopography Key"
      value=""
      onChange={() => undefined}
      secret
      savedCredential={{
        identity: "configured",
        field: "opentopography_key",
        onReveal: async () => {
          reads += 1;
          return { ok: true, value: "saved-secret" };
        },
      }}
    />,
  );
  const input = view.getByLabelText("OpenTopography Key") as HTMLInputElement;
  assert.equal(input.value, "");
  assert.equal(input.placeholder, "••••••••");
  assert.equal(reads, 0);

  await user.click(view.getByRole("button", { name: "显示已保存的 OpenTopography Key" }));
  assert.equal(reads, 1);
  assert.equal(input.value, "saved-secret");
  assert.equal(input.readOnly, true);

  await user.click(view.getByRole("button", { name: "隐藏已保存的 OpenTopography Key" }));
  assert.equal(input.value, "");
  assert.equal(input.readOnly, false);
});

test("typing a replacement draft leaves saved material untouched and uses the same input", async () => {
  const user = setupUser();
  let draft = "";
  let reads = 0;
  const view = render(
    <ProviderCredentialField
      label="接收邮箱"
      credentialLabel="GACOS 邮箱"
      value={draft}
      onChange={(value) => { draft = value; }}
      inputMode="email"
      savedCredential={{
        identity: "configured",
        field: "gacos_email",
        onReveal: async () => {
          reads += 1;
          return { ok: true, value: "saved@example.com" };
        },
      }}
    />,
  );
  const input = view.getByLabelText("GACOS 邮箱") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "new@example.com" } });
  assert.equal(draft, "new@example.com");
  assert.equal(reads, 0);
  assert.equal(view.queryByLabelText("GACOS 邮箱 已保存值"), null);
  assert.equal(view.queryByText("GACOS 邮箱 · 已保存"), null);

  // The component stays controlled; a parent rerender owns the new draft value.
  view.rerender(
    <ProviderCredentialField
      label="接收邮箱"
      credentialLabel="GACOS 邮箱"
      value={draft}
      onChange={(value) => { draft = value; }}
      inputMode="email"
      savedCredential={{ identity: "configured", field: "gacos_email", onReveal: async () => ({ ok: true, value: "saved@example.com" }) }}
    />,
  );
  assert.equal((view.getByLabelText("GACOS 邮箱") as HTMLInputElement).value, "new@example.com");
  assert.equal(view.queryByRole("button", { name: "显示已保存的 GACOS 邮箱" }), null);
  await user.clear(view.getByLabelText("GACOS 邮箱"));
});
