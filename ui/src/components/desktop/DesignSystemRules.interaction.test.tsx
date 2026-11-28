import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import React, { useState } from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentComposer } from "./AgentComposer";
import { AgentPlanCard } from "./AgentPlanCard";
import { AgentRunListStrip } from "./AgentRunListStrip";
import { ChatTokenizedText } from "./agent/ChatTokenizedText";
import {
  BUILTIN_SKILLS,
} from "../../lib/agentSkills";
import {
  extractSelectedSkillsFromPrompt,
  reconstructPromptWithSkills,
  tokenizeChatEntities,
} from "./agent/chatEntityTokens";
import type {
  AgentPermissionModeView,
  AgentRunSummary,
} from "../../lib/generated/v3-contracts";
import type { StreamPlanCardItem } from "../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

const mockPermissionView: AgentPermissionModeView = {
  effective: "confirm_changes",
  session_override: null,
  global_default: "confirm_changes",
};

const cssPath = path.resolve(import.meta.dirname, "../../index.css");

function ControlledComposerHarness({
  initialValue = "",
  onLastValue,
}: {
  initialValue?: string;
  onLastValue?: (val: string) => void;
}) {
  const [val, setVal] = useState(initialValue);
  return (
    <AgentComposer
      value={val}
      onInputChange={(next) => {
        setVal(next);
        onLastValue?.(next);
      }}
      onSubmit={() => {}}
      channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
      channels={[]}
      sessionId={null}
      permissionView={mockPermissionView}
      permissionStatus="ready"
      onPermissionSelect={() => {}}
      onPermissionSetGlobal={() => {}}
    />
  );
}

test("EOStage UI 设计规范回归验证套件（同心圆角、固定控件尺寸、角色态、内联芯片、自适应网格）", async (suite) => {
  await suite.test(
    "1. index.css 同心圆几何、固定控件、三列/双列网格、语义色令牌与前沿动效曲线存在且无硬编码芯片深色覆盖",
    () => {
      const css = fs.readFileSync(cssPath, "utf-8");
      assert.match(css, /\.eo-concentric-band\s*\{[^}]*border-radius:\s*16px;[^}]*padding:\s*3px;/s);
      assert.match(css, /\.eo-concentric-inner\s*\{[^}]*border-radius:\s*13px;/s);
      assert.match(css, /\.eo-segmented-pill\s*\{[^}]*height:\s*28px;/s);
      assert.match(css, /\.eo-settings-btn-primary\s*\{[^}]*height:\s*32px;[^}]*white-space:\s*nowrap;/s);
      assert.match(css, /\.sp-row-compact\s*\{[^}]*flex-shrink:\s*0;/s);
      assert.match(css, /\.sp-row-balanced\s*\{[^}]*flex-shrink:\s*0;/s);
      assert.match(css, /\.sp-grid-cards\s*\{[^}]*repeat\(auto-fill,\s*minmax\(min\(232px,\s*100%\),\s*1fr\)\)/s);
      assert.match(css, /\.sp-grid-cards-md\s*\{[^}]*repeat\(auto-fill,\s*minmax\(min\(280px,\s*100%\),\s*1fr\)\)/s);
      assert.match(css, /\.sp-grid-cards-2col\s*\{[^}]*repeat\(auto-fit,\s*minmax\(min\(184px,\s*100%\),\s*1fr\)\)/s);
      assert.match(css, /--sp-ease-out-expo:\s*cubic-bezier\(0\.16,\s*1,\s*0\.3,\s*1\);/);
      assert.match(css, /--sp-token-skill-bg:/);
      assert.match(css, /--sp-token-dataset-bg:/);
      assert.match(css, /--sp-token-aoi-bg:/);
      assert.match(css, /\.sp-chat-token-chip--skill\s*\{[^}]*var\(--sp-token-skill-bg\)/s);
      assert.equal(css.includes(".dark .sp-chat-token-chip--dataset"), false);
    }
  );

  await suite.test(
    "2. AgentComposer 工具按钮与碳黑发送按钮统一为 h-7 w-7 rounded-[7px] 且保留纯黑填充",
    () => {
      const view = render(<ControlledComposerHarness initialValue="检索卫星数据" />);
      const attachBtn = view.getByLabelText("附加范围或文件");
      const slashBtn = view.getByLabelText("选择技能与工作流");
      const sendBtn = view.getByTestId("agent-send-btn");

      assert.ok(attachBtn.className.includes("h-7") && attachBtn.className.includes("w-7"));
      assert.ok(slashBtn.className.includes("h-7") && slashBtn.className.includes("w-7"));
      assert.ok(sendBtn.className.includes("h-7") && sendBtn.className.includes("w-7"));
      assert.ok(sendBtn.className.includes("rounded-[7px]"));
      assert.ok(
        sendBtn.className.includes("bg-[#18181b]") ||
          sendBtn.className.includes("bg-sp-accent-fill")
      );
    }
  );

  await suite.test(
    "3. 选择技能拆分为 /skill 标签，不向 textarea 倾倒长提示词，同时保持完整回调字符串",
    () => {
      let latestWireValue = "";
      const view = render(
        <ControlledComposerHarness
          initialValue="请处理西安区域"
          onLastValue={(v) => {
            latestWireValue = v;
          }}
        />
      );

      const slashBtn = view.getByLabelText("选择技能与工作流");
      fireEvent.click(slashBtn);
      const item = view.getByTestId("skill-picker-item-radar-satellite-analysis");
      fireEvent.click(item);

      const chip = view.getByTestId("composer-skill-chip-radar-satellite-analysis");
      assert.ok(chip.textContent?.includes("SAR 与雷达卫星分析"));
      assert.equal(chip.textContent?.trim().startsWith("/"), false, "skill chip must not show leading /");

      const textarea = view.container.querySelector("textarea") as HTMLTextAreaElement;
      assert.equal(textarea.value, "请处理西安区域");
      assert.ok(latestWireValue.includes("Radar & SAR Satellite Analysis"));
      assert.ok(latestWireValue.startsWith("请处理西安区域\n\n"));

      // 点击芯片移除按钮恢复纯文本
      const removeBtn = chip.querySelector("button")!;
      fireEvent.click(removeBtn);
      assert.ok(view.queryByTestId("composer-skill-chip-radar-satellite-analysis") === null);
      assert.equal(latestWireValue, "请处理西安区域");

      // 在输入框中输入 `/insar` 自动弹出技能选择器，并校验左对齐与不压缩词条描述
      fireEvent.change(textarea, { target: { value: "请运行 /insar" } });
      const picker = view.getByTestId("agent-skill-picker");
      assert.ok(picker);
      assert.ok(picker.className.includes("-left-[15px]"));
      const firstItem = picker.querySelector('[data-testid^="skill-picker-item-"]') as HTMLElement;
      assert.ok(firstItem);
      assert.ok(firstItem.className.includes("shrink-0"));
      assert.equal(firstItem.className.includes("sp-row-balanced"), false);
    }
  );

  await suite.test(
    "4. ChatTokenizedText 无损渲染 /skill、@dataset 与 #AOI 芯片，不误拆普通路径",
    () => {
      const sample = "使用 /s1-insar 检索 @Sentinel-1 数据覆盖 #茂县叠溪镇 区域，保存在 /data/sar/out";
      const segs = tokenizeChatEntities(sample);
      assert.equal(segs.map((s) => s.value).join(""), sample);
      assert.ok(segs.some((s) => s.kind === "skill" && s.value === "/s1-insar"));
      assert.ok(segs.some((s) => s.kind === "dataset" && s.value === "@Sentinel-1"));
      assert.ok(segs.some((s) => s.kind === "aoi" && s.value === "#茂县叠溪镇"));
      assert.ok(!segs.some((s) => s.value === "/data"));

      const skill = BUILTIN_SKILLS[0];
      const extracted = extractSelectedSkillsFromPrompt(
        reconstructPromptWithSkills("分析 @Landsat #成都", [skill]),
        BUILTIN_SKILLS
      );
      assert.equal(extracted.selectedSkills.length, 1);
      assert.equal(extracted.cleanText, "分析 @Landsat #成都");

      const view = render(<ChatTokenizedText text={sample} />);
      assert.equal(view.container.textContent, sample);
      assert.ok(view.container.querySelector('[data-token-kind="dataset"]'));
      assert.ok(view.container.querySelector('[data-token-kind="aoi"]'));
      assert.ok(view.container.querySelector('[data-token-kind="skill"]'));

      const selectedView = render(
        <ChatTokenizedText
          text={reconstructPromptWithSkills("分析 @Landsat #成都", [skill])}
          skills={BUILTIN_SKILLS}
        />,
      );
      const selectedChip = selectedView.container.querySelector('[data-token-kind="skill"]');
      assert.ok(selectedChip);
      assert.equal(selectedChip!.textContent?.trim(), skill.name);
      assert.equal(selectedChip!.textContent?.trim().startsWith("/"), false);
    }
  );

  await suite.test(
    "5. AI Agent 活动态统一使用 .timeline-quiet-dots 而非旋转 Loader2",
    () => {
      const planItem: StreamPlanCardItem = {
        kind: "plan_card",
        seq: 12,
        run_id: "run-1",
        plan_id: "plan-1",
        plan_hash: "hash-1",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 SLC 影像",
            tier: 1,
            status: "running",
          },
        ],
      };
      const planView = render(
        <AgentPlanCard item={planItem} progressStatus="running" />
      );
      assert.ok(planView.getByTestId("plan-step-quiet-dots"));
      assert.equal(planView.container.querySelector(".animate-spin"), null);

      cleanup();

      const runningRuns: AgentRunSummary[] = [
        {
          run_id: "run-active",
          session_id: "sess-1",
          status: "running",
          started_at: "2026-11-14 10:00:00",
          ended_at: null,
          model_route: null,
        },
      ];
      const stripView = render(<AgentRunListStrip runs={runningRuns} />);
      assert.ok(stripView.getByTestId("run-strip-quiet-dots"));
      assert.equal(stripView.container.querySelector(".animate-spin"), null);
    }
  );
});
