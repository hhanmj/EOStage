import "../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY } from "../i18n";
import { DesktopTitlebar } from "../components/desktop/DesktopTitlebar";
import { SettingsModal } from "../components/SettingsModal";
import { PluginsAndSkillsPage } from "../components/desktop/PluginsAndSkillsPage";
import { AboutEOStageModal } from "../components/desktop/AboutEOStageModal";
import { SidebarSessionTree } from "../components/desktop/SidebarSessionTree";
import { AdminBoundaryModal } from "../components/desktop/AdminBoundaryModal";
import { DesktopDownloadDrawer } from "../components/desktop/DesktopDownloadDrawer";
import { DesktopGuidedTour } from "../components/desktop/DesktopGuidedTour";
import { TaskArtifactsVerificationPanel } from "../components/desktop/TaskArtifactsVerificationPanel";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";

const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/DesktopAgentWorkbench.tsx")) {
      const source = `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
      return { format: "module", shortCircuit: true, source: transformSync(source, { loader: "tsx", format: "esm" }).code };
    }
    return nextLoad(url, context);
  },
});
const { DesktopActivityRail } = await import("../components/desktop/DesktopActivityRail");
hooks.deregister();

function renderShell() {
  return render(
    <I18nProvider>
      <DesktopTitlebar />
      <DesktopActivityRail activeRoute="workbench" onNavigate={() => {}} expanded />
      <SettingsModal open onClose={() => {}} initialCategory="appearance" />
    </I18nProvider>,
  );
}

test.afterEach(() => {
  cleanup();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

test("English switches representative titlebar, rail, and settings labels immediately", async () => {
  const view = renderShell();
  assert.ok(view.getByText("对话"));
  assert.ok(view.getByRole("button", { name: "地图工作台" }));
  assert.ok(view.getByRole("dialog", { name: "偏好设置" }));

  fireEvent.click(view.getByRole("button", { name: "English" }));
  assert.ok(view.getByText("Chat"));
  assert.ok(view.getByRole("button", { name: "Map workbench" }));
  assert.ok(view.getByRole("dialog", { name: "Preferences" }));
  assert.ok(view.getByRole("button", { name: "Provider accounts" }));
  assert.ok(view.getByText("Appearance & display"));
  assert.ok(view.getByText("Diagnostics & crash reporting"));
  const appearanceTextWithoutAutonyms = (view.getByTestId("appearance-merged-settings").textContent || "").replace(/简体中文|日本語/g, "");
  assert.equal(/[\u4e00-\u9fff]/.test(appearanceTextWithoutAutonyms), false, "Appearance settings in English mode must not contain hardcoded Chinese");
  assert.ok(view.getByTestId("titlebar-workspace-switcher").textContent?.includes("Smart Search"));
  fireEvent.click(view.getByTestId("settings-nav-usage_stats"));
  assert.ok(view.getByRole("button", { name: "Refresh statistics" }));
  await waitFor(() => {
    assert.ok(view.getByText("Total sessions"));
  });
  assert.equal(/[\u4e00-\u9fff]/.test(view.getByTestId("usage-stats-settings-tab").textContent || ""), false, "Usage stats settings in English mode must not contain hardcoded Chinese");
  assert.equal(document.documentElement.lang, "en");
});

test("Japanese switches representative titlebar, rail, and settings labels immediately", () => {
  const view = renderShell();
  fireEvent.click(view.getByRole("button", { name: "日本語" }));
  assert.ok(view.getByText("チャット"));
  assert.ok(view.getByRole("button", { name: "マップワークベンチ" }));
  assert.ok(view.getByRole("dialog", { name: "環境設定" }));
  assert.ok(view.getByRole("button", { name: "データサービスアカウント" }));
  assert.ok(view.getByText("異常診断とログ送信"));
  assert.ok(view.getByTestId("titlebar-workspace-switcher").textContent?.includes("スマート検索"));
  assert.equal(document.documentElement.lang, "ja");
});

test("PluginsAndSkillsPage renders Japanese & English without leaked CJK in English mode", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  const jaView = render(
    <I18nProvider>
      <PluginsAndSkillsPage />
    </I18nProvider>,
  );
  assert.ok(jaView.getByTestId("plugins-and-skills-page").textContent?.includes("スキル"));
  cleanup();

  window.localStorage.setItem(LOCALE_STORAGE_KEY, "en");
  const enView = render(
    <I18nProvider>
      <PluginsAndSkillsPage />
    </I18nProvider>,
  );
  const enRoot = enView.getByTestId("plugins-and-skills-page");
  assert.ok(enRoot.textContent?.includes("Skills"));
  assert.equal(/[\u4e00-\u9fff]/.test(enRoot.textContent || ""), false, "PluginsAndSkillsPage in English must not leak CJK");
});

test("Japanese mode localizes About modal, Sidebar tree, Admin boundary modal, Download drawer tabs, and Guided tour without Simplified Chinese leaks", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  const forbiddenJaLeaks = /生成报告|新建对话|检索区域|帮助与关于|快捷键清单|请先在设置|生アーカイブ|关闭弹窗|清空输入|检查更新|仅查看当前|等待准备|文件进度/;

  const mainView = render(
    <I18nProvider>
      <div className="sp-modern">
        <SidebarSessionTree
          activeRoute="agent"
          onOpenSession={() => {}}
          onNavigate={() => {}}
          fixtureSessions={() => []}
        />
        <DesktopDownloadDrawer open variant="page" harnessData={[]} />
        <DesktopGuidedTour open onFinish={() => {}} />
      </div>
    </I18nProvider>,
  );

  // Verify Download drawer tabs in Japanese
  assert.ok(mainView.getByRole("tab", { name: "タスク一覧" }));
  assert.ok(mainView.getByRole("tab", { name: "成果・検証" }));

  // Verify Guided tour skip button never wraps and Step 4 uses 公式原アーカイブ
  const skipBtn = mainView.getByTestId("guided-tour-skip-btn");
  assert.ok(skipBtn.className.includes("whitespace-nowrap"), "Skip button must stay single-line");
  assert.ok(skipBtn.className.includes("shrink-0"), "Skip button must not shrink");

  const nextBtn = () => mainView.getByTestId("guided-tour-next-btn");
  for (let step = 1; step <= 7; step += 1) {
    const bodyText = document.body.textContent || "";
    assert.equal(
      forbiddenJaLeaks.test(bodyText),
      false,
      `Japanese shell/tour step ${step} leaked Simplified Chinese`,
    );
    if (step === 4) {
      assert.ok(bodyText.includes("公式原アーカイブ"), "Tour step 4 must use 公式原アーカイブ");
    }
    if (step < 7) {
      fireEvent.click(nextBtn());
    }
  }
  cleanup();

  // Verify AboutEOStageModal & AdminBoundaryModal in Japanese separately so ModalOverlay aria-hidden never collides
  render(
    <I18nProvider>
      <AboutEOStageModal open onClose={() => {}} />
    </I18nProvider>,
  );
  assert.equal(
    forbiddenJaLeaks.test(document.body.textContent || ""),
    false,
    "AboutEOStageModal in ja leaked Simplified Chinese",
  );
  cleanup();

  render(
    <I18nProvider>
      <AdminBoundaryModal open onClose={() => {}} onSelectBoundary={async () => {}} />
    </I18nProvider>,
  );
  assert.equal(
    forbiddenJaLeaks.test(document.body.textContent || ""),
    false,
    "AdminBoundaryModal in ja leaked Simplified Chinese",
  );
  cleanup();
});

test("TaskArtifactsVerificationPanel renders zero CJK in English and zero Simplified Chinese in Japanese across empty and populated states", () => {
  const sampleTasks = [
    {
      task_id: "task-art-1",
      provider: "CDSE",
      dataset: "SENTINEL-1",
      title: "S1A_IW_SLC",
      status: "completed",
      done_bytes: 2048,
      total_bytes: 2048,
      items_count: 1,
      completed_items_count: 1,
      items_unit: "scenes",
      output_dir: "/tmp/eostage-outputs",
    },
    {
      task_id: "task-art-comp-1",
      auxiliary_kind: "AUX_POEORB",
      source_task_id: "task-art-1",
      provider: "CDSE",
      dataset: "SENTINEL-1",
      status: "completed",
      done_bytes: 512,
      total_bytes: 512,
    },
  ] as any[];

  for (const locale of ["en", "ja"] as const) {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    const emptyView = render(
      <I18nProvider>
        <TaskArtifactsVerificationPanel tasks={[]} />
      </I18nProvider>,
    );
    const emptyText = emptyView.getByTestId("artifacts-verification-panel").textContent || "";
    if (locale === "en") {
      assert.ok(emptyText.includes("No delivered outputs yet"));
      assert.equal(/[\u4e00-\u9fff]/.test(emptyText), false, `English empty artifacts leaked CJK: ${emptyText}`);
    } else {
      assert.ok(emptyText.includes("成果物はまだありません"));
      assert.equal(/暂无交付成果/.test(emptyText), false, `Japanese empty artifacts leaked zh-CN: ${emptyText}`);
    }
    cleanup();

    const populatedView = render(
      <I18nProvider>
        <TaskArtifactsVerificationPanel tasks={sampleTasks} onOpenFolder={() => {}} onOpenTask={() => {}} />
      </I18nProvider>,
    );
    const popText = populatedView.getByTestId("artifacts-verification-panel").textContent || "";
    if (locale === "en") {
      assert.ok(popText.includes("Local files verified"));
      assert.ok(popText.includes("Open folder"));
      assert.equal(/[\u4e00-\u9fff]/.test(popText), false, `English artifacts card leaked CJK: ${popText}`);
    } else {
      assert.ok(popText.includes("ローカルファイル検証済み"));
      assert.ok(popText.includes("フォルダーを開く"));
      assert.equal(/暂无交付成果|本地文件已核验|伴随精轨|已完成|打开目录|查看任务|已落盘/.test(popText), false, `Japanese artifacts card leaked zh-CN: ${popText}`);
    }
    cleanup();
  }
});
