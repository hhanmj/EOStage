import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "@/test-utils/render-harness";
import { createAccountSettingsFixture } from "@/test-utils/account-settings-fixture";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "@/lib/desktopShellBridge";
import { I18nProvider, LOCALE_STORAGE_KEY, type Locale } from "@/i18n";
import { BhoonidhiCredentialPanel } from "./BhoonidhiCredentialPanel";
import { UsgsCredentialSessionPanel } from "./ProviderSessionCredentialPanel";
import { DesktopCdseAccountPanel } from "./desktop/DesktopAccountPanels";
import { InplaceLoginCard } from "@/shell/components/InplaceLoginCard";
import type { TypedError } from "@/shell/api/types";

const success = async () => ({ ok: true as const });
const noop = () => {};
const authError: TypedError = {
  code: "AUTH_REQUIRED",
  message_zh: "后端原文",
  retryable: false,
  needs_account: "cdse",
  needs_user_confirmation: false,
};

function renderIn(locale: Locale, node: React.ReactNode) {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  return render(<I18nProvider>{node}</I18nProvider>);
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

const cases = [
  {
    locale: "en" as const,
    username: "CDSE Username",
    password: "CDSE Password",
    otp: "CDSE One-time code (optional)",
    login: "Log in",
    clear: "Clear CDSE credentials",
    valid: "Account valid",
    bhoonidhi: "Bhoonidhi credential settings",
    savedNotLoggedIn: "Saved · Not logged in",
    localCredentials: "Local credentials",
    saved: "Saved",
    checkLogin: "Check login",
    usgsUsername: "USGS Username",
    usgsLogin: "Log in",
    credentialsSaved: "Credentials saved",
    loginCard: "Data center account authorization",
    later: "Later",
  },
  {
    locale: "ja" as const,
    username: "CDSE ユーザー名",
    password: "CDSE パスワード",
    otp: "CDSE ワンタイムコード（任意）",
    login: "ログイン",
    clear: "CDSE の認証情報をクリア",
    valid: "アカウントは有効です",
    bhoonidhi: "Bhoonidhi 認証情報設定",
    savedNotLoggedIn: "保存済み・未ログイン",
    localCredentials: "ローカル認証情報",
    saved: "保存済み",
    checkLogin: "ログインを確認",
    usgsUsername: "USGS ユーザー名",
    usgsLogin: "ログイン",
    credentialsSaved: "認証情報を保存済み",
    loginCard: "データセンターアカウントの認可",
    later: "後で",
  },
];

for (const expected of cases) {
  test(`${expected.locale}: CDSE account panel labels, login button and verified status`, async () => {
    const fixture = createAccountSettingsFixture();
    registerTestDesktopShellBridge(fixture.bridge);
    const view = renderIn(expected.locale, <DesktopCdseAccountPanel />);
    assert.ok(view.getByLabelText(expected.username));
    assert.ok(view.getByLabelText(expected.password));
    assert.ok(view.getByLabelText(expected.otp));
    assert.ok(view.getByRole("button", { name: expected.clear }));
    const login = await waitFor(() => {
      const button = view.getByRole("button", { name: expected.login }) as HTMLButtonElement;
      assert.equal(button.disabled, false);
      return button;
    });
    fireEvent.click(login);
    await view.findByText(expected.valid);
    assert.deepEqual(fixture.state.checks, ["cdse"]);
    assert.equal(document.documentElement.lang, expected.locale);
  });

  test(`${expected.locale}: Bhoonidhi panel title and saved-state status`, () => {
    const view = renderIn(expected.locale, (
      <BhoonidhiCredentialPanel status="login:f***" onSave={success} onClear={success} onChanged={success}
        onCheckAuth={async () => ({ ok: true, session_available: true, catalogue_verified: false })} />
    ));
    assert.ok(view.getByRole("form", { name: expected.bhoonidhi }));
    assert.ok(view.getByText(expected.savedNotLoggedIn));
    assert.ok(view.getByText(expected.localCredentials));
    assert.ok(view.getByText(expected.saved));
    assert.ok(view.getByRole("button", { name: expected.checkLogin }));
  });

  test(`${expected.locale}: USGS session panel keeps Application Token untranslated`, () => {
    const view = renderIn(expected.locale, (
      <UsgsCredentialSessionPanel session={null} username="" applicationToken="" saved credentialIdentity="usgs"
        busy={null} error={null} note={null} onUsernameChange={noop} onApplicationTokenChange={noop}
        onSave={noop} onLogin={noop} onRefresh={noop} onClear={noop} onLogout={noop} />
    ));
    assert.ok(view.getByLabelText(expected.usgsUsername));
    assert.ok(view.getByLabelText("USGS M2M Application Token"));
    assert.ok(view.getByText("Application Token"));
    assert.ok(view.getByRole("button", { name: expected.usgsLogin }));
    assert.ok(view.getByText(expected.credentialsSaved));
  });

  test(`${expected.locale}: in-place login card title and actions`, () => {
    const view = renderIn(expected.locale, (
      <InplaceLoginCard error={authError}
        onClose={noop} />
    ));
    assert.ok(view.getByRole("dialog", { name: expected.loginCard }));
    assert.ok(view.getByText("Copernicus Data Space (CDSE)"));
    assert.ok(view.getByText("后端原文"));
    assert.ok(view.getByRole("button", { name: expected.later }));
  });
}
