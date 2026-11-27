import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import React from "react";
import { waitFor, within } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider } from "../../i18n";
import {
  DOWNLOAD_TOAST_AUTO_DISMISS_MS,
  PROGRESS_TOAST_EXIT_MS,
  ProgressNotificationHost,
} from "../ui/ProgressNotification";
import { DownloadFilterBar } from "./DownloadFilterBar";
import { ModelChannelsSettingsTab } from "../../shell/components/settings/ModelChannelsSettingsTab";
import {
  StorageSettingsTab,
  type CacheSummaryResponse,
  type ClearCachesResponse,
} from "../../shell/components/settings/StorageSettingsTab";
import { clearAllTopToastsForTest } from "../ui/TopCenterToast";
import type { ApplicationTask } from "../../shell/api/types";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

test.afterEach(() => {
  cleanup();
  clearAllTopToastsForTest();
});

function makeTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: "task-toast-1",
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1 合肥监测",
    provider: "ASF",
    dataset: "Sentinel-1",
    status: "running",
    done_bytes: 120 * 1024 * 1024,
    total_bytes: 1024 * 1024 * 1024,
    created_at: new Date(Date.now() - 3000).toISOString(),
    updated_at: new Date().toISOString(),
    selected_count: 2,
    completed_count: 0,
    failed_count: 0,
    paused_count: 0,
    queued_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    ...overrides,
  };
}

function makeCacheSummary(overrides: Partial<CacheSummaryResponse> = {}): CacheSummaryResponse {
  return {
    ok: true,
    cache_dir: "/Users/demo/Library/Application Support/EOStage",
    sqlite_path: "/Users/demo/Library/Application Support/EOStage/tasks.sqlite3",
    sqlite_size_bytes: 14_200_000,
    sqlite_wal_bytes: 1_048_576,
    total_size_bytes: 38_500_000,
    total_cap_bytes: 268_435_456,
    categories: [
      {
        id: "search_cache",
        label_zh: "检索结果缓存",
        description_zh: "缓存近期多源遥感目录检索结果，避免重复分页查询。",
        location: "tasks.sqlite3:asf_search_cache",
        size_bytes: 18_500_000,
        cap_bytes: 134_217_728,
        entry_count: 14,
        max_entries: 64,
        ttl_seconds: 86_400,
        clearable: true,
      },
      {
        id: "admin_boundary_cache",
        label_zh: "行政区边界缓存",
        description_zh: "缓存已解析的行政区多边形边界，加速 AOI 定位。",
        location: "tasks.sqlite3:administrative_boundary_cache",
        size_bytes: 12_000_000,
        cap_bytes: 67_108_864,
        entry_count: 32,
        max_entries: 256,
        ttl_seconds: 2_592_000,
        clearable: true,
      },
      {
        id: "catalog_and_preview_cache",
        label_zh: "目录与缩略图缓存",
        description_zh: "缓存快视图缩略图与 STAC 目录临时元数据。",
        location: "cache/previews",
        size_bytes: 8_000_000,
        cap_bytes: 67_108_864,
        entry_count: 48,
        max_entries: 512,
        ttl_seconds: 604_800,
        clearable: true,
      },
    ],
    protected_notice_zh:
      "清理缓存仅删除可重建的检索、边界、目录与临时预览缓存，不会影响已下载影像、下载任务记录、工作区设置或账号凭据。",
    ...overrides,
  };
}

test("Item 3a: ProgressNotificationHost renders frosted glass, auto-dismisses after 2s, pauses on hover/focus, and re-appears on finish/new task", async () => {
  assert.equal(DOWNLOAD_TOAST_AUTO_DISMISS_MS, 2000);
  assert.equal(PROGRESS_TOAST_EXIT_MS, 260);

  const cssContent = fs.readFileSync(path.resolve(__dirname, "../../index.css"), "utf8");
  assert.ok(cssContent.includes("@keyframes spProgressNotifExit"));
  assert.ok(cssContent.includes("scale(0.97) translateY(-4px)"));
  assert.ok(cssContent.includes(".sp-progress-notif-exit"));
  assert.ok(cssContent.includes("prefers-reduced-motion: reduce"));

  const dismissedIds: string[] = [];
  const task1 = makeTask({ task_id: "t-glass-1", status: "running" });

  const view = render(
    <ProgressNotificationHost
      tasks={[task1]}
      pendingPreparations={[]}
      autoDismissMs={120}
      exitAnimationMs={60}
      onDismissTaskNotification={(id) => dismissedIds.push(id)}
    />
  );

  const card = view.getByTestId("progress-notification-card");
  assert.ok(card.className.includes("bg-sp-surface/80"));
  assert.ok(card.className.includes("dark:bg-sp-surface/75"));
  assert.ok(card.className.includes("backdrop-blur-md"));
  assert.ok(card.className.includes("border-sp-line/70"));
  assert.ok(card.className.includes("shadow-lg"));
  assert.ok(card.className.includes("sp-progress-notif-glass"));

  // Hover pauses auto-dismiss past the timeout window.
  fireEvent.mouseEnter(card);
  await sleep(180);
  assert.ok(view.getByTestId("progress-notification-card"));
  assert.equal(
    view.getByTestId("progress-notification-card").getAttribute("data-exiting"),
    "false"
  );

  // Mouse leave resumes timer, plays dissolve exit, then unmounts without marking read.
  fireEvent.mouseLeave(card);
  await waitFor(() => {
    assert.equal(view.queryByTestId("progress-notification-card") === null, true);
  });
  assert.deepEqual(dismissedIds, []);

  // Same task completes -> re-appears smoothly for another auto-dismiss window.
  view.rerender(
    <ProgressNotificationHost
      tasks={[{ ...task1, status: "completed", done_bytes: 1024 * 1024 * 1024 }]}
      pendingPreparations={[]}
      autoDismissMs={140}
      exitAnimationMs={50}
      onDismissTaskNotification={(id) => dismissedIds.push(id)}
    />
  );
  const completedCard = view.getByTestId("progress-notification-card");
  assert.equal(completedCard.getAttribute("data-notification-status"), "completed");
  assert.ok(completedCard.textContent?.includes("已完成"));

  // Focus pauses the completed card; blur resumes and dismisses it.
  const closeBtn = within(completedCard).getByRole("button", { name: "关闭进度通知" });
  fireEvent.focus(closeBtn);
  await sleep(180);
  assert.ok(view.getByTestId("progress-notification-card"));

  fireEvent.blur(closeBtn);
  await waitFor(() => {
    assert.equal(view.queryByTestId("progress-notification-card") === null, true);
  });

  // New task starting triggers a fresh toast appearance, then failure also re-appears after auto-dismiss.
  const task2 = makeTask({ task_id: "t-glass-2", name: "Landsat C2 L2 新任务", status: "running" });
  view.rerender(
    <ProgressNotificationHost
      tasks={[{ ...task1, status: "completed" }, task2]}
      pendingPreparations={[]}
      autoDismissMs={100}
      exitAnimationMs={40}
      onDismissTaskNotification={(id) => dismissedIds.push(id)}
    />
  );
  const secondCard = view.getByTestId("progress-notification-card");
  assert.ok(secondCard.textContent?.includes("Landsat C2 L2 新任务"));
  await waitFor(() => {
    assert.equal(view.queryByTestId("progress-notification-card") === null, true);
  });

  // Auto-dismissed running task2 fails -> briefly re-appears with failed status.
  view.rerender(
    <ProgressNotificationHost
      tasks={[
        { ...task1, status: "completed" },
        {
          ...task2,
          status: "failed",
          error: { code: "DL005", message_zh: "上游服务返回 HTTP 503，已停止重试" } as any,
        },
      ]}
      pendingPreparations={[]}
      autoDismissMs={140}
      exitAnimationMs={50}
      onDismissTaskNotification={(id) => dismissedIds.push(id)}
    />
  );
  const failedCard = view.getByTestId("progress-notification-card");
  assert.equal(failedCard.getAttribute("data-notification-status"), "failed");
  assert.ok(failedCard.textContent?.includes("HTTP 503"));

  // Accessible close button plays smooth dissolve exit then marks dismissed.
  fireEvent.click(within(failedCard).getByRole("button", { name: "关闭进度通知" }));
  assert.equal(failedCard.getAttribute("data-exiting"), "true");
  assert.ok(failedCard.className.includes("sp-progress-notif-exit"));
  await waitFor(() => {
    assert.equal(view.queryByTestId("progress-notification-card") === null, true);
  });
  assert.deepEqual(dismissedIds, ["t-glass-2"]);
});

test("Item 3b: DownloadFilterBar applies 12px wrapped track radius instead of rounded-full bulges", () => {
  const cssContent = fs.readFileSync(path.resolve(__dirname, "../../index.css"), "utf8");
  assert.ok(cssContent.includes(".eo-segmented-track.eo-segmented-track-wrap"));
  assert.ok(cssContent.includes("border-radius: 12px;"));
  assert.ok(cssContent.includes("border-radius: 9px;"));

  const tasks: ApplicationTask[] = [
    makeTask({ task_id: "f1", dataset: "NISAR L", status: "running" }),
    makeTask({ task_id: "f2", dataset: "GACOS", status: "paused" }),
    makeTask({ task_id: "f3", dataset: "NISAR S", status: "completed" }),
    makeTask({ task_id: "f4", dataset: "Sentinel-1", status: "failed" }),
    makeTask({ task_id: "f5", dataset: "optical.landsat.c2.l2", status: "cancelled" }),
  ];

  const view = render(
    <I18nProvider>
      <DownloadFilterBar
        tasks={tasks}
        selectedType="all"
        onSelectType={() => {}}
        selectedStatus="all"
        onSelectStatus={() => {}}
      />
    </I18nProvider>
  );

  const statusTrack = view.getByTestId("download-status-filter-bar");
  const datasetTrack = view.getByTestId("download-dataset-filter-bar");

  for (const track of [statusTrack, datasetTrack]) {
    assert.ok(track.className.includes("eo-segmented-track-wrap"));
    assert.ok(track.className.includes("rounded-[12px]"));
    assert.equal(track.className.includes("rounded-full"), false);
  }
});

test("Item 6: ModelChannelsSettingsTab turns Local CLI into 开发中 placeholder and prunes unsupported OAuth providers", () => {
  const view = render(
    <I18nProvider>
      <ModelChannelsSettingsTab />
    </I18nProvider>
  );

  fireEvent.click(view.getByTestId("toggle-oauth-pkce-helper"));

  // Third-party local CLI credential scanner is removed from public release.
  assert.equal(view.queryByTestId("local-cli-import-panel"), null);
  assert.equal(view.queryByTestId("refresh-local-cli-btn"), null);

  // Verified official OAuth PKCE / Device Flow providers remain (OpenAI Codex, Claude Pro/Max, GitHub Copilot, OpenRouter).
  const pkceButtons = view.getAllByTestId(/^pkce-provider-/);
  assert.deepEqual(
    pkceButtons.map((el) => el.getAttribute("data-testid")),
    [
      "pkce-provider-openai_codex_pkce",
      "pkce-provider-claude_oauth_pkce",
      "pkce-provider-openrouter_pkce",
      "pkce-provider-github_copilot_device",
    ]
  );
  assert.equal(view.queryByTestId("pkce-provider-google_gemini_pkce"), null);
  assert.equal(view.queryByTestId("pkce-provider-kimi_code_device"), null);

  // OpenAI Codex PKCE generates valid S256 auth URL.
  fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
  assert.ok(
    view.getByTestId("pkce-authorize-url-display").textContent?.includes("auth.openai.com/oauth/authorize")
  );

  // Claude Pro / Max PKCE generates valid S256 auth URL.
  fireEvent.click(view.getByTestId("pkce-provider-claude_oauth_pkce"));
  fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
  assert.ok(
    view.getByTestId("pkce-authorize-url-display").textContent?.includes("claude.ai/oauth/authorize")
  );

  // OpenRouter PKCE generates valid S256 auth URL.
  fireEvent.click(view.getByTestId("pkce-provider-openrouter_pkce"));
  fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
  assert.ok(
    view.getByTestId("pkce-authorize-url-display").textContent?.includes("openrouter.ai/auth")
  );
});

test("Item 2: StorageSettingsTab displays cache directory, breakdown vs cap, confirmation, and safe clear feedback", async () => {
  const initialSummary = makeCacheSummary();
  const emptySummary = makeCacheSummary({
    sqlite_size_bytes: 2_100_000,
    sqlite_wal_bytes: 0,
    total_size_bytes: 0,
    categories: initialSummary.categories.map((cat) => ({
      ...cat,
      size_bytes: 0,
      entry_count: 0,
    })),
  });

  let summaryCalls = 0;
  let clearCalls = 0;

  const view = render(
    <I18nProvider>
      <StorageSettingsTab
        storageRootPath="/Users/demo/EOStageData"
        onGetCacheSummary={async () => {
          summaryCalls += 1;
          return initialSummary;
        }}
        onClearDisposableCaches={async () => {
          clearCalls += 1;
          const res: ClearCachesResponse = {
            ok: true,
            cleared_categories: [
              "search_cache",
              "admin_boundary_cache",
              "catalog_and_preview_cache",
            ],
            reclaimed_bytes: 38_500_000,
            removed_entries: 94,
            vacuumed: true,
            summary: emptySummary,
          };
          return res;
        }}
      />
    </I18nProvider>
  );

  await waitFor(() => {
    assert.ok(view.getByTestId("cache-dir-path").textContent?.includes("Application Support/EOStage"));
  });
  assert.equal(summaryCalls, 1);
  assert.ok(view.getByTestId("cache-total-usage").textContent?.includes("38.5 MB"));
  assert.ok(view.getByTestId("cache-total-usage").textContent?.includes("268.4 MB"));
  assert.ok(view.getByTestId("cache-category-search_cache").textContent?.includes("检索结果缓存"));
  assert.ok(view.getByTestId("cache-category-admin_boundary_cache").textContent?.includes("行政区边界缓存"));
  assert.ok(
    view.getByTestId("cache-category-catalog_and_preview_cache").textContent?.includes("目录与缩略图缓存")
  );
  assert.ok(
    view.getByTestId("cache-protected-notice").textContent?.includes("不会影响已下载影像")
  );

  // Clicking 清理缓存 opens confirmation panel; clicking 取消 does NOT call backend clear.
  fireEvent.click(view.getByTestId("cache-clear-btn"));
  assert.ok(view.getByTestId("cache-clear-confirm-panel"));
  fireEvent.click(view.getByTestId("cache-clear-cancel-btn"));
  assert.equal(view.queryByTestId("cache-clear-confirm-panel"), null);
  assert.equal(clearCalls, 0);

  // Clicking 清理缓存 -> 确认清理 executes clear, updates summary to 0 B, and shows clear feedback.
  fireEvent.click(view.getByTestId("cache-clear-btn"));
  fireEvent.click(view.getByTestId("cache-clear-confirm-btn"));

  await waitFor(() => {
    assert.equal(clearCalls, 1);
    const feedback = view.getByTestId("cache-clear-feedback");
    assert.ok(feedback.textContent?.includes("38.5 MB"));
    assert.ok(feedback.textContent?.includes("94 条"));
    assert.ok(feedback.textContent?.includes("已下载影像、任务与账号均未受影响"));
  });
  assert.ok(view.getByTestId("cache-total-usage").textContent?.includes("0 B"));
});

test("Item 2b: StorageSettingsTab invokes native get_cache_summary / clear_disposable_caches and surfaces errors cleanly", async () => {
  // Without Tauri internals or overrides, browser preview stays honest (no fake numbers, clear disabled).
  const browserView = render(
    <I18nProvider>
      <StorageSettingsTab storageRootPath="/Users/demo/EOStageData" />
    </I18nProvider>
  );
  assert.equal(
    (browserView.getByTestId("cache-clear-btn") as HTMLButtonElement).disabled,
    true
  );
  assert.equal(browserView.queryByTestId("cache-total-usage"), null);
  cleanup();

  // Error on first summary load shows alert banner and retry recovers cleanly.
  let attempts = 0;
  const retryView = render(
    <I18nProvider>
      <StorageSettingsTab
        storageRootPath="/Users/demo/EOStageData"
        onGetCacheSummary={async () => {
          attempts += 1;
          if (attempts === 1) throw new Error("无法访问缓存数据库");
          return makeCacheSummary();
        }}
      />
    </I18nProvider>
  );
  await waitFor(() => {
    assert.ok(retryView.getByTestId("cache-error-banner").textContent?.includes("无法访问缓存数据库"));
  });
  fireEvent.click(within(retryView.getByTestId("cache-error-banner")).getByRole("button", { name: "重试" }));
  await waitFor(() => {
    assert.ok(retryView.getByTestId("cache-total-usage").textContent?.includes("38.5 MB"));
  });
  cleanup();

  // Native Tauri IPC wiring sends exact backend command names.
  const invoked: Array<{ cmd: string; args: unknown }> = [];
  const prevInternals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  try {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, args?: unknown) => {
        invoked.push({ cmd, args });
        if (cmd === "get_cache_summary") {
          return makeCacheSummary();
        }
        if (cmd === "clear_disposable_caches") {
          return {
            ok: true,
            cleared_categories: ["search_cache"],
            reclaimed_bytes: 18_500_000,
            removed_entries: 14,
            vacuumed: true,
            summary: makeCacheSummary({ total_size_bytes: 0 }),
          } satisfies ClearCachesResponse;
        }
        return null;
      },
    };
    const nativeView = render(
      <I18nProvider>
        <StorageSettingsTab storageRootPath="/Users/demo/EOStageData" />
      </I18nProvider>
    );
    await waitFor(() => {
      assert.ok(nativeView.getByTestId("cache-dir-path"));
    });
    fireEvent.click(nativeView.getByTestId("cache-clear-btn"));
    fireEvent.click(nativeView.getByTestId("cache-clear-confirm-btn"));
    await waitFor(() => {
      assert.ok(nativeView.getByTestId("cache-clear-feedback"));
    });
    assert.ok(invoked.some((entry) => entry.cmd === "get_cache_summary"));
    assert.ok(
      invoked.some(
        (entry) =>
          entry.cmd === "clear_disposable_caches" &&
          typeof entry.args === "object" &&
          entry.args !== null &&
          "categories" in entry.args
      )
    );
  } finally {
    if (prevInternals === undefined) {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    } else {
      (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = prevInternals;
    }
  }
});
