import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { createWorkbenchTestDesktopShellBridge } from "../../../test-utils/workbenchDesktopBridge";
import { DiagnosticsSettingsSection } from "./DiagnosticsSettingsSection";

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

test("DiagnosticsSettingsSection opt-in switch, JSON inspector, upload, and clear", async () => {
  let enabled = false;
  let cleared = false;
  let uploadedCalls = 0;
  let records = [
    {
      report_id: "diag-001",
      created_at: "2026-11-22T09:00:00Z",
      app_version: "2.2.0-beta.0",
      os: "macos",
      arch: "aarch64",
      category: "frontend_error",
      error_code: "UI_FETCH_ERR",
      source: "App.tsx",
      sanitized_message: "Failed connecting at <HOME>/work with <redacted_key>",
      sanitized_stack: "at load (<HOME>/ui/App.tsx:10)",
      uploaded: false,
    },
  ];
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      diagnosticsGetState: async () => ({
        enabled,
        total_count: records.length,
        pending_count: records.filter((r) => !r.uploaded).length,
      }),
      diagnosticsSetEnabled: async (next: boolean) => {
        enabled = next;
        return {
          enabled,
          total_count: records.length,
          pending_count: records.filter((r) => !r.uploaded).length,
        };
      },
      diagnosticsList: async () => records,
      diagnosticsUpload: async () => {
        uploadedCalls += 1;
        records = records.map((r) => ({ ...r, uploaded: true }));
        return {
          attempted: 1,
          uploaded: 1,
          failed: 0,
          message_zh: "已成功上报 1 条脱敏诊断记录。",
        };
      },
      diagnosticsClear: async () => {
        cleared = true;
        records = [];
        return { enabled, total_count: 0, pending_count: 0 };
      },
    },
  });

  const view = render(<DiagnosticsSettingsSection />);
  await waitFor(() =>
    assert.match(
      view.getByTestId("diagnostics-local-count").textContent || "",
      /本机缓存 1 条（待上报 1 条/
    )
  );

  // Starts disabled by default
  const sw = view.getByTestId("diagnostics-upload-switch");
  assert.equal(sw.getAttribute("aria-checked"), "false");

  // Toggle ON -> persists
  fireEvent.click(sw);
  await waitFor(() => assert.equal(sw.getAttribute("aria-checked"), "true"));
  assert.equal(enabled, true);

  // Open viewer modal -> shows exact sanitized JSON, and Escape closes only the nested modal
  let outerEscapeCalled = false;
  const outerListener = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !e.defaultPrevented) {
      outerEscapeCalled = true;
    }
  };
  window.addEventListener("keydown", outerListener);
  try {
    fireEvent.click(view.getByTestId("diagnostics-view-records-btn"));
    const modal = await waitFor(() => view.getByTestId("diagnostics-viewer-modal"));
    assert.match(modal.textContent || "", /UI_FETCH_ERR/);
    assert.match(modal.textContent || "", /<redacted_key>/);

    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => assert.equal(view.queryByTestId("diagnostics-viewer-modal"), null));
    assert.equal(outerEscapeCalled, false);

    // Reopen and close via close button as well
    fireEvent.click(view.getByTestId("diagnostics-view-records-btn"));
    await waitFor(() => view.getByTestId("diagnostics-viewer-modal"));
    fireEvent.click(view.getByRole("button", { name: "关闭诊断记录窗口" }));
    await waitFor(() => assert.equal(view.queryByTestId("diagnostics-viewer-modal"), null));
  } finally {
    window.removeEventListener("keydown", outerListener);
  }

  // Explicit upload marks uploaded and resets pending count
  fireEvent.click(view.getByTestId("diagnostics-upload-now-btn"));
  await waitFor(() =>
    assert.match(
      view.getByTestId("diagnostics-local-count").textContent || "",
      /本机缓存 1 条（待上报 0 条/
    )
  );
  assert.equal(uploadedCalls, 1);

  // Clear local records
  fireEvent.click(view.getByTestId("diagnostics-clear-btn"));
  await waitFor(() =>
    assert.match(
      view.getByTestId("diagnostics-local-count").textContent || "",
      /本机缓存 0 条（待上报 0 条/
    )
  );
  assert.equal(cleared, true);
});
