/**
 * AgentWorkflowSuite.interaction.test.tsx
 * Agent 工作流套件（性能与用量 + 工作步骤双独立设置页、上下文容量胶囊、
 * 工具上下文防爆代理、OAuth PKCE 免 Key 浏览器授权、多要素矢量选择、QGIS 目录清单导出）UI 交互测试套件
 */

import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY } from "@/i18n";
import { AgentContextHud } from "./AgentContextHud";
import { AgentToolGroupCard } from "./AgentToolGroupCard";
import { SettingsModal } from "../SettingsModal";
import { PerformanceUsageSettingsTab } from "@/shell/components/settings/PerformanceUsageSettingsTab";
import { WorkStepsSettingsTab } from "@/shell/components/settings/WorkStepsSettingsTab";
import { ModelChannelsSettingsTab } from "@/shell/components/settings/ModelChannelsSettingsTab";
import { PluginsAndSkillsPage } from "./PluginsAndSkillsPage";
import { VectorFeaturePickerModal } from "./VectorFeaturePickerModal";
import { DownloadTaskRow } from "./DownloadTaskRow";
import {
  getPerformanceOnlyPrefs,
  setPerformanceOnlyPrefs,
  getWorkStepsPrefs,
  setWorkStepsPrefs,
} from "@/lib/performanceAndUsagePrefs";
import type { ApplicationTask } from "@/shell/api/types";

test.afterEach(() => {
  cleanup();
  setPerformanceOnlyPrefs({ hudMode: "minimal" });
  setWorkStepsPrefs({
    stepDetailMode: "concise",
    collapseTiming: "completion",
    foldedReplies: "all",
  });
});

test("1. SettingsModal 采用三分组导航，并将「性能与用量」与「工作步骤显示」合并入「外观」，联动真实数据 AgentContextHud 与 AgentToolGroupCard", () => {
  const view = render(
    <I18nProvider>
      <div>
        <SettingsModal
          open={true}
          onClose={() => {}}
          initialCategory="appearance"
        />
        <AgentContextHud
          runs={[
            {
              run_id: "r1",
              status: "completed",
              started_at: "2026-11-05T10:00:00.000Z",
              ended_at: "2026-11-05T10:00:04.000Z",
              usage_total: { prompt: 8000, completion: 400 },
            },
          ]}
          events={[
            { kind: "user_message", payload: { text: "查询 Sentinel-1" } },
            { kind: "tool_call", payload: { tool_id: "search_scenes", duration_ms: 1200 } },
          ]}
          draftText="请按轨道号筛选"
        />
        <AgentToolGroupCard
          item={{
            kind: "tool_group",
            seq: 2,
            run_id: "r1",
            status: "done",
            rounds: [
              {
                run_id: "r1",
                call_id: "c1",
                tool_id: "search_scenes",
                status: "done",
                summary_zh: "检索到 12 景 Sentinel-1 SLC 影像（轨道 142 升轨）",
              },
            ],
          }}
          resolveToolName={(id) => (id === "search_scenes" ? "检索遥感数据" : id)}
        />
      </div>
    </I18nProvider>
  );

  // 1. 左侧设置栏分为「基础设置」「Agent 功能」「数据与统计」三大分组，且性能与用量/工作步骤显示不再单独列出
  assert.ok(view.getByText("基础设置"));
  assert.ok(view.getByText("Agent 功能"));
  assert.ok(view.getByText("数据与统计"));
  assert.equal(view.queryByRole("button", { name: "性能与用量" }), null);
  assert.equal(view.queryByRole("button", { name: "工作步骤显示" }), null);

  // 2. 当前处于「外观」面板：同时包含工作步骤显示与性能与用量设置项
  assert.ok(view.getByTestId("appearance-merged-settings"));
  assert.ok(view.getByTestId("performance-usage-settings-tab"));
  assert.ok(view.getByTestId("work-steps-settings-tab"));

  // 切换到详细 HUD 模式 (pro)
  fireEvent.click(view.getByTestId("perf-hud-mode-pro"));
  assert.equal(getPerformanceOnlyPrefs().hudMode, "pro");
  assert.equal(getWorkStepsPrefs().stepDetailMode, "concise");

  // 验证 AgentContextHud 详细模式底栏去重：仅展示核心指标（轮步 / 总用量 / 纯圆环），砍掉重复的 tok/s、平均缓存与圆环旁数字
  const activityPillText = view.getByTestId("pulse-pill-activity").textContent || "";
  assert.ok(activityPillText.includes("1轮 1步"));
  assert.ok(!activityPillText.includes("tok/s"), "详细模式底栏砍掉重复的 token 速率");
  const usagePillText = view.getByTestId("pulse-pill-usage").textContent || "";
  assert.ok(usagePillText.includes("8.4k tok"));
  assert.ok(!usagePillText.includes("平均缓存"), "详细模式底栏砍掉重复的平均缓存");
  assert.equal(
    (view.getByTestId("pulse-pill-context").textContent || "").trim(),
    "",
    "详细模式圆环旁不显示重复数字"
  );

  // 点击会话统计胶囊：输出速度 (100 tok/s) 仅在对应浮层中展示
  fireEvent.click(view.getByTestId("pulse-pill-activity"));
  assert.ok(view.getByTestId("pulse-panel-activity").textContent?.includes("100 tok/s"));

  // 点击 Token 用量胶囊：详细输入/输出与缓存命中率仅在 Token 用量浮层展示
  fireEvent.click(view.getByTestId("pulse-pill-usage"));
  assert.ok(view.getByTestId("pulse-hud-popover"));
  assert.ok(view.getByTestId("pulse-panel-usage"));
  assert.ok(view.getByText("8,400"));

  // 点击上下文圆环：详细模式下上下文浮层只展示容量分解与自动压缩水位，不重复平均缓存/tok/s/累计用量
  fireEvent.click(view.getByTestId("pulse-pill-context"));
  const proContextPanel = view.getByTestId("pulse-panel-context");
  assert.ok(proContextPanel);
  assert.ok(view.getByTestId("pulse-mode-toggle"));
  assert.ok(view.getByTestId("pulse-context-segmented-bar"));
  assert.ok(view.getByTestId("pulse-context-legend"));
  assert.ok(proContextPanel.textContent?.includes("自动压缩水位"));
  assert.equal(view.queryByTestId("pulse-context-cache-hit-rate"), null);
  assert.ok(!(proContextPanel.textContent || "").includes("tok/s"));
  assert.ok(!(proContextPanel.textContent || "").includes("累计"));

  // 右键胶囊切换回极简模式：工具栏仅保留纯圆圈 SVG 进度环，上下文浮层保留平均缓存命中率
  fireEvent.contextMenu(view.getByTestId("pulse-pill-activity"));
  assert.equal(getPerformanceOnlyPrefs().hudMode, "minimal");
  assert.equal(view.queryByTestId("pulse-pill-activity"), null);
  assert.equal(view.queryByTestId("pulse-pill-usage"), null);
  assert.equal((view.getByTestId("pulse-pill-context").textContent || "").trim(), "");
  assert.ok(view.getByTestId("pulse-context-ring-svg"));
  assert.ok(view.getByTestId("pulse-panel-context").textContent?.includes("已用 0.8%"));
  assert.ok(view.getByTestId("pulse-context-cache-hit-rate"));
  assert.equal(
    (view.getByTestId("pulse-context-cache-hit-rate").textContent || "").trim(),
    "未上报",
    "未上报缓存字段的渠道必须诚实显示「未上报」而非 0.0%"
  );
  fireEvent.contextMenu(view.getByTestId("pulse-pill-context"));
  assert.equal(getPerformanceOnlyPrefs().hudMode, "pro");
  assert.ok(view.getByTestId("perf-hud-right-click-hint"));
  assert.ok(view.getByTestId("pulse-panel-context").textContent?.includes("自动压缩水位"));

  // 3. 在同一个「外观」面板内操作工作步骤显示设置项
  fireEvent.click(view.getByTestId("work-step-collapse-next-input"));
  assert.equal(getWorkStepsPrefs().stepDetailMode, "concise");
  assert.equal(getWorkStepsPrefs().collapseTiming, "next-input");
  assert.ok(view.getByText("已展开"));

  fireEvent.click(view.getByTestId("work-step-collapse-completion"));
  assert.equal(getWorkStepsPrefs().collapseTiming, "completion");
  assert.ok(view.getByText("已收起"));

  fireEvent.click(view.getByTestId("work-step-mode-detailed"));
  assert.equal(getWorkStepsPrefs().stepDetailMode, "detailed");
  assert.ok(view.getByText("工具执行步骤"));
  assert.ok(view.getByText("search_scenes"));

  // 4. 点击左侧「数据与统计 -> 使用统计」入口可查看使用统计面板
  fireEvent.click(view.getByRole("button", { name: "使用统计" }));
  assert.ok(view.getByTestId("usage-stats-settings-tab"));
});

test("2. PerformanceUsageSettingsTab 与 WorkStepsSettingsTab 在英文 (en) 与日文 (ja) 下完整本地化且无残留硬编码中文", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "en");
  const enView = render(
    <I18nProvider>
      <div>
        <PerformanceUsageSettingsTab />
        <WorkStepsSettingsTab />
      </div>
    </I18nProvider>
  );

  assert.ok(enView.getAllByText("Performance & usage").length >= 1);
  assert.ok(enView.getAllByText("Work details").length >= 1);
  assert.equal(
    /[\u4e00-\u9fff]/.test(enView.container.textContent || ""),
    false,
    "英文模式下不应残留任何中文字符"
  );
  cleanup();

  window.localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  const jaView = render(
    <I18nProvider>
      <div>
        <PerformanceUsageSettingsTab />
        <WorkStepsSettingsTab />
      </div>
    </I18nProvider>
  );

  assert.ok(jaView.getAllByText("パフォーマンスと使用状況").length >= 1);
  assert.ok(jaView.getAllByText("作業の詳細").length >= 1);
  assert.equal(
    jaView.container.textContent?.includes("当前为简洁模式"),
    false,
    "日文模式下不应残留硬编码简体中文"
  );
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

test("3. PluginsAndSkillsPage 工具页签聚焦连接器管理并移除内部注册工具契约列表、假内置连接器与伪开关模拟卡", () => {
  const view = render(
    <I18nProvider>
      <PluginsAndSkillsPage initialTab="tools" />
    </I18nProvider>
  );

  assert.equal(view.queryByTestId("tools-context-broker-card"), null);
  assert.equal(view.queryByTestId("connector-card-connector-workbench-map-mcp"), null);
  assert.equal(view.queryByTestId("geodata-native-workbench"), null);
  assert.equal(view.queryByTestId("tool-entry-create_task"), null);
  assert.ok(view.getByTestId("connectors-empty-state"));
});

test("4. ModelChannelsSettingsTab 支持免 API Key 浏览器一键授权 (OAuth PKCE S256) 生成与回调校验", () => {
  const view = render(
    <I18nProvider>
      <ModelChannelsSettingsTab />
    </I18nProvider>
  );

  const toggleBtn = view.getByTestId("toggle-oauth-pkce-helper");
  fireEvent.click(toggleBtn);
  assert.ok(view.getByTestId("oauth-pkce-helper-panel"));

  // 生成 PKCE 授权链接并暴露一键打开浏览器按钮
  fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
  const urlDisplay = view.getByTestId("pkce-authorize-url-display");
  assert.ok(urlDisplay.textContent?.includes("code_challenge_method=S256"));
  assert.ok(view.getByTestId("open-pkce-browser-btn"));

  // 缺失 state 参数的回调链接必须被 CSRF 校验拦截
  fireEvent.change(view.getByTestId("pkce-callback-input"), {
    target: {
      value: "http://localhost:1455/auth/callback?code=ac_missing_state",
    },
  });
  fireEvent.click(view.getByTestId("verify-pkce-callback-btn"));
  assert.ok(
    view.getByTestId("pkce-status-banner").textContent?.includes("state")
  );

  // 粘贴仅含真实 Base64URL 编码 JWT (不含顶层 chatgpt_account_id 捷径字段) 的 OAuth Token JSON 并校验提取
  const validJwtPayload = Buffer.from(
    JSON.stringify({
      "https://api.openai.com/auth": {
        chatgpt_account_id: "acct_pkce_999",
      },
      exp: Math.floor(Date.now() / 1000) + 3600,
    }),
    "utf8"
  ).toString("base64url");
  fireEvent.change(view.getByTestId("pkce-callback-input"), {
    target: {
      value: JSON.stringify({
        access_token: `eyJhbGciOiJSUzI1NiJ9.${validJwtPayload}.sig`,
      }),
    },
  });
  fireEvent.click(view.getByTestId("verify-pkce-callback-btn"));
  assert.ok(
    view.getByTestId("pkce-status-banner").textContent?.includes("acct_pkce_999")
  );

  // 验证不再展示第三方本机 CLI 凭据扫描面板，以及精简无广告描述的官方预设卡片（含品牌 SVG 图标、Kimi 简写与防折行添加按钮）
  assert.equal(view.queryByTestId("local-cli-import-panel"), null);
  assert.equal(view.queryByTestId("refresh-local-cli-btn"), null);
  assert.equal(view.queryByTestId("local-cli-card-codex_cli"), null);
  assert.equal(view.queryByTestId("local-cli-card-gemini_cli"), null);
  const kimiCard = view.getByTestId("preset-provider-card-moonshot-kimi");
  assert.ok(kimiCard.textContent?.includes("Kimi"));
  assert.ok(!kimiCard.textContent?.includes("Kimi (Moonshot)"));
  assert.ok(!kimiCard.textContent?.includes("超长百万上下文"));

  // 已保留 4 个官方 OAuth 平台（OpenAI Codex、Claude Pro/Max、GitHub Copilot、OpenRouter），继续裁剪非官方预设；切换 PKCE 供应商标签应清空旧授权链接
  assert.equal(view.queryByTestId("pkce-provider-google_gemini_pkce"), null);
  assert.equal(view.queryByTestId("pkce-provider-kimi_code_device"), null);
  assert.ok(view.getByTestId("pkce-provider-claude_oauth_pkce"));
  assert.ok(view.getByTestId("pkce-provider-github_copilot_device"));
  fireEvent.click(view.getByTestId("pkce-provider-openrouter_pkce"));
  assert.equal(view.queryByTestId("pkce-authorize-url-display"), null);
  fireEvent.click(view.getByTestId("generate-pkce-auth-url-btn"));
  assert.ok(
    view.getByTestId("pkce-authorize-url-display").textContent?.includes("openrouter.ai/auth")
  );

  // 点击左侧 EOStage 官方入口，确认渲染单一官方视图且不混入自定义供应商
  fireEvent.click(view.getByTestId("official-provider-eostage-official"));
  assert.ok(view.getByTestId("official-eostage-panel"));
  assert.ok(view.getByTestId("official-panel-login-btn"));
  assert.ok(
    !view.getByTestId("provider-group-custom").textContent?.includes("eostage-official"),
    "自定义供应商列表不应混入 eostage-official"
  );
});

test("5. VectorFeaturePickerModal 与 DownloadTaskRow QGIS 目录清单导出验证", () => {
  let confirmedPayload: {
    featureIds: string[];
    displayField: string;
    displayName: string;
  } | null = null;

  const pickerView = render(
    <VectorFeaturePickerModal
      open={true}
      preview={{
        ok: true,
        path: "beijing_districts.shp",
        file_name: "beijing_districts.shp",
        total_features: 2,
        display_field: "NAME",
        fields: ["NAME", "CODE"],
        geojson: { type: "FeatureCollection", features: [] },
        features: [
          {
            id: "",
            index: undefined as any,
            name: "",
            properties: { CODE: "ZONE_A" },
          } as any,
          {
            id: "feat-fengtai",
            index: 1,
            name: "丰台区",
            properties: { CODE: "ZONE_B" },
          } as any,
        ],
      }}
      onConfirm={(params) => {
        confirmedPayload = params;
      }}
      onCancel={() => {}}
    />
  );

  assert.ok(pickerView.getByTestId("vector-feature-picker-modal"));
  fireEvent.click(pickerView.getByTestId("vector-picker-confirm"));
  assert.ok(confirmedPayload);
  assert.equal(confirmedPayload!.featureIds.length, 2);
  cleanup();

  const completedTask: ApplicationTask = {
    task_id: "task-qgis-demo",
    revision: 1,
    name: "Sentinel-1 汶川同震形变数据",
    category: "scene_download",
    status: "completed",
    provider: "ASF",
    dataset: "Sentinel-1 SLC",
    created_at: "2026-11-05T00:00:00Z",
    updated_at: "2026-11-05T01:00:00Z",
    done_bytes: 4_000_000_000,
    total_bytes: 4_000_000_000,
    items_count: 1,
    completed_items_count: 1,
    output_dir: "D:/SAR/Wenchuan",
    scene_items: [
      {
        item_id: "i1",
        scene_id: "S1A_IW_SLC__1SDV_20261001",
        status: "completed",
        done_bytes: 4_000_000_000,
        total_bytes: 4_000_000_000,
      },
    ],
  } as any;

  const rowView = render(
    <I18nProvider>
      <DownloadTaskRow task={completedTask} />
    </I18nProvider>
  );

  fireEvent.click(rowView.getByRole("button", { name: "展开分景进度" }));
  fireEvent.click(rowView.getByTestId("export-qgis-catalog-btn"));
  assert.ok(
    rowView.getByText("已导出 catalog.geojson（可直接拖入 QGIS）与 README_数据说明.md")
  );
});
