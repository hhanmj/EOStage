import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import React from "react";
import { cleanup, fireEvent, render } from "@/test-utils/render-harness";
import { I18nProvider } from "@/i18n";
import { InplaceLoginCard } from "@/shell/components/InplaceLoginCard";
import { DesktopDownloadDrawer } from "./DesktopDownloadDrawer";
import { AgentComposer } from "./AgentComposer";
import { DownloadDestinationConfirmation } from "@/components/DownloadDestinationConfirmation";
import { createDownloadAcquisitionDraft } from "@/lib/downloadDestinationConfirmation";
import type { TypedError } from "@/shell/api/types";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test.afterEach(() => {
  cleanup();
});

test("InplaceLoginCard overlays above DesktopDownloadDrawer and ManualWorkbench does not trap it in z-[1000]", () => {
  const authError: TypedError = {
    code: "EARTHDATA_ACCOUNT_LOGIN_REQUIRED",
    message_zh: "需要登录 Earthdata 后才能继续",
    retryable: false,
    needs_account: "earthdata",
    needs_user_confirmation: false,
  };

  const view = render(
    <I18nProvider>
      <DesktopDownloadDrawer
        open={true}
        onClose={() => {}}
        tasks={[]}
        pendingPreparations={[]}
      />
      <InplaceLoginCard
        error={authError}
        onClose={() => {}}
        onSuccess={() => {}}
        onOpenSettings={() => {}}
      />
    </I18nProvider>,
  );

  const drawerDialog = view.getByRole("dialog", { name: "下载与任务中心" });
  const authDialog = view.getByRole("dialog", { name: "数据中心账户授权" });
  assert.ok((drawerDialog.getAttribute("class") || "").includes("z-[2500]"));
  assert.ok((authDialog.getAttribute("class") || "").includes("z-[2600]"));

  const manualWorkbenchPath = path.resolve(__dirname, "ManualWorkbench.tsx");
  const manualWorkbenchSrc = fs.readFileSync(manualWorkbenchPath, "utf8");
  assert.equal(
    /z-\[1000\][\s\S]{0,120}<InplaceLoginCard/.test(manualWorkbenchSrc),
    false,
    "ManualWorkbench must not wrap InplaceLoginCard in a lower stacking context",
  );
});

test("DesktopDownloadDrawer renders clear failure reason and actionable configure-account button", () => {
  const configuredIds: Array<string | undefined> = [];
  const view = render(
    <I18nProvider>
      <DesktopDownloadDrawer
        open={true}
        onClose={() => {}}
        tasks={[]}
        onConfigureAccount={(accountId) => configuredIds.push(accountId)}
        pendingPreparations={[
          {
            id: "prep-s2-1",
            title: "Sentinel-2 L2A",
            count: 1,
            outputRoot: "C:\\test\\downloads",
            status: "failed",
            error:
              "免账号分析波段镜像（MPC）暂未收录该景影像。如需直接下载官方完整 SAFE 归档，请先配置 Copernicus Data Space (CDSE) 账号。",
            actionAccountId: "cdse",
            actionLabel: "前往配置 CDSE 账号",
          },
        ]}
      />
    </I18nProvider>,
  );

  const alertBox = view.getByRole("alert");
  assert.ok((alertBox.textContent || "").includes("免账号分析波段镜像（MPC）暂未收录该景影像"));
  const btn = view.getByTestId("prep-configure-account-btn");
  assert.equal(btn.textContent, "前往配置 CDSE 账号");
  fireEvent.click(btn);
  assert.deepEqual(configuredIds, ["cdse"]);
});

test("DownloadDestinationConfirmation renders CDSE & USGS configuration shortcuts and passes target accountId", () => {
  const openedAccountIds: Array<string | undefined> = [];
  const draft = createDownloadAcquisitionDraft({
    sourceBatchId: "s2-batch-1",
    sourceBatchRevision: 1,
    selectionScope: "exact_item",
    selectedProductIds: ["scene-s2-1"],
    selectedProductCount: 1,
    acquisition: {
      kind: "sentinel2",
      frozenParameters: {
        delivery_form: "analysis_bands",
        safe_archive_available: false,
        analysis_bands_available: true,
        band_preset: "all_reflectance",
        selected_band_keys: ["B02", "B03", "B04"],
      },
    },
    destinationPrefill: "F:/downloads/s2",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 2,
  });

  const view = render(
    <I18nProvider>
      <DownloadDestinationConfirmation
        key="sentinel2"
        draft={draft}
        chooseDirectory={async () => null}
        onCancel={() => {}}
        onConfirm={async () => {}}
        onOpenAccountSettings={(accountId) => {
          openedAccountIds.push(accountId);
        }}
      />
    </I18nProvider>,
  );

  const btn = view.getByTestId("confirm-open-cdse-account-btn");
  assert.equal(btn.textContent, "前往配置 CDSE 账号");
  fireEvent.click(btn);
  assert.deepEqual(openedAccountIds, ["cdse"]);

  const landsatDraft = createDownloadAcquisitionDraft({
    sourceBatchId: "landsat-batch-1",
    sourceBatchRevision: 1,
    selectionScope: "exact_item",
    selectedProductIds: ["LC09_L2SP_1"],
    selectedProductCount: 1,
    acquisition: {
      kind: "landsat",
      frozenParameters: {
        delivery_form: "complete_asset_set",
        official_archive_available: false,
        complete_asset_set_available: true,
        official_archive_unavailable_reason: "需要先配置 USGS M2M 账号才能下载官方原始归档（.tar）。",
      },
    },
    destinationPrefill: "F:/downloads/landsat",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 2,
  });

  view.rerender(
    <I18nProvider>
      <DownloadDestinationConfirmation
        key="landsat"
        draft={landsatDraft}
        chooseDirectory={async () => null}
        onCancel={() => {}}
        onConfirm={async () => {}}
        onOpenAccountSettings={(accountId) => {
          openedAccountIds.push(accountId);
        }}
      />
    </I18nProvider>,
  );

  const usgsBtn = view.getByTestId("confirm-open-usgs-account-btn");
  assert.equal(usgsBtn.textContent, "前往配置 USGS M2M 账号");
  fireEvent.click(usgsBtn);
  assert.deepEqual(openedAccountIds, ["cdse", "usgs_m2m"]);
});

test("AgentComposer shows clear inline model-required hint when text entered or Enter pressed without configured model", () => {
  let openedModelSettings = 0;
  let draftText = "";

  const view = render(
    <I18nProvider>
      <AgentComposer
        value={draftText}
        onInputChange={(next) => {
          draftText = next;
        }}
        onSubmit={() => {}}
        channelReadiness={{
          ready: false,
          disabled: true,
          reason_zh: "未配置可用模型渠道",
        }}
        channels={[]}
        sessionId={null}
        onChannelGoSettings={() => {
          openedModelSettings += 1;
        }}
        permissionView={{
          effective: "confirm_changes",
          session_override: null,
          global_default: "confirm_changes",
        }}
        permissionStatus="ready"
        onPermissionSelect={() => {}}
        onPermissionSetGlobal={() => {}}
      />
    </I18nProvider>,
  );

  const sendBtn = view.getByTestId("agent-send-btn") as HTMLButtonElement;
  assert.equal(sendBtn.disabled, true);
  assert.equal(sendBtn.getAttribute("title"), "请先配置模型渠道后发送");
  assert.ok(view.queryByTestId("composer-model-required-hint") === null);

  // 空输入按 Enter 也会触发未配置模型提醒条
  const input = view.getByRole("textbox", { name: "科学需求输入框" });
  fireEvent.keyDown(input, { key: "Enter" });

  const hint = view.getByTestId("composer-model-required-hint");
  assert.ok((hint.textContent || "").includes("暂未配置对话模型"));
  const goBtn = view.getByTestId("composer-go-model-settings-btn");
  fireEvent.click(goBtn);
  assert.equal(openedModelSettings, 1);
});

test("DesktopDownloadDrawer shows configure-account button on failed USGS M2M task card and passes accountId", () => {
  const configuredIds: Array<string | undefined> = [];
  const view = render(
    <I18nProvider>
      <DesktopDownloadDrawer
        open={true}
        onClose={() => {}}
        onConfigureAccount={(accountId) => configuredIds.push(accountId)}
        tasks={[
          {
            task_id: "task-usgs-fail-1",
            revision: 1,
            category: "scene_download",
            name: "Landsat C2 L2 · 官方归档",
            provider: "usgs_m2m",
            dataset: "science.landsat.c2-l2.v1",
            status: "failed",
            done_bytes: 0,
            total_bytes: null,
            error: {
              code: "USGS_M2M_AUTH_FAILED",
              message_zh: "USGS M2M 已保存凭据当前无法建立下载会话，请检查账号状态后重试。",
              retryable: false,
              needs_account: "usgs_m2m",
              needs_user_confirmation: false,
            },
          },
        ]}
        pendingPreparations={[]}
      />
    </I18nProvider>,
  );

  const btn = view.getByTestId("task-configure-account-btn");
  assert.equal(btn.textContent, "前往配置 USGS M2M 账号");
  fireEvent.click(btn);
  assert.deepEqual(configuredIds, ["usgs_m2m"]);
});
