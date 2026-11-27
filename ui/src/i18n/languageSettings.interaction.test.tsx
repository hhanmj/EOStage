import "../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { fireEvent, render, cleanup } from "../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, detectSystemLocale, t } from "../i18n";
import { SettingsModal } from "../components/SettingsModal";

function renderSettings() {
  return render(
    <I18nProvider>
      <SettingsModal open={true} onClose={() => {}} initialCategory="appearance" />
    </I18nProvider>,
  );
}

test.afterEach(() => {
  cleanup();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

test("language changes immediately, persists after remount, and restores zh-CN", () => {
  document.documentElement.lang = "zh-CN";
  const first = renderSettings();
  const languageGroup = first.getByRole("group", { name: "语言 / Language" });
  assert.equal(languageGroup.textContent, "简体中文English日本語");
  assert.equal(first.getByRole("button", { name: "简体中文" }).getAttribute("aria-pressed"), "true");

  fireEvent.click(first.getByRole("button", { name: "English" }));
  assert.ok(first.getByRole("group", { name: "Language" }));
  assert.equal(document.documentElement.lang, "en");
  assert.equal(window.localStorage.getItem(LOCALE_STORAGE_KEY), "en");
  assert.equal(t("settings.language.label"), "Language");

  first.unmount();
  const second = renderSettings();
  assert.ok(second.getByRole("group", { name: "Language" }));
  assert.equal(second.getByRole("button", { name: "English" }).getAttribute("aria-pressed"), "true");

  fireEvent.click(second.getByRole("button", { name: "简体中文" }));
  assert.ok(second.getByRole("group", { name: "语言 / Language" }));
  assert.equal(document.documentElement.lang, "zh-CN");
  assert.equal(window.localStorage.getItem(LOCALE_STORAGE_KEY), "zh-CN");
  assert.equal(t("settings.language.label"), "语言 / Language");
});

test("Japanese updates the document language and keeps every language self-named", () => {
  const view = renderSettings();
  fireEvent.click(view.getByRole("button", { name: "日本語" }));
  assert.ok(view.getByRole("group", { name: "言語" }));
  assert.equal(document.documentElement.lang, "ja");
  assert.equal(view.getByRole("button", { name: "日本語" }).getAttribute("aria-pressed"), "true");
});

test("invalid and unavailable preference storage preserve the zh-CN default", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "fr");
  const invalid = renderSettings();
  assert.ok(invalid.getByRole("group", { name: "语言 / Language" }));
  invalid.unmount();

  const storagePrototype = Object.getPrototypeOf(window.localStorage) as Storage;
  const originalGetItem = Object.getOwnPropertyDescriptor(storagePrototype, "getItem");
  const originalSetItem = Object.getOwnPropertyDescriptor(storagePrototype, "setItem");
  Object.defineProperty(storagePrototype, "getItem", {
    configurable: true,
    value() { throw new Error("Storage unavailable"); },
  });
  Object.defineProperty(storagePrototype, "setItem", {
    configurable: true,
    value() { throw new Error("Storage unavailable"); },
  });
  try {
    const unavailable = renderSettings();
    assert.ok(unavailable.getByRole("group", { name: "语言 / Language" }));
    fireEvent.click(unavailable.getByRole("button", { name: "English" }));
    assert.ok(unavailable.getByRole("group", { name: "Language" }));
    assert.equal(document.documentElement.lang, "en");
  } finally {
    if (originalGetItem) Object.defineProperty(storagePrototype, "getItem", originalGetItem);
    if (originalSetItem) Object.defineProperty(storagePrototype, "setItem", originalSetItem);
  }
});

test("detectSystemLocale automatically maps OS language tags to zh-CN, en, or ja", () => {
  assert.equal(detectSystemLocale(["ja-JP", "en-US"]), "ja");
  assert.equal(detectSystemLocale(["en-GB"]), "en");
  assert.equal(detectSystemLocale(["zh-Hans-CN", "en-US"]), "zh-CN");
  assert.equal(detectSystemLocale(["fr-FR", "de-DE"]), "zh-CN");
});
