import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render, setupUser } from "@/test-utils/render-harness";
import { BhoonidhiCredentialPanel } from "./BhoonidhiCredentialPanel";

test.afterEach(() => cleanup());

test("only explicit login detection enters the Bhoonidhi session check", async () => {
  const user = setupUser();
  let checks = 0;
  let refreshes = 0;
  const view = render(<BhoonidhiCredentialPanel status="login:f***"
    onSave={success} onClear={success} onChanged={async () => { refreshes++; }}
    onCheckAuth={async () => { checks++; return { ok: true, session_available: true, catalogue_verified: false }; }} />);
  assert.equal(checks, 0);
  await user.click(view.getByRole("button", { name: "刷新状态" }));
  assert.equal(refreshes, 1);
  assert.equal(checks, 0);
  await user.click(view.getByRole("button", { name: "检测登录" }));
  assert.equal(checks, 1);
  assert.equal(refreshes, 2);
  assert.equal((await view.findByRole("status")).textContent, "登录可用。");
});

test("saved reveal stays display-only and deletion is an explicit action", async () => {
  const user = setupUser();
  let reads = 0;
  let cleared = 0;
  const view = render(<BhoonidhiCredentialPanel status="login:f***"
    onSave={success} onClear={async () => { cleared++; return { ok: true }; }} onChanged={success}
    onReveal={async () => { reads++; return { ok: true, value: "fixture-display-only" }; }} />);
  assert.equal(reads, 0);
  await user.click(view.getByRole("button", { name: "显示已保存的 Bhoonidhi 密码" }));
  assert.equal((view.getByLabelText("Bhoonidhi 密码") as HTMLInputElement).value, "fixture-display-only");
  assert.equal((view.getByLabelText("Bhoonidhi 密码") as HTMLInputElement).readOnly, true);
  await user.click(view.getByRole("button", { name: "隐藏已保存的 Bhoonidhi 密码" }));
  assert.equal((view.getByLabelText("Bhoonidhi 密码") as HTMLInputElement).value, "");
  assert.equal((view.getByRole("button", { name: "保存凭据" }) as HTMLButtonElement).disabled, true);
  await user.click(view.getByRole("button", { name: "清除凭据" }));
  assert.equal(cleared, 1);
  assert.equal((view.getByLabelText("Bhoonidhi 密码") as HTMLInputElement).value, "");
});

const success = async () => ({ ok: true as const });

function fill(view: ReturnType<typeof render>) {
  fireEvent.change(view.getByLabelText("Bhoonidhi 用户名"), { target: { value: "  fixture-user  " } });
  fireEvent.change(view.getByLabelText("Bhoonidhi 密码"), { target: { value: "fixture-only" } });
}

test("save invokes the supplied credential action and clears submitted fields without claiming login", async () => {
  const user = setupUser();
  let saved = 0;
  let refreshed = 0;
  const view = render(<BhoonidhiCredentialPanel status="none" onClear={success}
    onSave={async (username, password) => {
      saved++;
      assert.equal(username, "fixture-user");
      assert.equal(password, "fixture-only");
      return { ok: true };
    }} onChanged={async () => { refreshed++; }} />);
  fill(view);
  await user.click(view.getByRole("button", { name: "保存凭据" }));
  await view.findByRole("status");
  assert.equal(saved, 1);
  assert.equal(refreshed, 1);
  assert.equal((view.getByLabelText("Bhoonidhi 密码") as HTMLInputElement).value, "");
  assert.equal((view.getByLabelText("Bhoonidhi 用户名") as HTMLInputElement).value, "");
  assert.equal(view.getByRole("status").textContent, "凭据已保存。");
});

test("a rejected save shows a failure, clears password, and does not report success", async () => {
  const user = setupUser();
  let refreshed = 0;
  const view = render(<BhoonidhiCredentialPanel status="none" onClear={success}
    onSave={async () => ({ ok: false, error: "凭据保存失败", code: "DL004" })}
    onChanged={async () => { refreshed++; }} />);
  fill(view);
  await user.click(view.getByRole("button", { name: "保存凭据" }));
  await view.findByRole("alert");
  assert.equal(refreshed, 0);
  assert.equal(view.queryByRole("status"), null);
  assert.equal((view.getByLabelText("Bhoonidhi 密码") as HTMLInputElement).value, "");
});

test("explicit clear deletes without resubmitting credentials and refreshes projected status once", async () => {
  const user = setupUser();
  let saved = 0;
  let cleared = 0;
  let refreshed = 0;
  const view = render(<BhoonidhiCredentialPanel status="login:f***"
    onSave={async () => { saved++; return { ok: true }; }}
    onClear={async () => { cleared++; return { ok: true }; }}
    onChanged={async () => { refreshed++; }} />);
  await user.click(view.getByRole("button", { name: "清除凭据" }));
  await view.findByRole("status");
  assert.equal(saved, 0);
  assert.equal(cleared, 1);
  assert.equal(refreshed, 1);
});

test("Bhoonidhi exposes the supplied registration entry without coupling it to credential saving", async () => {
  const user = setupUser();
  let registered = 0;
  const view = render(<BhoonidhiCredentialPanel status="none" onSave={success} onClear={success}
    onChanged={success} onRegister={() => { registered++; }} />);
  await user.click(view.getByRole("button", { name: "注册" }));
  assert.equal(registered, 1);
});

test("empty drafts never save and explicit clear is admitted only for an existing saved login", () => {
  const configured = render(<BhoonidhiCredentialPanel status="login:f***" onSave={success} onClear={success} onChanged={success} />);
  assert.equal((configured.getByRole("button", { name: "保存凭据" }) as HTMLButtonElement).disabled, true);
  assert.equal((configured.getByRole("button", { name: "清除凭据" }) as HTMLButtonElement).disabled, false);
  cleanup();
  const unavailable = render(<BhoonidhiCredentialPanel status="unavailable" onSave={success} onClear={success} onChanged={success} />);
  assert.equal((unavailable.getByRole("button", { name: "保存凭据" }) as HTMLButtonElement).disabled, true);
  assert.equal((unavailable.getByRole("button", { name: "清除凭据" }) as HTMLButtonElement).disabled, true);
  assert.ok(unavailable.getByText("凭据服务不可用"));
  assert.equal(unavailable.queryByText("已登录"), null);
});

test("credential action bar exposes save refresh clear and login detection with one shared layout", () => {
  const view = render(<BhoonidhiCredentialPanel status="login:f***" onSave={success} onClear={success} onChanged={success}
    onCheckAuth={async () => ({ ok: true, session_available: true, catalogue_verified: false })} />);
  const actions = view.container.querySelector('[data-credential-action-bar="bhoonidhi"]');
  assert.ok(actions);
  assert.ok(actions.classList.contains("grid-cols-[repeat(auto-fit,minmax(min(100%,6rem),1fr))]"));
  assert.equal(actions.querySelectorAll("button").length, 4);
  assert.ok(view.getByRole("button", { name: "刷新状态" }));
  assert.ok(view.getByRole("button", { name: "清除凭据" }));
  assert.ok(view.getByRole("button", { name: "检测登录" }));
  assert.ok(view.getByRole("button", { name: "保存凭据" }));
});
