import "../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import { createElement, useEffect } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { I18nProvider, LOCALE_STORAGE_KEY, t, useI18n } from "./index.ts";
import { MESSAGES } from "./locales.ts";

test("global translation initializes from storage and updates synchronously with the provider setter", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  let setLocale!: ReturnType<typeof useI18n>["setLocale"];
  function CaptureLocale() {
    setLocale = useI18n().setLocale;
    useEffect(() => {
      assert.equal(t("shell.titlebar.defaultTitle"), MESSAGES.ja.shell.titlebar.defaultTitle);
    }, []);
    return null;
  }
  try {
    render(createElement(I18nProvider, null, createElement(CaptureLocale)));
    assert.equal(t("shell.titlebar.defaultTitle"), MESSAGES.ja.shell.titlebar.defaultTitle);
    act(() => setLocale("en"));
    assert.equal(t("shell.titlebar.defaultTitle"), MESSAGES.en.shell.titlebar.defaultTitle);
    act(() => setLocale("zh-CN"));
    assert.equal(t("shell.titlebar.defaultTitle"), MESSAGES["zh-CN"].shell.titlebar.defaultTitle);
  } finally {
    cleanup();
    window.localStorage.removeItem(LOCALE_STORAGE_KEY);
    act(() => setLocale("zh-CN"));
  }
});
