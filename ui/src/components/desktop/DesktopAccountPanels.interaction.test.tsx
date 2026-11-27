import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render, setupUser } from "@/test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { createAccountSettingsFixture } from "@/test-utils/account-settings-fixture";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "@/lib/desktopShellBridge";
import {
  DesktopCdseAccountPanel,
  DesktopEarthdataAccountPanel,
  DesktopUsgsAccountPanel,
  DesktopBhoonidhiAccountPanel,
} from "./DesktopAccountPanels";

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

test("CDSE saves a complete draft on blur without a save button and login verifies saved credentials", async () => {
  const fixture = createAccountSettingsFixture();
  registerTestDesktopShellBridge(fixture.bridge);
  const view = render(<DesktopCdseAccountPanel />);
  const user = setupUser();
  assert.equal(view.queryByRole("button", { name: "保存凭据" }), null);

  // 验证已保存账号回显用户名
  const userInput = view.getByLabelText("CDSE 用户名") as HTMLInputElement;
  await waitFor(() => assert.equal(userInput.value, "fixture-user"));

  // 用户清空后输入新的替换用户名
  await user.clear(userInput);
  await user.type(userInput, "replacement-user");
  fireEvent.blur(userInput);
  assert.equal(fixture.state.writes, 0);

  const pwdInput = view.getByLabelText("CDSE 密码") as HTMLInputElement;
  await user.type(pwdInput, "synthetic replacement");
  await waitFor(() => assert.equal((view.getByLabelText("CDSE 密码") as HTMLInputElement).value, "synthetic replacement"));
  fireEvent.blur(pwdInput);
  await waitFor(() => assert.ok(view.queryByText("保存中…") || view.queryByText("已保存")));
  await waitFor(() => {
    assert.equal(fixture.state.writes, 1);
    assert.equal(fixture.values.get("cdse_username"), "replacement-user");
    assert.equal(fixture.values.get("cdse_password"), "synthetic replacement");
  });
  assert.equal(fixture.state.writes, 1);
  const login = await waitFor(() => {
    const button = view.getByRole("button", { name: "登录" }) as HTMLButtonElement;
    assert.equal(button.disabled, false);
    return button;
  });
  fireEvent.click(login);
  await view.findByText("账号有效");
  assert.deepEqual(fixture.state.checks, ["cdse"]);
});

test("Earthdata token auto-saves after a short pause and verification calls the forced auth check", async () => {
  const fixture = createAccountSettingsFixture();
  registerTestDesktopShellBridge(fixture.bridge);
  const view = render(<DesktopEarthdataAccountPanel />);
  const initialVerify = view.getByRole("button", { name: "验证" }) as HTMLButtonElement;
  await waitFor(() => assert.equal(initialVerify.disabled, false));
  fireEvent.click(view.getByRole("button", { name: "Token" }));
  await setupUser().type(view.getByLabelText("Earthdata Token"), "synthetic-token");
  await waitFor(() => assert.equal((view.getByLabelText("Earthdata Token") as HTMLInputElement).value, "synthetic-token"));
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(fixture.state.writes, 0);
  await waitFor(
    () => {
      assert.equal(fixture.state.writes, 1);
      assert.equal(fixture.values.get("earthdata_token"), "synthetic-token");
      assert.equal(fixture.values.has("earthdata_username"), false);
    },
    { timeout: 1500 },
  );
  assert.equal(fixture.state.writes, 1);
  const verify = await waitFor(() => {
    const button = view.getByRole("button", { name: "验证" }) as HTMLButtonElement;
    assert.equal(button.disabled, false);
    return button;
  });
  fireEvent.click(verify);
  await view.findByText("账号有效");
  assert.deepEqual(fixture.state.checks, ["earthdata"]);
});

test("Earthdata username and password mode auto-saves through the login credential bridge", async () => {
  const fixture = createAccountSettingsFixture();
  registerTestDesktopShellBridge(fixture.bridge);
  const view = render(<DesktopEarthdataAccountPanel />);
  fireEvent.click(view.getByRole("button", { name: "用户名 + 密码" }));
  const user = setupUser();

  // 验证已保存账号回显用户名
  const userInput = view.getByLabelText("Earthdata 用户名") as HTMLInputElement;
  await waitFor(() => assert.equal(userInput.value, "fixture-user"));

  await user.clear(userInput);
  await user.type(userInput, "different-user");
  await user.type(view.getByLabelText("Earthdata 密码"), "synthetic-password");
  await waitFor(() => assert.equal((view.getByLabelText("Earthdata 密码") as HTMLInputElement).value, "synthetic-password"));
  fireEvent.blur(view.getByLabelText("Earthdata 密码"));
  await waitFor(() => assert.equal(view.getByRole("status").textContent, "已保存"));
  assert.equal(fixture.state.writes, 1);
  assert.equal(fixture.values.get("earthdata_username"), "different-user");
  assert.equal(fixture.values.get("earthdata_password"), "synthetic-password");
  assert.equal(fixture.values.has("earthdata_token"), false);
  assert.equal(view.queryByRole("button", { name: "保存凭据" }), null);
});

test("Earthdata reports detected netrc validity and keeps the replacement form visible", async () => {
  const fixture = createAccountSettingsFixture();
  const credentials = fixture.bridge.credentials!;
  const readStatus = credentials.getStatus;
  credentials.getStatus = async () => {
    const result = await readStatus();
    return result.ok ? { ...result, status: { ...result.status, earthdata: "netrc", earthdata_input: { mode: "netrc", token: "", username: "", password: "" } } } : result;
  };
  credentials.checkEarthdataAuth = async () => ({ ok: true, configured: true, status: "expired", message: "凭据已过期" });
  registerTestDesktopShellBridge(fixture.bridge);
  const view = render(<DesktopEarthdataAccountPanel />);
  await view.findByText("已检测到本机 .netrc 凭据");
  assert.ok(view.getByLabelText("Earthdata Token"));
  fireEvent.click(view.getByRole("button", { name: "验证" }));
});

test("Saved credential inputs display masked placeholder, keep-blank notice, and toggle reveal", async () => {
  const fixture = createAccountSettingsFixture();
  registerTestDesktopShellBridge(fixture.bridge);
  const view = render(<DesktopCdseAccountPanel />);
  const user = setupUser();

  // 1. 回显真实已保存用户名
  await waitFor(() => {
    const userElem = view.getByLabelText("CDSE 用户名") as HTMLInputElement;
    assert.equal(userElem.value, "fixture-user");
  });

  // 2. 密码框显示遮罩占位符与「已保存」徽标（无冗余留空提示）
  const pwdElem = view.getByLabelText("CDSE 密码") as HTMLInputElement;
  assert.equal(pwdElem.value, "");
  assert.equal(pwdElem.placeholder, "••••••••");
  assert.ok(view.queryByText("已保存，留空即保持不变") === null);
  assert.ok(view.getAllByText("已保存").length >= 1);

  // 3. 点击眼睛按钮触发 reveal，显示真实明文密码
  const eyeBtn = view.getByRole("button", { name: "显示" });
  await user.click(eyeBtn);

  await waitFor(() => {
    assert.equal(pwdElem.value, "synthetic password");
    assert.equal(pwdElem.type, "text");
  });

  // 4. 再次点击眼睛按钮（隐藏），清空内存明文并恢复遮罩占位符
  const hideBtn = view.getByRole("button", { name: "隐藏" });
  await user.click(hideBtn);

  await waitFor(() => {
    assert.equal(pwdElem.value, "");
    assert.equal(pwdElem.type, "password");
    assert.equal(pwdElem.placeholder, "••••••••");
  });

  // 5. A5 字体与输入类规范：用户名用 font-sans，密钥用 font-mono + placeholder:font-sans，均统一 eo-settings-input
  const userElem = view.getByLabelText("CDSE 用户名") as HTMLInputElement;
  assert.ok(userElem.className.includes("eo-settings-input"));
  assert.ok(userElem.className.includes("font-sans"));
  assert.ok(pwdElem.className.includes("eo-settings-input"));
  assert.ok(pwdElem.className.includes("font-mono"));
});

test("Earthdata unknown/unavailable probes do not block catalog sources while invalid marks failure", async () => {
  const { isAccountVerificationFailed, setAccountVerificationFailed } = await import("@/lib/dataSourceCatalogLogic");
  setAccountVerificationFailed("earthdata", false);

  const fixture = createAccountSettingsFixture();
  const credentials = fixture.bridge.credentials!;
  credentials.checkEarthdataAuth = async () => ({
    ok: true,
    configured: true,
    status: "unknown",
    message: "Earthdata Login 服务可达",
  });
  registerTestDesktopShellBridge(fixture.bridge);
  const view = render(<DesktopEarthdataAccountPanel />);
  const verifyBtn = await waitFor(() => {
    const btn = view.getByRole("button", { name: "验证" }) as HTMLButtonElement;
    assert.equal(btn.disabled, false);
    return btn;
  });

  fireEvent.click(verifyBtn);
  await view.findByText("Earthdata Login 服务可达");
  assert.equal(isAccountVerificationFailed("earthdata"), false);

  credentials.checkEarthdataAuth = async () => ({
    ok: true,
    configured: true,
    status: "invalid",
    message: "Earthdata Login 拒绝了当前凭据",
  });
  fireEvent.click(verifyBtn);
  await view.findByText("Earthdata Login 拒绝了当前凭据");
  assert.equal(isAccountVerificationFailed("earthdata"), true);
  setAccountVerificationFailed("earthdata", false);
});
