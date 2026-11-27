import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { cleanup, fireEvent, render, setupUser } from "@/test-utils/render-harness";
import { ProviderAccountFields } from "./ProviderAccountFields";

test.afterEach(() => cleanup());

function Harness({ disabled = false }: { disabled?: boolean }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  return <ProviderAccountFields provider="CDSE" username={username} password={password}
    onUsernameChange={setUsername} onPasswordChange={setPassword} disabled={disabled}
    oneTimeCode={{ value: code, onChange: setCode }} />;
}

test("CDSE fields retain separate controlled username, password and optional OTP", () => {
  const view = render(<Harness />);
  const username = view.getByLabelText("CDSE 用户名") as HTMLInputElement;
  const password = view.getByLabelText("CDSE 密码") as HTMLInputElement;
  const otp = view.getByLabelText("CDSE 一次性验证码") as HTMLInputElement;
  fireEvent.change(username, { target: { value: "fixture-user" } });
  fireEvent.change(password, { target: { value: "fixture-only" } });
  fireEvent.change(otp, { target: { value: "123456" } });
  assert.equal(username.value, "fixture-user");
  assert.equal(password.value, "fixture-only");
  assert.equal(password.type, "password");
  assert.equal(otp.value, "123456");
  assert.equal(otp.autocomplete, "one-time-code");
});

test("password visibility is explicit and does not replace the controlled value", async () => {
  const user = setupUser();
  const view = render(<Harness />);
  const password = view.getByLabelText("CDSE 密码") as HTMLInputElement;
  fireEvent.change(password, { target: { value: "fixture-only" } });
  await user.click(view.getByRole("button", { name: "显示 CDSE 密码" }));
  assert.equal(password.type, "text");
  assert.equal(password.value, "fixture-only");
  await user.click(view.getByRole("button", { name: "隐藏 CDSE 密码" }));
  assert.equal(password.type, "password");
});

test("busy account forms disable fields and visibility controls", () => {
  const view = render(<Harness disabled />);
  for (const label of ["CDSE 用户名", "CDSE 密码", "CDSE 一次性验证码"]) {
    assert.equal((view.getByLabelText(label) as HTMLInputElement).disabled, true);
  }
  assert.equal((view.getByRole("button", { name: "显示 CDSE 密码" }) as HTMLButtonElement).disabled, true);
});

test("shared account fields support token-style secrets without changing draft semantics", async () => {
  const user = setupUser();
  function UsgsHarness() {
    const [username, setUsername] = useState("");
    const [token, setToken] = useState("");
    return <ProviderAccountFields
      provider="USGS"
      username={username}
      password={token}
      onUsernameChange={setUsername}
      onPasswordChange={setToken}
      passwordLabel="Application Token"
      passwordCredentialLabel="USGS M2M Application Token"
      passwordAutoComplete="off"
    />;
  }
  const view = render(<UsgsHarness />);
  const token = view.getByLabelText("USGS M2M Application Token") as HTMLInputElement;
  fireEvent.change(token, { target: { value: "fixture-token" } });
  assert.equal(token.value, "fixture-token");
  assert.equal(token.type, "password");
  assert.equal(token.autocomplete, "off");
  await user.click(view.getByRole("button", { name: "显示 USGS M2M Application Token" }));
  assert.equal(token.type, "text");
  assert.equal(token.value, "fixture-token");
});
