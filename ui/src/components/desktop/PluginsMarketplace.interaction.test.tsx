import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import {
  registerTestDesktopShellBridge,
  clearTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { PluginsAndSkillsPage } from "./PluginsAndSkillsPage";
import { SettingsModal } from "../SettingsModal";
import { UsageAnalyticsDashboard } from "../settings/UsageAnalyticsDashboard";
import {
  ProviderBrandLogo,
  resolveProviderBrandKey,
} from "../../shell/components/settings/ProviderBrandLogo";
import { ChannelListItem } from "../../shell/components/settings/ChannelListItem";
import { AgentModelPicker } from "./AgentModelPicker";
import { AgentComposer } from "./AgentComposer";
import { setPerformanceOnlyPrefs } from "@/lib/performanceAndUsagePrefs";
import type { AgentChannelView } from "@/lib/generated/v3-contracts";

if (typeof globalThis.localStorage === "undefined" && typeof window !== "undefined") {
  (globalThis as any).localStorage = window.localStorage;
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  setPerformanceOnlyPrefs({ hudMode: "minimal" });
  if (typeof localStorage !== "undefined") {
    localStorage.clear();
  }
});

test("Slice 7 Item 11: 移除应用内 DSH 外部市场、保留 EOStage 官方目录与 skill-edit-modal 防误触", async (t) => {
  await t.test("1. 插件市场仅保留 EOStage 官方源，不渲染 Extension Market / EOStage.stream / EOStage Plugin Hub 切换器", async () => {
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        async getPluginsStatus() {
          return {
            ok: true,
            plugins_root: "/tmp/plugins",
            installed_plugins: [],
            available_official_plugins: [],
            custom_skills: [],
            custom_connectors: [],
          };
        },
        async getRuntimeComponents() {
          return {
            ok: true,
            install_root: "/tmp/runtime",
            components: [],
          };
        },
      },
    } as any);

    const fetchRegistryOverride = async () =>
      ({
        ok: true,
        source_url: "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/raw/registry.json",
        updated_at: "2026-11-14T00:00:00Z",
        blocked_count: 0,
        market_id: "eostage",
        market_title: "EOStage 官方扩展源",
        entries: [],
      }) as any;

    const { getByTestId, queryByTestId } = render(
      <PluginsAndSkillsPage
        initialTab="plugins"
        fetchRegistryOverride={fetchRegistryOverride}
      />,
    );

    fireEvent.click(getByTestId("plugins-subtab-browse"));

    await waitFor(() => {
      assert.ok(getByTestId("plugin-marketplace-sources-bar"));
      assert.ok(getByTestId("plugins-browse-view"));
    });

    assert.equal(queryByTestId("eo-market-source-switcher"), null);
    assert.equal(queryByTestId("market-source-pill-dsh-market"), null);
    assert.equal(queryByTestId("market-source-pill-deepseek-stream"), null);
    assert.equal(queryByTestId("market-source-pill-dsh-plugin-hub"), null);
    assert.ok(getByTestId("plugin-author-filter-all"));
    assert.ok(getByTestId("plugin-author-filter-EOStage"));
  });

  await t.test("2. skill-edit-modal 保持 Slice 1 ModalOverlay closeOnBackdropClick={false}", async () => {
    const { getByTestId } = render(
      <PluginsAndSkillsPage initialTab="skills" />,
    );
    fireEvent.click(getByTestId("create-skill-btn"));
    assert.ok(getByTestId("skill-edit-modal"));
    const overlay = getByTestId("skill-edit-overlay");
    fireEvent.click(overlay);
    assert.ok(getByTestId("skill-edit-modal"), "clicking backdrop must not close skill-edit-modal");
  });
});

test("Slice 7 Item 10: UsageAnalyticsDashboard & SettingsModal SVG 交互统计面板", async (t) => {
  await t.test("1. UsageAnalyticsDashboard 30 天趋势切换、悬停明细、Donut、Tool 与 Provider/GEE 真实聚合展示", () => {
    const today = new Date().toISOString().slice(0, 10);
    const { getByTestId, container } = render(
      <UsageAnalyticsDashboard
        stats={{
          prompt_tokens: 12000,
          completion_tokens: 3400,
          reasoning_tokens: 800,
          cached_tokens: 1500,
          total_tokens: 16200,
          run_count: 5,
          download_total_tasks: 2,
          download_completed_tasks: 2,
          download_active_tasks: 0,
          download_failed_tasks: 0,
          download_transferred_bytes: 1024 * 1024 * 50,
          daily_series: [
            {
              date: today,
              prompt_tokens: 12000,
              completion_tokens: 3400,
              reasoning_tokens: 800,
              runs: 5,
              download_bytes: 1024 * 1024 * 50,
            },
          ],
          model_breakdown: [
            {
              route: "openai/gpt-5.2",
              channel_id: "openai",
              model_id: "gpt-5.2",
              run_count: 5,
              model_calls: 6,
              tool_calls: 4,
              prompt_tokens: 12000,
              completion_tokens: 3400,
              reasoning_tokens: 800,
              cached_tokens: 1500,
              total_tokens: 16200,
            },
          ],
          tool_breakdown: [
            { tool_id: "sar_search", call_count: 4, success_count: 4 },
          ],
          provider_download_breakdown: [
            {
              source_id: "asf",
              display_name: "ASF Sentinel-1 SAR",
              task_count: 2,
              bytes_transferred: 1024 * 1024 * 50,
            },
          ],
          gee_quota_status: {
            configured: true,
            verified: true,
            project_id: "eostage-gee-proj",
            auth_mode: "service_account",
          },
        }}
      />,
    );

    assert.ok(getByTestId("usage-analytics-dashboard"));
    assert.ok(getByTestId("usage-analytics-daily-chart"));
    assert.ok(getByTestId("usage-analytics-model-donut"));
    assert.ok(getByTestId("usage-analytics-tools").textContent?.includes("sar_search"));
    assert.ok(getByTestId("usage-analytics-providers").textContent?.includes("ASF Sentinel-1 SAR"));
    assert.ok(getByTestId("usage-analytics-gee").textContent?.includes("eostage-gee-proj"));

    // Hover a day group in the daily SVG chart to inspect exact tooltip
    const dayGroup = container.querySelector('[data-testid="usage-analytics-daily-chart"] svg g[tabindex="0"]');
    assert.ok(dayGroup, "expected interactive SVG bar group");
    fireEvent.mouseEnter(dayGroup!);
    const tooltip = getByTestId("usage-daily-tooltip");
    assert.ok(tooltip.textContent?.includes(today));
    assert.ok(tooltip.textContent?.includes("12,000"));

    // Switch chart mode to Downloads
    fireEvent.click(getByTestId("usage-chart-mode-downloads"));
    assert.ok(tooltip.textContent?.includes("下载流量"));
  });

  await t.test("2. SettingsModal usage_stats 页签保留全部既有 data-testid 且挂载可视化面板", async () => {
    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (cmd: string) => {
        if (cmd === "application_usage_statistics") {
          return {
            ok: true,
            collected_at: "2026-11-14T12:00:00Z",
            data_root: "/tmp/eostage",
            sqlite_size_bytes: 8192,
            disk_free_bytes: 1024 * 1024 * 1024,
            workspace_count: 1,
            session_count: 3,
            run_count: 7,
            completed_run_count: 6,
            tool_call_count: 9,
            prompt_tokens: 4000,
            completion_tokens: 1000,
            reasoning_tokens: 200,
            cached_tokens: 500,
            total_tokens: 5200,
            model_breakdown: [],
            download_total_tasks: 1,
            download_completed_tasks: 1,
            download_active_tasks: 0,
            download_failed_tasks: 0,
            download_transferred_bytes: 2048,
            download_selected_items: 1,
            installed_plugins_count: 1,
            custom_skills_count: 2,
            daily_series: [],
            tool_breakdown: [],
            provider_download_breakdown: [],
            gee_quota_status: null,
          };
        }
        return {};
      },
    };

    const { getByTestId } = render(
      <SettingsModal
        open={true}
        initialCategory="usage_stats"
        onClose={() => {}}
      />,
    );

    await waitFor(() => {
      assert.ok(getByTestId("usage-stat-sessions"));
      assert.ok(getByTestId("usage-stat-runs"));
      assert.ok(getByTestId("usage-stat-tokens"));
      assert.ok(getByTestId("usage-stat-downloads"));
      assert.ok(getByTestId("usage-stats-model-table"));
      assert.ok(getByTestId("refresh-usage-stats-btn"));
      assert.ok(getByTestId("usage-analytics-dashboard"));
    });
  });

  await t.test("3. 空数据时 UsageAnalyticsDashboard 如实展示 0 数据诚实空态（绝不伪造估算）", () => {
    const { getByTestId } = render(
      <UsageAnalyticsDashboard
        stats={{
          prompt_tokens: 0,
          completion_tokens: 0,
          reasoning_tokens: 0,
          cached_tokens: 0,
          total_tokens: 0,
          run_count: 0,
          download_total_tasks: 0,
          download_completed_tasks: 0,
          download_active_tasks: 0,
          download_failed_tasks: 0,
          download_transferred_bytes: 0,
          daily_series: [],
          model_breakdown: [],
          tool_breakdown: [],
          provider_download_breakdown: [],
          gee_quota_status: null,
        }}
      />,
    );
    assert.ok(getByTestId("usage-analytics-dashboard").textContent?.includes("近 30 天暂无用量记录"));
    assert.ok(getByTestId("usage-analytics-model-donut").textContent?.includes("暂无模型消耗分布记录"));
    assert.ok(getByTestId("usage-analytics-tools").textContent?.includes("暂无工具调用记录"));
  });

  await t.test("4. SettingsModal network 页签正确挂载 TransferEngineSettingsSection", () => {
    const { getByTestId } = render(
      <SettingsModal
        open={true}
        initialCategory="network"
        onClose={() => {}}
      />,
    );
    assert.ok(getByTestId("transfer-engine-settings-section"));
  });
});

test("真实厂商商标图标与超长模型名省略号回归测试", async (t) => {
  await t.test("1. ProviderBrandLogo 与 ChannelListItem 映射全部 12 家官方商标且渐变 ID 互不冲突", () => {
    assert.equal(resolveProviderBrandKey("custom-oai", "https://api.openai.com/v1"), "openai");
    assert.equal(resolveProviderBrandKey("anthropic", "https://api.anthropic.com/v1"), "anthropic");
    assert.equal(resolveProviderBrandKey("claude-code-cli", "https://api.anthropic.com/v1"), "claude");
    assert.equal(resolveProviderBrandKey("gemini", "https://generativelanguage.googleapis.com/v1beta"), "google-gemini");
    assert.equal(resolveProviderBrandKey("tokenh", "https://relay.example.invalid/v1", "gemini-2.5-pro"), "custom");
    assert.equal(resolveProviderBrandKey("deepseek", "https://api.deepseek.com/v1"), "deepseek");
    assert.equal(resolveProviderBrandKey("kimi-cn", "https://api.moonshot.cn/v1"), "moonshot-kimi");
    assert.equal(resolveProviderBrandKey("minimax", "https://api.minimaxi.com/v1"), "minimax");
    assert.equal(resolveProviderBrandKey("zhipu", "https://open.bigmodel.cn/api/paas/v4"), "zhipu-glm");
    assert.equal(resolveProviderBrandKey("aliyun-bailian", "https://dashscope.aliyuncs.com/compatible-mode/v1"), "aliyun-bailian");
    assert.equal(resolveProviderBrandKey("openrouter", "https://openrouter.ai/api/v1"), "openrouter");
    assert.equal(resolveProviderBrandKey("xai", "https://api.x.ai/v1"), "xai-grok");
    assert.equal(resolveProviderBrandKey("github-copilot", ""), "github-copilot");
    assert.equal(resolveProviderBrandKey("my-relay", "https://relay.example.invalid/v1"), "custom");

    const mkChannel = (id: string, base_url: string, model: string): AgentChannelView => ({
      id,
      protocol: "chat_completions",
      base_url,
      model,
      reasoning_mode: "off",
      key_ready: true,
      is_default: false,
    });

    const view = render(
      <div>
        <ChannelListItem channel={mkChannel("codex-local", "https://api.openai.com/v1", "gpt-5.4")} isSelected={false} onSelect={() => {}} />
        <ChannelListItem channel={mkChannel("gemini-relay", "https://generativelanguage.googleapis.com/v1beta", "gemini-2.5-pro")} isSelected={false} onSelect={() => {}} />
        <ChannelListItem channel={mkChannel("minimax-node", "https://api.minimaxi.com/v1", "minimax-m2")} isSelected={false} onSelect={() => {}} />
        <ChannelListItem channel={mkChannel("claude-cli", "https://api.anthropic.com/v1", "claude-sonnet-4-5")} isSelected={false} onSelect={() => {}} />
        <ProviderBrandLogo provider="gemini" />
        <ProviderBrandLogo provider="gemini" />
        <ProviderBrandLogo provider="minimax" />
        <ProviderBrandLogo provider="minimax" />
      </div>,
    );

    const brands = Array.from(view.container.querySelectorAll("svg[data-brand]")).map((el) =>
      el.getAttribute("data-brand"),
    );
    assert.deepEqual(brands, [
      "openai",
      "google-gemini",
      "minimax",
      "claude",
      "google-gemini",
      "google-gemini",
      "minimax",
      "minimax",
    ]);

    const gradIds = Array.from(view.container.querySelectorAll("linearGradient[id]")).map((el) =>
      el.getAttribute("id"),
    );
    assert.ok(gradIds.length >= 4, `expected at least 4 gradient IDs, got ${gradIds.length}`);
    assert.equal(new Set(gradIds).size, gradIds.length, "SVG linearGradient IDs must be unique");
  });

  await t.test("2. AgentModelPicker 与 AgentComposer 为超长模型名提供省略号截断、完整 tooltip 及详细 HUD 紧凑宽度", async () => {
    const longModelId = "anthropic/claude-opus-4-5-20261101-ultra-long-reasoning-preview";
    const channel: AgentChannelView = {
      id: "tokenh-enterprise-gateway-region-ap-southeast",
      protocol: "chat_completions",
      base_url: "https://relay.example.invalid/v1",
      model: longModelId,
      models: [{ id: longModelId, input_types: ["text"], reasoning_levels: [] }],
      reasoning_mode: "off",
      key_ready: true,
      is_default: true,
    };

    const minimalPicker = render(
      <AgentModelPicker
        channels={[channel]}
        sessionId="long-model-minimal"
        ready={true}
      />,
    );
    fireEvent.click(minimalPicker.getByTestId("agent-model-picker-trigger"));
    fireEvent.click(await waitFor(() => minimalPicker.getByRole("menuitemradio", { name: longModelId })));

    const minimalLabel = minimalPicker.getByTestId("agent-model-picker-label");
    assert.equal(minimalLabel.textContent, longModelId);
    assert.ok(minimalLabel.className.includes("truncate"));
    assert.ok(minimalLabel.className.includes("min-w-0"));

    const minimalTrigger = minimalPicker.getByTestId("agent-model-picker-trigger");
    assert.ok((minimalTrigger.getAttribute("title") || "").includes(longModelId));
    assert.ok((minimalTrigger.getAttribute("title") || "").includes(channel.id));
    assert.ok(minimalTrigger.className.includes("max-w-full"));
    assert.ok((minimalTrigger.parentElement?.className || "").includes("max-w-[130px]"));

    cleanup();

    setPerformanceOnlyPrefs({ hudMode: "pro" });
    const composerView = render(
      <AgentComposer
        value=""
        onInputChange={() => {}}
        onSubmit={() => {}}
        channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
        channels={[channel]}
        sessionId="long-model-pro"
        contextRuns={[
          {
            run_id: "r-pro-1",
            status: "completed",
            started_at: "2026-11-14T00:00:00.000Z",
            ended_at: "2026-11-14T00:00:02.000Z",
            usage_total: { prompt: 12000, completion: 800 },
          },
        ]}
        permissionView={null}
        permissionStatus="ready"
        onPermissionSelect={() => {}}
        onPermissionSetGlobal={() => {}}
      />,
    );

    assert.ok(composerView.getByTestId("pulse-pill-usage"));
    assert.ok(composerView.getByTestId("pulse-pill-context"));
    const proTrigger = composerView.getByTestId("agent-model-picker-trigger");
    assert.ok(proTrigger.className.includes("max-w-full"));
    assert.ok((proTrigger.parentElement?.className || "").includes("max-w-[104px]"));
    fireEvent.click(proTrigger);
    fireEvent.click(await waitFor(() => composerView.getByRole("menuitemradio", { name: longModelId })));
    const proLabel = composerView.getByTestId("agent-model-picker-label");
    assert.ok(proLabel.className.includes("truncate"));
    assert.ok((composerView.getByTestId("agent-model-picker-trigger").getAttribute("title") || "").includes(longModelId));
  });

  await t.test("5. SettingsModal 默认落点为 appearance 且记忆上次页签，并托管 runtime_tools 运行环境", async () => {
    window.localStorage.removeItem("eostage.shell.lastSettingsCategory");
    const settingsModal = await import("../SettingsModal");
    assert.equal(settingsModal.readLastSettingsCategory(), "appearance");

    window.localStorage.setItem("eostage.shell.lastSettingsCategory", "work_steps");
    assert.equal(settingsModal.readLastSettingsCategory(), "appearance");

    const view = render(
      <SettingsModal open={true} onClose={() => {}} />
    );

    const navAppearance = view.getByTestId("settings-nav-appearance");
    assert.equal(navAppearance.getAttribute("aria-current"), "page");

    fireEvent.click(view.getByTestId("settings-nav-runtime_tools"));
    assert.equal(
      window.localStorage.getItem("eostage.shell.lastSettingsCategory"),
      "runtime_tools"
    );
    assert.ok(view.getByTestId("runtime-tools-settings-tab"));
    assert.ok(view.getByTestId("runtime-panel"));
  });

  await t.test("6. AgentModelPicker 在模型下拉列表和当前触发按钮上均显示免费模型 Free 绿色角标", async () => {
    cleanup();
    const channel: AgentChannelView = {
      id: "eostage-official",
      protocol: "chat_completions",
      base_url: "https://api.eostage.org/eostage/v1",
      model: "neohorse-1.5-flash-preview",
      models: [
        {
          id: "neohorse-1.5-flash-preview",
          name: "NeoHorse 1.5 Flash Preview",
          is_free: true,
        },
        {
          id: "glm-5.3-flash",
          name: "GLM-5.3 Flash",
          is_free: false,
        },
      ],
      key_ready: true,
      is_default: true,
    };

    const picker = render(
      <AgentModelPicker
        channels={[channel]}
        sessionId="free-badge-picker"
        ready={true}
      />,
    );

    const triggerBadge = await waitFor(() => picker.getByTestId("agent-model-picker-free-badge"));
    assert.equal(triggerBadge.textContent, "Free");
    assert.ok(triggerBadge.className.includes("text-sp-success-text"));

    fireEvent.click(picker.getByTestId("agent-model-picker-trigger"));
    const freeRowBadge = await waitFor(() =>
      picker.getByTestId("model-picker-free-badge-neohorse-1.5-flash-preview")
    );
    assert.equal(freeRowBadge.textContent, "Free");
    assert.ok(freeRowBadge.className.includes("eo-settings-badge-free"));
    assert.equal(picker.queryByTestId("model-picker-free-badge-glm-5.3-flash"), null);

    fireEvent.click(picker.getByRole("menuitemradio", { name: /GLM-5\.3 Flash/i }));
    await waitFor(() => {
      assert.equal(picker.queryByTestId("agent-model-picker-free-badge"), null);
    });

    // Non-official channel MUST never display Free badge even if is_free=true exists in data
    cleanup();
    const customSpoof: AgentChannelView = {
      id: "custom-openai",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-spoof-free",
      models: [{ id: "gpt-spoof-free", name: "Spoof Free", is_free: true }],
      key_ready: true,
      is_default: true,
    };
    const customPicker = render(
      <AgentModelPicker
        channels={[customSpoof]}
        sessionId="free-badge-custom-blocked"
        ready={true}
      />,
    );
    await waitFor(() => {
      assert.equal(customPicker.getByTestId("agent-model-picker-label").textContent, "Spoof Free");
    });
    assert.equal(customPicker.queryByTestId("agent-model-picker-free-badge"), null);
    fireEvent.click(customPicker.getByTestId("agent-model-picker-trigger"));
    await waitFor(() => customPicker.getByRole("menuitemradio", { name: "Spoof Free" }));
    assert.equal(customPicker.queryByTestId("model-picker-free-badge-gpt-spoof-free"), null);
  });
});
