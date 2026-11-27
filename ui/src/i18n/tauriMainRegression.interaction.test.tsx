import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import React from "react";
import { cleanup, fireEvent, render } from "@/test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, useI18n } from "./index";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test.afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

test("tauriMain.tsx must wrap App with I18nProvider to ensure language switching works on desktop", () => {
  const tauriMainPath = path.resolve(__dirname, "../tauriMain.tsx");
  const content = fs.readFileSync(tauriMainPath, "utf-8");

  assert.ok(
    /import\s+\{[^}]*I18nProvider[^}]*\}\s+from\s+["']\.\/i18n["']/.test(content),
    "tauriMain.tsx must import I18nProvider from ./i18n",
  );

  assert.ok(
    /<I18nProvider>[\s\S]*?<App\s*\/>[\s\S]*?<\/I18nProvider>/.test(content),
    "tauriMain.tsx must wrap <App /> with <I18nProvider>",
  );
});

function ConsumerComponent() {
  const { locale, setLocale, t } = useI18n();
  return (
    <div>
      <span data-testid="current-locale">{locale}</span>
      <span data-testid="translated-title">{t("shell.titlebar.defaultTitle")}</span>
      <button type="button" onClick={() => setLocale("en")}>
        Switch to English
      </button>
      <button type="button" onClick={() => setLocale("ja")}>
        Switch to Japanese
      </button>
    </div>
  );
}

test("I18nProvider enables dynamic locale switching and syncs with localStorage", () => {
  const view = render(
    <I18nProvider>
      <ConsumerComponent />
    </I18nProvider>,
  );

  assert.equal(view.getByTestId("current-locale").textContent, "zh-CN");
  assert.equal(view.getByTestId("translated-title").textContent, "对话");

  fireEvent.click(view.getByText("Switch to English"));
  assert.equal(view.getByTestId("current-locale").textContent, "en");
  assert.equal(view.getByTestId("translated-title").textContent, "Chat");
  assert.equal(window.localStorage.getItem(LOCALE_STORAGE_KEY), "en");

  fireEvent.click(view.getByText("Switch to Japanese"));
  assert.equal(view.getByTestId("current-locale").textContent, "ja");
  assert.equal(view.getByTestId("translated-title").textContent, "チャット");
  assert.equal(window.localStorage.getItem(LOCALE_STORAGE_KEY), "ja");
});
