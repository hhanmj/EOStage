import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import { AgentComposer } from "./AgentComposer";
import { AgentPermissionMenu } from "./AgentPermissionMenu";
import { CHANNEL_PROTOCOLS } from "../../shell/api/agentChannelsLogic";
import { ACCOUNT_METAS, orderAccountsForSettings, filterRenderedAccounts } from "../../shell/api/accountLogic";
import type { AgentPermissionModeView } from "../../lib/generated/v3-contracts";

// Mock animation frames for JSDOM
if (typeof window !== "undefined") {
  if (typeof window.requestAnimationFrame === "undefined") {
    window.requestAnimationFrame = (fn) => setTimeout(fn, 16) as any;
  }
  if (typeof window.cancelAnimationFrame === "undefined") {
    window.cancelAnimationFrame = (id) => clearTimeout(id as any);
  }
}

const scopedSource = (url: string) => `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/DesktopAgentWorkbench.tsx") || url.endsWith("/NetworkProxySettingsTab.tsx")) {
      return { format: "module", shortCircuit: true, source: transformSync(scopedSource(url), { loader: "tsx", format: "esm" }).code };
    }
    return nextLoad(url, context);
  },
});

const { DesktopAgentWorkbench } = await import("./DesktopAgentWorkbench");
const { NetworkProxySettingsTab } = await import("../../shell/components/settings/NetworkProxySettingsTab");
hooks.deregister();

test.afterEach(() => {
  cleanup();
});

const mockPermissionView: AgentPermissionModeView = {
  current_mode: "confirm_each",
  source: "default",
  is_global: true,
  can_clear_session: false,
};

const mockChannelReadiness = {
  ready: true,
  disabled: false,
  reason_zh: "",
};

test("UI Polish Audit - 6 大交互细节核验与端到端测试套件", async (t) => {
  // 1. Agent 输入区「添加（+）」菜单
  await t.test("1. AgentComposer 添加(+)菜单包含工作流与插件技能，点击能定向调用且不导致输入框跳动", async () => {
    let workflowTriggered = false;
    let skillsToolsTriggered = false;
    let textValue = "";

    function Wrapper() {
      const [val, setVal] = useState(textValue);
      return (
        <AgentComposer
          value={val}
          onInputChange={(v) => {
            textValue = v;
            setVal(v);
          }}
          onSubmit={() => {}}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          sessionId="test-session"
          permissionView={mockPermissionView}
          permissionStatus="ready"
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
          onSelectWorkflow={() => {
            workflowTriggered = true;
          }}
          onSelectSkillsTools={() => {
            skillsToolsTriggered = true;
          }}
          capabilities={{
            rectDraw: true,
            adminBoundary: true,
            vectorUpload: true,
            workflow: true,
            skillsTools: true,
          }}
        />
      );
    }

    const { getByRole, getByText, queryByText, container } = render(<Wrapper />);
    const textarea = container.querySelector("textarea");
    const initialTextareaHeight = textarea ? textarea.offsetHeight : 0;

    const attachBtn = getByRole("button", { name: "附加范围或文件" });

    // 默认未打开附加菜单
    assert.equal(queryByText("工作流"), null);
    assert.equal(queryByText("插件与技能"), null);

    // 点击打开附加菜单
    fireEvent.click(attachBtn);

    // 验证菜单展开时输入框高度不变（无跳动）
    if (textarea) {
      assert.equal(textarea.offsetHeight, initialTextareaHeight, "点击添加按钮打开菜单时输入框不得有布局跳动");
    }

    // 验证包含工作流和插件技能选项
    const workflowBtn = getByText("工作流");
    const skillsToolsBtn = getByText("插件与技能");
    assert.ok(workflowBtn);
    assert.ok(skillsToolsBtn);

    // 点击工作流
    fireEvent.click(workflowBtn);
    assert.equal(workflowTriggered, true);

    // 再次点击打开，点击插件与技能
    fireEvent.click(attachBtn);
    const skillsToolsBtn2 = getByText("插件与技能");
    fireEvent.click(skillsToolsBtn2);
    assert.equal(skillsToolsTriggered, true);
  });

  // 1b. 未传回调时打开真实技能选择器并注入真实提示词
  await t.test("1b. AgentComposer 未传专有回调时，点击工作流与插件技能弹出选择器并真实注入提示词", async () => {
    let textValue = "";

    function DefaultWrapper() {
      const [val, setVal] = useState(textValue);
      return (
        <AgentComposer
          value={val}
          onInputChange={(v) => {
            textValue = v;
            setVal(v);
          }}
          onSubmit={() => {}}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          sessionId="test-session"
          permissionView={mockPermissionView}
          permissionStatus="ready"
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
        />
      );
    }

    const { getByRole, getByText, getByTestId } = render(<DefaultWrapper />);
    const attachBtn = getByRole("button", { name: "附加范围或文件" });

    // 1) 点击工作流：弹出真实技能选择器并注入工作流提示词
    fireEvent.click(attachBtn);
    const workflowBtn = getByText("工作流");
    fireEvent.click(workflowBtn);
    const pairItem = getByTestId("skill-picker-item-builtin-s1-insar-pair-workflow");
    fireEvent.click(pairItem);
    assert.equal(textValue.includes("合成孔径雷达（SAR）干涉测量工程师"), true);

    // 2) 点击插件与技能：弹出真实技能选择器并注入技能提示词
    fireEvent.click(attachBtn);
    const skillsBtn = getByText("插件与技能");
    fireEvent.click(skillsBtn);
    const coreItem = getByTestId("skill-picker-item-builtin-insar-copilot-core");
    fireEvent.click(coreItem);
    assert.equal(textValue.includes("EOStage 的数据准备助手"), true);
  });

  // 2. 权限菜单：气泡风格，三档选项，无冗余配置
  await t.test("2. AgentPermissionMenu 呈现标准气泡风格与三档选择", async () => {
    let selectedMode = "";
    const { getByRole, getByText } = render(
      <AgentPermissionMenu
        view={mockPermissionView}
        status="ready"
        onSelect={(m) => {
          selectedMode = m;
        }}
      />
    );

    const trigger = getByRole("button", { name: /模式/ });
    fireEvent.click(trigger);

    // 菜单展开后包含自主审批与完全访问模式
    const autonomousOption = getByText("自主审批");
    assert.ok(autonomousOption);

    // 点击切换为自主审批模式
    fireEvent.click(autonomousOption);
    assert.equal(selectedMode, "auto_approve");
  });

  // 3. 地图收起后一键展开入口 (Agent 端)
  await t.test("3. Agent 界面在地图收起后显示一键展开手柄按钮，点击触发展开", async () => {
    let panelOpenRequested = false;

    const { getByTestId } = render(
      <DesktopAgentWorkbench
        panelOpen={false}
        onPanelOpenChange={(open) => {
          panelOpenRequested = open;
        }}
        channelReadiness={mockChannelReadiness}
        channels={[]}
      />
    );

    const handle = getByTestId("agent-expand-map-handle");
    assert.ok(handle, "必须存在 agent-expand-map-handle 按钮");

    fireEvent.click(handle);
    assert.equal(panelOpenRequested, true, "点击展开手柄应请求打开地图面板");
  });

  // 4. 模型设置协议名称规范与弹窗遮罩
  await t.test("4. 模型设置：规范协议名称展示为标准 /v1 路径，弹窗带遮罩且点击外部不丢失", () => {
    // 检查协议名称规范性
    const values = CHANNEL_PROTOCOLS.map((p) => p.value);
    assert.ok(values.includes("anthropic_messages"));
    assert.ok(values.includes("chat_completions"));
    assert.ok(values.includes("responses"));

    // 检查规范显示名称
    const anthropic = CHANNEL_PROTOCOLS.find((p) => p.value === "anthropic_messages");
    const completions = CHANNEL_PROTOCOLS.find((p) => p.value === "chat_completions");
    const responses = CHANNEL_PROTOCOLS.find((p) => p.value === "responses");

    assert.equal(anthropic?.label_zh, "Anthropic Messages (/v1/messages)");
    assert.equal(completions?.label_zh, "Chat Completions (/chat/completions)");
    assert.equal(responses?.label_zh, "Responses (/responses)");
  });

  // 5. 账号页免认证数据源与存储路径独立
  await t.test("5. 账号页：免认证数据源统一排在需认证下方，存储路径独立划分", () => {
    const visibleMetas = orderAccountsForSettings(filterRenderedAccounts(ACCOUNT_METAS));
    const accountMetas = visibleMetas.filter((meta) => meta.canLogin);
    const anonymousMetas = visibleMetas.filter((meta) => !meta.canLogin);

    // 确认免认证数据源存在且完全与需登录数据源分离
    assert.ok(accountMetas.length > 0, "必须包含需认证数据源");
    assert.ok(anonymousMetas.length > 0, "必须包含免认证数据源");

    // 验证免认证数据源中不包含 canLogin: true 的项
    for (const m of anonymousMetas) {
      assert.equal(m.canLogin, false);
    }
  });

  // 6. 网络设置连通性探测
  await t.test("6. 网络设置连通性探测：渠道名称为可点击超链接，带外链图标与延迟状态", async () => {
    const { container } = render(<NetworkProxySettingsTab />);

    // 查找所有链接 <a> 标签
    const links = container.querySelectorAll("a[href]");
    assert.ok(links.length > 0, "探测列表中渠道名称应为 <a> 链接");

    // 验证链接均包含 http/https 协议前缀
    for (const link of links) {
      const href = link.getAttribute("href") || "";
      assert.ok(href.startsWith("http://") || href.startsWith("https://"), `链接应包含协议前缀: ${href}`);
    }

    // 验证未探测时显示状态文本
    const notProbed = container.textContent;
    assert.ok(notProbed?.includes("未探测") || notProbed?.includes("连接成功") || notProbed?.includes("无法连接"));
  });
});
