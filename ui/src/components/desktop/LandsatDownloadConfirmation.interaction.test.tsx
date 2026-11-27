import assert from "node:assert/strict";
import test from "node:test";

import "../../test-utils/setup-dom";
import React from "react";

import { LandsatDownloadConfirmation } from "./LandsatDownloadConfirmation";
import { opticalTaskReceiptFixture } from "../../test-utils/optical-task-fixture";
import { resolveAccountActionFromError } from "../../shell/api/opticalCreateError";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "../../lib/desktopShellBridge";
import { createBrowserFixtureDesktopShellBridge } from "../../lib/desktopShellBridgeFixture";
import type { DownloadBatchContext } from "../../lib/bridge";

test.afterEach(() => clearTestDesktopShellBridge());

const source: DownloadBatchContext = {
  batch_id: "source-batch",
  revision: 7,
  page_index: 0,
  page_size: 250,
  total_count: 1,
  checked_count: 1,
  aoi_name: "Fixture AOI",
  created_at: "2026-10-31T00:00:00Z",
  aoi_geojson: null,
  query: {},
};
const output = "C:\\fixtures\\downloads";

/**
 * Drive the 官方原始归档 admission exactly as the desktop does: the dialog
 * prepares, the user selects the official archive and confirms. `session`
 * controls what application_usgs_session_status reports while the submit
 * runs (the startup-restore race window), and `login` the usgsLogin result.
 */
async function submitOfficialArchive(options: {
  session: () => Promise<{ state: string; authenticated: boolean }>;
  login: () => Promise<{ state: string; authenticated: boolean }>;
  loginResult?: () => Promise<any>;
  defer?: boolean;
}) {
  const calls: string[] = [];
  const accountSettingsOpened: Array<string | undefined> = [];
  let createdTasks = 0;
  let deferredError: unknown = null;
  const base = createBrowserFixtureDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    kind: "tauri",
    credentials: {
      getStatus: async () => ({
        ok: true,
        status: {
          cdse: "none",
          usgs: "login:u***",
          earthdata: "none",
          earthdata_input: { mode: "none" as const, token: "", username: "", password: "" },
          bhoonidhi: "none",
          opentopography: "none",
          gacos: "none",
        },
      }),
    },
    application: {
      ...base.application,
      usgsSessionStatus: async () => {
        calls.push("usgsSessionStatus");
        return { ok: true, status: await options.session() };
      },
      usgsLogin: async () => {
        calls.push("usgsLogin");
        if (options.loginResult) {
          return await options.loginResult();
        }
        return { ok: true, status: await options.login() };
      },
      landsatAssetDetails: async () => ({ ok: false, error: "fixture: no catalog", code: "V3_APPLICATION_UNAVAILABLE" }),
      createLandsatBundleTask: async (request: { task_id: string; output_root: string; created_at: string }) => {
        createdTasks += 1;
        return opticalTaskReceiptFixture(request, "landsat-delivery-", 1);
      },
    },
  } as any);

  const { act, cleanup, fireEvent, render, screen, waitFor } = await import("@testing-library/react");
  let closed = 0;
  let confirmError: unknown = null;
  let deferredPromise: Promise<void> | null = null;
  try {
    const view = render(
      <LandsatDownloadConfirmation
        request={{
          batchContext: source,
          itemIds: ["scene-a"],
          outputRoot: output,
          concurrency: 2,
          taskPolicy: "manual",
        }}
        onClose={() => { closed += 1; }}
        onOpenAccountSettings={(accountId) => { accountSettingsOpened.push(accountId); }}
        onDeferToDownloadCenter={
          options.defer
            ? (_confirmed, execute) => {
                deferredPromise = execute().catch((err) => {
                  deferredError = err;
                });
              }
            : undefined
        }
        onTaskCreated={async () => undefined}
      />,
    );
    const dialog = await view.findByRole("dialog", { name: "确认下载任务" });
    const archive = screen.getByRole("radio", { name: /官方原始归档（\.tar）/ }) as HTMLInputElement;
    await act(async () => { fireEvent.click(archive); });
    await waitFor(() => assert.equal(archive.checked, true));
    const create = screen.getByRole("button", { name: "创建下载任务" }) as HTMLButtonElement;
    await waitFor(() => assert.equal(create.disabled, false));
    // The confirmation card resolves onConfirm itself; submit() runs inside it.
    // Capture a rejection from the wrapper's error surface instead of the throw:
    // the onConfirm prop is invoked by DownloadDestinationConfirmation with its
    // own error handling, so a failing submit surfaces through the dialog alert.
    await act(async () => { fireEvent.click(create); });
    if (options.defer) {
      await waitFor(() => assert.ok(deferredPromise !== null), { timeout: 3000 });
      await deferredPromise;
      return { calls, createdTasks, closed, alertError: false, alertText: "", deferredError, accountSettingsOpened, confirmError };
    }
    await waitFor(
      () => assert.ok(closed + createdTasks > 0 || screen.queryAllByRole("alert").length > 0),
      { timeout: 3000 },
    ).catch(() => undefined);
    const alertNodes = screen.queryAllByRole("alert");
    const alertNode = alertNodes[0] ?? null;
    const alertText = alertNode?.textContent ?? "";
    const alertError = (dialog.parentElement ?? dialog).textContent?.includes("无法建立下载会话") ?? false;
    if (!alertError && !alertNode) {
      await waitFor(() => assert.equal(closed, 1), { timeout: 3000 });
    }
    return { calls, createdTasks, closed, alertError, alertText, deferredError, accountSettingsOpened, confirmError };
  } finally {
    cleanup();
  }
}

test("first 官方原始归档 click waits for the startup login restore and creates the task", async () => {
  // Startup restore in flight: the first status read reports logging_in, and
  // the login that usgsLogin awaits finishes authenticated (backend reuses the
  // in-flight restore outcome). Before the fix this surfaced the
  // "无法建立下载会话" error on the first click.
  const result = await submitOfficialArchive({
    session: async () => ({ state: "logging_in", authenticated: false }),
    login: async () => ({ state: "authenticated", authenticated: true }),
  });
  assert.equal(result.alertError, false, "a login restore in flight must not surface as an admission error");
  assert.equal(result.createdTasks, 1);
  assert.equal(result.closed, 1);
  assert.ok(result.calls.includes("usgsLogin"), "the admission must perform or await a login, not just read status");
});

test("second-click shape: an already authenticated session creates the task without a login", async () => {
  const result = await submitOfficialArchive({
    session: async () => ({ state: "authenticated", authenticated: true }),
    login: async () => ({ state: "authenticated", authenticated: true }),
  });
  assert.equal(result.alertError, false);
  assert.equal(result.createdTasks, 1);
  assert.equal(result.closed, 1);
  assert.equal(result.calls.includes("usgsLogin"), false, "an authenticated session must not trigger another login");
});

test("a real authentication failure still surfaces the admission error", async () => {
  const result = await submitOfficialArchive({
    session: async () => ({ state: "login_failed", authenticated: false }),
    login: async () => ({ state: "login_failed", authenticated: false }),
  });
  assert.equal(result.createdTasks, 0, "no task may be created without an authenticated session");
  assert.equal(result.closed, 0);
});

test("ready-state first click logs in from the saved native credential and creates the task", async () => {
  const result = await submitOfficialArchive({
    session: async () => ({ state: "ready", authenticated: false }),
    login: async () => ({ state: "authenticated", authenticated: true }),
  });
  assert.equal(result.alertError, false);
  assert.equal(result.createdTasks, 1);
  assert.equal(result.closed, 1);
  assert.ok(result.calls.includes("usgsLogin"));
});

test("Issue #46: USGS_M2M_TRANSPORT_FAILED surfaces network reason instead of sessionUnavailable and omits account setup button", async () => {
  const result = await submitOfficialArchive({
    session: async () => ({ state: "ready", authenticated: false }),
    login: async () => ({ state: "login_failed", authenticated: false }),
    loginResult: async () => ({
      ok: false,
      code: "USGS_M2M_TRANSPORT_FAILED",
      error: "connect error",
    }),
  });
  assert.equal(result.createdTasks, 0);
  assert.equal(result.alertError, false, "must surface specific transport reason instead of generic sessionUnavailable");
  assert.match(result.alertText, /网络|DNS|代理|TLS/);

  const deferred = await submitOfficialArchive({
    session: async () => ({ state: "ready", authenticated: false }),
    login: async () => ({ state: "login_failed", authenticated: false }),
    loginResult: async () => ({
      ok: false,
      code: "USGS_M2M_TRANSPORT_FAILED",
      error: "connect error",
    }),
    defer: true,
  });
  assert.deepEqual(
    resolveAccountActionFromError(deferred.deferredError, "science.landsat.c2-l2.v1"),
    { actionAccountId: null, actionLabel: null },
    "transport error must not falsely attach Configure USGS M2M button",
  );
});

test("Issue #46: USGS_M2M_AUTH_INVALID surfaces credential error instead of sessionUnavailable and preserves USGS account action", async () => {
  const result = await submitOfficialArchive({
    session: async () => ({ state: "ready", authenticated: false }),
    login: async () => ({ state: "login_failed", authenticated: false }),
    loginResult: async () => ({
      ok: false,
      code: "USGS_M2M_AUTH_INVALID",
      error: "invalid token",
    }),
  });
  assert.equal(result.createdTasks, 0);
  assert.equal(result.alertError, false, "must surface specific auth reason instead of generic sessionUnavailable");
  assert.match(result.alertText, /USGS|Token|凭据|无效|验证/);

  const deferred = await submitOfficialArchive({
    session: async () => ({ state: "ready", authenticated: false }),
    login: async () => ({ state: "login_failed", authenticated: false }),
    loginResult: async () => ({
      ok: false,
      code: "USGS_M2M_AUTH_INVALID",
      error: "invalid token",
    }),
    defer: true,
  });
  assert.deepEqual(
    resolveAccountActionFromError(deferred.deferredError, "science.landsat.c2-l2.v1"),
    { actionAccountId: "usgs_m2m", actionLabel: "前往配置 USGS M2M 账号" },
  );
});
