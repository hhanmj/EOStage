import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, useI18n, type Locale } from "../../i18n";
import { MESSAGES } from "../../i18n/locales";
import { formatBridgeErrorValue } from "../../lib/bridgeErrorFormat";
import { createTauriDesktopShellBridge } from "../../lib/desktopShellBridgeTauri";
import { projectErrorCard } from "../../shell/api/approvalLogic";
import { explainErrorCode, formatErrorWithCode, preferCodeExplanation } from "../../shell/api/errorExplanations";
import type { ApplicationTask } from "../../shell/api/types";
import { TaskCard } from "./DesktopDownloadDrawer";

const en = MESSAGES.en.errors;
const ja = MESSAGES.ja.errors;
let setLocale: ((locale: Locale) => void) | null = null;

function CaptureLocale() {
  setLocale = useI18n().setLocale;
  return null;
}

function mount(locale: Locale, children: React.ReactNode = null) {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  return render(<I18nProvider><CaptureLocale />{children}</I18nProvider>);
}

test.afterEach(() => {
  if (setLocale) act(() => setLocale?.("zh-CN"));
  setLocale = null;
  cleanup();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

const pausedTask: ApplicationTask = {
  task_id: "task-disk-1",
  revision: 3,
  category: "scene_download",
  name: "Sentinel-1 SLC",
  provider: "ASF",
  dataset: "Sentinel-1",
  status: "paused",
  done_bytes: 1_000_000_000,
  total_bytes: 4_000_000_000,
  output_dir: "D:/InSAR_Data/s1",
  updated_at: "2026-10-26T08:05:00Z",
  pause_reason: null,
};

test("an English task card explains a rejected resume by its error code", async () => {
  const view = mount("en", (
    <TaskCard
      task={pausedTask}
      onResume={async () => { throw { code: "DISK_LOW", message_key: "disk_low", context: {} }; }}
    />
  ));
  fireEvent.click(view.getByRole("button", { name: MESSAGES.en.downloads.task.resumeTask }));
  await waitFor(() => assert.ok(view.getByText(en.explanations.DISK_LOW)));
  assert.equal(view.queryByText(/DISK_LOW/), null);
  assert.equal(view.queryByText(MESSAGES["zh-CN"].errors.explanations.DISK_LOW), null);
});

test("en and ja prefer the code-keyed explanation over the backend message_zh", () => {
  mount("en");
  assert.equal(explainErrorCode("step_limit"), en.explanations.step_limit);
  assert.equal(formatBridgeErrorValue({ code: "ASF_TIMEOUT", error: "后端返回的中文说明" }), en.bridge.ASF_TIMEOUT);
  assert.equal(formatBridgeErrorValue({ code: "ASF_QUERY_INVALID" }, { dataset: "SLC-BURST" }), en.bridgeText.asfQueryInvalidBurst);
  assert.equal(preferCodeExplanation("主机自己结束运行的原因", "step_limit"), en.explanations.step_limit);
  assert.equal(
    projectErrorCard({ code: "repeated_tool_call", status: null, message_zh: "主机原因" }).wording,
    en.explanations.repeated_tool_call,
  );
  assert.equal(
    formatBridgeErrorValue({ code: "BHOONIDHI_AUTH_UNAVAILABLE", context: "账号尚未激活。" }),
    en.bridge.BHOONIDHI_AUTH_UNAVAILABLE,
  );
  assert.equal(
    formatBridgeErrorValue({ code: "V3_APPLICATION_INVALID_REQUEST", context: "start date must precede end date" }),
    "The request parameters are invalid: start date must precede end date.",
  );
  assert.equal(
    formatBridgeErrorValue({ code: "BHOONIDHI_HTTP_ERROR", context: "http_status:413" }),
    `The Bhoonidhi catalogue request failed (HTTP 413). ${en.bridgeText.bhoonidhiHintTooLarge}`,
  );
  assert.match(
    formatBridgeErrorValue({ code: "DEM_RANGE_LIMIT_EXCEEDED", context: "width=40000;height=100;pixels=4000000;coarser=1" }),
    /^The selected range is about 40000 × 100 pixels \(about 0\.04 × 10⁸ pixels\).* or use the 90 m dataset;/,
  );
  act(() => setLocale?.("ja"));
  assert.equal(formatBridgeErrorValue({ code: "ASF_TIMEOUT", error: "后端返回的中文说明" }), ja.bridge.ASF_TIMEOUT);
  assert.equal(explainErrorCode("ORB002"), ja.explanations.ORB002);
});

test("an unknown code keeps its original text in en: message_zh, raw code or generic copy", () => {
  mount("en");
  assert.equal(explainErrorCode("UNKNOWN_CODE_X"), null);
  assert.equal(formatErrorWithCode("UNKNOWN_CODE_X"), "UNKNOWN_CODE_X");
  assert.equal(preferCodeExplanation("后端原因", "UNKNOWN_CODE_X"), "后端原因");
  assert.equal(formatBridgeErrorValue({ code: "UNKNOWN_CODE_X", error: "后端中文原文" }), "后端中文原文");
  assert.equal(formatBridgeErrorValue({ code: "UNKNOWN_CODE_X" }), en.bridgeText.generic);
  assert.equal(
    projectErrorCard({ code: "UNKNOWN_CODE_X", status: null, message_zh: "主机原因" }).wording,
    "主机原因",
  );
});

test("a Tauri ShellError message_key is shown through the current-locale errors namespace", async () => {
  mount("en");
  let messageKey = "application_service.unavailable";
  const bridge = createTauriDesktopShellBridge({
    invoke: async () => { throw { code: "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE", message_key: messageKey }; },
    getCurrentWindow: () => ({ minimize: async () => undefined, toggleMaximize: async () => undefined,
      close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }), startResizeDragging: async () => undefined }),
    open: async () => null,
  }, "tauri://localhost/", false);
  const result = await bridge.application.readNisarArchivedRecoveryContextV2("archived-source", "confirmed-successor");
  assert.deepEqual(result, {
    ok: false,
    error: en.messageKeys.application_service.unavailable,
    code: "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE",
  });
  // The generic service text never displaces the specific code explanation.
  assert.equal(formatBridgeErrorValue(result), en.bridge.NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE);

  messageKey = "unknown.message_key";
  const unknown = await bridge.application.readNisarArchivedRecoveryContextV2("archived-source", "confirmed-successor");
  assert.equal(unknown.ok ? null : unknown.error, "unknown.message_key");
});
