import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { projectEvidence, buildTaskCardModels } from "./agentConversationLogic.ts";
import type { StreamConclusionItem } from "./agentStreamLogic.ts";
import type { ApplicationTask } from "./types.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";
import { explainActionError } from "./errorExplanations.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-2 Retro Fixes Specification (R1-R3: Contrast, Font Size, Duplicate Banners)", async (t) => {
  const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
  const content = fs.readFileSync(workbenchPath, "utf-8");
  const conclusionCardPath = path.resolve(__dirname, "../../components/desktop/AgentConclusionCard.tsx");
  const conclusionCardContent = fs.readFileSync(conclusionCardPath, "utf-8");

  await t.test(
    "R2: no text-[9px], text-[10px], or text-[11px] (minimum font size rule: >= 12px)",
    () => {
      assert.doesNotMatch(
        content,
        /text-\[(9|10|11)px\]/,
        "DesktopAgentWorkbench.tsx must not contain font sizes below 12px (text-[9px], text-[10px], text-[11px])"
      );
      assert.doesNotMatch(
        conclusionCardContent,
        /text-\[(9|10|11)px\]/,
        "AgentConclusionCard.tsx must not contain font sizes below 12px (text-[9px], text-[10px], text-[11px])"
      );
    }
  );

  await t.test(
    "R1: no raw Tailwind palette backgrounds or borders (bg-red-, bg-blue-, bg-emerald-, bg-amber-, border-red-, border-blue-)",
    () => {
      assert.doesNotMatch(
        content,
        /(?:bg-(?:red|blue|emerald|amber)-|border-(?:red|blue)-)/,
        "DesktopAgentWorkbench.tsx must not use raw Tailwind palette colors (bg-red-, bg-blue-, bg-emerald-, bg-amber-, border-red-, border-blue-)"
      );
      assert.doesNotMatch(
        conclusionCardContent,
        /(?:bg-(?:red|blue|emerald|amber)-|border-(?:red|blue)-)/,
        "AgentConclusionCard.tsx must not use raw Tailwind palette colors (bg-red-, bg-blue-, bg-emerald-, bg-amber-, border-red-, border-blue-)"
      );
    }
  );

  await t.test(
    "R1: every line containing text-sp-status- must be an icon element with h-3 or h-4 (no status-dot color on text)",
    () => {
      const lines = [
        ...content.split(/\r?\n/),
        ...conclusionCardContent.split(/\r?\n/),
      ];
      const invalidStatusTextLines: { lineNum: number; line: string }[] = [];
      lines.forEach((line, idx) => {
        if (line.includes("text-sp-status-")) {
          if (!line.includes("h-3") && !line.includes("h-4")) {
            invalidStatusTextLines.push({ lineNum: idx + 1, line: line.trim() });
          }
        }
      });
      assert.strictEqual(
        invalidStatusTextLines.length,
        0,
        `Every line containing text-sp-status- must also contain h-3 or h-4 (icon sizing only). Found:\n` +
          invalidStatusTextLines.map((l) => `  L${l.lineNum}: ${l.line}`).join("\n")
      );
    }
  );

  await t.test(
    "R3: demoNotice and actionError banners must each be rendered exactly once (no duplicate inside conversation)",
    () => {
      const demoNoticeMatches = content.match(/\{\s*demoNotice\s*&&\s*\(/g) || [];
      assert.strictEqual(
        demoNoticeMatches.length,
        1,
        "DesktopAgentWorkbench.tsx must render {demoNotice && ( exactly once"
      );

      const actionErrorMatches = content.match(/\{\s*actionError\s*&&\s*\(/g) || [];
      assert.strictEqual(
        actionErrorMatches.length,
        1,
        "DesktopAgentWorkbench.tsx must render {actionError && ( exactly once"
      );
    }
  );
});

test("W5-2 Retro Fixes Specification (R4-R6: Evidence Status, Action Error Mapping, Terminology & Dead Code)", async (t) => {
  const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
  const content = fs.readFileSync(workbenchPath, "utf-8");
  const conclusionCardPath = path.resolve(__dirname, "../../components/desktop/AgentConclusionCard.tsx");
  const conclusionCardContent = fs.readFileSync(conclusionCardPath, "utf-8");

  await t.test("R4: projectEvidence status logic (uncited, unverified with refs, unverified with zero refs, verified)", () => {
    // 1. 无 claims 或 claims 为空 -> uncited
    const noClaimsItem: StreamConclusionItem = {
      kind: "conclusion",
      seq: 1,
      run_id: "run-001",
      text: "回答文本",
    };
    const emptyClaimsItem: StreamConclusionItem = {
      kind: "conclusion",
      seq: 2,
      run_id: "run-002",
      text: "回答文本",
      claims: [],
    };
    const ev1 = projectEvidence(noClaimsItem);
    const ev2 = projectEvidence(emptyClaimsItem);
    assert.strictEqual(ev1.status, "uncited", "no claims must yield status='uncited'");
    assert.strictEqual(ev1.verified, false);
    assert.strictEqual(ev2.status, "uncited", "empty claims must yield status='uncited'");
    assert.strictEqual(ev2.verified, false);

    // 2. 有 claims，有 refs，但存在 claim.verified === false -> unverified
    const unverifiedWithRefsItem: StreamConclusionItem = {
      kind: "conclusion",
      seq: 3,
      run_id: "run-003",
      text: "回答文本",
      claims: [
        { statement: "陈述 1", verified: true, refs: [{ kind: "task", id: "t1" }] },
        { statement: "陈述 2", verified: false, refs: [{ kind: "task", id: "t2" }] },
      ],
    };
    const ev3 = projectEvidence(unverifiedWithRefsItem);
    assert.strictEqual(ev3.status, "partial", "some cited claims are verified");
    assert.strictEqual(ev3.verified, false);
    assert.strictEqual(ev3.refCount, 2);

    // 3. 有 claims，即使 claim.verified === true 但无 refs (0 refs) -> unverified
    const unverifiedZeroRefsItem: StreamConclusionItem = {
      kind: "conclusion",
      seq: 4,
      run_id: "run-004",
      text: "回答文本",
      claims: [
        { statement: "陈述 1", verified: true, refs: [] },
      ],
    };
    const ev4 = projectEvidence(unverifiedZeroRefsItem);
    assert.strictEqual(ev4.status, "unverified", "claim without refs must yield status='unverified'");
    assert.strictEqual(ev4.verified, false);
    assert.strictEqual(ev4.refCount, 0);

    // 4. 有 claims，每个 claim 均 verified === true 且均有 >= 1 ref -> verified
    const verifiedItem: StreamConclusionItem = {
      kind: "conclusion",
      seq: 5,
      run_id: "run-005",
      text: "回答文本",
      claims: [
        { statement: "陈述 1", verified: true, refs: [{ kind: "batch", id: "b1" }] },
        { statement: "陈述 2", verified: true, refs: [{ kind: "web", id: "w1" }] },
      ],
    };
    const ev5 = projectEvidence(verifiedItem);
    assert.strictEqual(ev5.status, "verified", "all claims verified with >=1 ref must yield status='verified'");
    assert.strictEqual(ev5.verified, true);
    assert.strictEqual(ev5.refCount, 2);
  });

  await t.test("R5: explainActionError mapping & fallback logic", () => {
    const fallback = "操作失败，请重试";

    // 1. 已知错误码 -> 返回对应中文解释
    assert.strictEqual(
      explainActionError({ code: "model_timeout" }, fallback),
      "模型服务响应超时"
    );
    assert.strictEqual(
      explainActionError({ code: "AGENT_CHANNEL_KEY_MISSING" }, fallback),
      "未配置访问密钥"
    );

    // 2. 未知错误码 -> 返回 fallback
    assert.strictEqual(
      explainActionError({ code: "UNKNOWN_TAURI_ERROR_XYZ" }, fallback),
      fallback
    );

    // 3. 原生 Error 实例 -> 返回 fallback（绝不返回 err.message 英文泄露）
    const errorInst = new Error("Connection refused: 127.0.0.1:4000");
    assert.strictEqual(explainActionError(errorInst, fallback), fallback);

    // 4. 字符串错误 -> 返回 fallback
    assert.strictEqual(explainActionError("Network timeout string", fallback), fallback);

    // 5. 空值 / 非对象 -> 返回 fallback
    assert.strictEqual(explainActionError(null, fallback), fallback);
    assert.strictEqual(explainActionError(undefined, fallback), fallback);

    // 6. 仅含 message_key 或 context 无 code -> 返回 fallback
    assert.strictEqual(
      explainActionError({ message_key: "timeout_key", context: { foo: "bar" } }, fallback),
      fallback
    );
  });

  await t.test("R4-R6: DesktopAgentWorkbench.tsx source assertions", () => {
    // R4 结论溯源更新（Owner UX 决策 #6：彻底移除无意义的「未引用证据」占位框，仅当 projEvidence.status !== "uncited" 时渲染核验溯源）
    assert.doesNotMatch(
      conclusionCardContent,
      /agent\.conclusion\.uncited\b|未引用证据/,
      "AgentConclusionCard.tsx must not render redundant '未引用证据' placeholder"
    );
    assert.match(
      conclusionCardContent,
      /projEvidence\.status\s*!==\s*"uncited"/,
      "AgentConclusionCard.tsx must guard evidence box with projEvidence.status !== 'uncited'"
    );

    // 结论卡抽离至 AgentConclusionCard 且 DesktopAgentWorkbench 不再直接包含 ChannelReasoningNote
    assert.match(
      content,
      /<AgentConclusionCard\b/,
      "DesktopAgentWorkbench.tsx's case 'conclusion' must render <AgentConclusionCard"
    );
    assert.doesNotMatch(
      content,
      /<ChannelReasoningNote\b/,
      "DesktopAgentWorkbench.tsx must no longer directly contain <ChannelReasoningNote (it must appear only via AgentConclusionCard)"
    );

    // R5 不得使用 raw err?.message 或 err instanceof Error ? err.message
    assert.doesNotMatch(
      content,
      /err\?\.message/,
      "DesktopAgentWorkbench.tsx must not contain 'err?.message'"
    );
    assert.doesNotMatch(
      conclusionCardContent,
      /err\?\.message/,
      "AgentConclusionCard.tsx must not contain 'err?.message'"
    );
    assert.doesNotMatch(
      content,
      /\.message\s*:/,
      "DesktopAgentWorkbench.tsx must not contain '.message :'"
    );
    assert.doesNotMatch(
      conclusionCardContent,
      /\.message\s*:/,
      "AgentConclusionCard.tsx must not contain '.message :'"
    );

    // R6(a) 去除实现术语「桌面桥接」、「外壳」
    assert.doesNotMatch(
      content,
      /桌面桥接/,
      "DesktopAgentWorkbench.tsx must not contain '桌面桥接'"
    );
    assert.doesNotMatch(
      conclusionCardContent,
      /桌面桥接/,
      "AgentConclusionCard.tsx must not contain '桌面桥接'"
    );
    assert.doesNotMatch(
      content,
      /外壳/,
      "DesktopAgentWorkbench.tsx must not contain '外壳'"
    );
    assert.doesNotMatch(
      conclusionCardContent,
      /外壳/,
      "AgentConclusionCard.tsx must not contain '外壳'"
    );

    // R6(b) 占位文本去过度承诺
    assert.doesNotMatch(
      content,
      /自动分析并编排/,
      "DesktopAgentWorkbench.tsx must not contain '自动分析并编排'"
    );
    assert.doesNotMatch(
      conclusionCardContent,
      /自动分析并编排/,
      "AgentConclusionCard.tsx must not contain '自动分析并编排'"
    );

    // R6(c) 移除死代码 handleOpenRunTrace, openTraceDetail, 事件审计哈希链
    assert.doesNotMatch(
      content,
      /handleOpenRunTrace/,
      "DesktopAgentWorkbench.tsx must not contain dead code 'handleOpenRunTrace'"
    );
    assert.doesNotMatch(
      content,
      /openTraceDetail/,
      "DesktopAgentWorkbench.tsx must not contain dead state 'openTraceDetail'"
    );
    assert.doesNotMatch(
      content,
      /事件审计哈希链/,
      "DesktopAgentWorkbench.tsx must not contain dead modal text '事件审计哈希链'"
    );
  });
});

test("W5-2 Retro Fixes Specification (R7-R9: Map Panel Overlays, Empty Surface, Conversation Header Cleanup)", async (t) => {
  const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
  const content = fs.readFileSync(workbenchPath, "utf-8");

  await t.test("R7: DesktopAgentWorkbench.tsx does not contain map overlays, hint chip, or legend block", () => {
    assert.doesNotMatch(
      content,
      /点击足迹定位到结果清单/,
      "DesktopAgentWorkbench.tsx must not contain map hint '点击足迹定位到结果清单'"
    );
    assert.doesNotMatch(
      content,
      /map-hint/,
      "DesktopAgentWorkbench.tsx must not contain 'map-hint' class"
    );
    assert.doesNotMatch(
      content,
      /map-legend/,
      "DesktopAgentWorkbench.tsx must not contain 'map-legend' class"
    );
  });

  await t.test("R8: DesktopAgentWorkbench.tsx empty-state and root container use bg-sp-surface", () => {
    assert.match(
      content,
      /className="[^"]*agent-empty[^"]*bg-sp-surface[^"]*"/,
      "DesktopAgentWorkbench.tsx .agent-empty container must include bg-sp-surface"
    );
    assert.doesNotMatch(
      content,
      /<div className="[^"]*relative flex h-full w-full overflow-hidden[^"]*bg-sp-canvas/,
      "DesktopAgentWorkbench.tsx root container must no longer contain bg-sp-canvas"
    );
  });

  await t.test("R9: DesktopAgentWorkbench.tsx duplicate header removed and in-stream running row updated", () => {
    assert.doesNotMatch(
      content,
      /本地预览/,
      "DesktopAgentWorkbench.tsx must not contain '本地预览'"
    );
    assert.doesNotMatch(
      content,
      /任务进行中/,
      "DesktopAgentWorkbench.tsx must not contain '任务进行中'"
    );
    assert.doesNotMatch(
      content,
      /取消本次运行<\/span>/,
      "DesktopAgentWorkbench.tsx must not contain old header text '取消本次运行</span>'"
    );
    assert.doesNotMatch(
      content,
      /agent\.workbench\.runningIndicator/,
      "DesktopAgentWorkbench.tsx must not render duplicate runningIndicator bottom bar"
    );
    assert.doesNotMatch(
      content,
      /onStop=\{foldMeta\.isActive && activeRunId \? handleCancelRun : undefined\}/,
      "DesktopAgentWorkbench.tsx no longer renders a duplicate stop button on unified thinking/process header (Item 12)"
    );
    assert.match(
      content,
      /onStop=\{handleCancelRun\}/,
      "DesktopAgentWorkbench.tsx wires single morphing stop control onto AgentComposer"
    );
  });
});

test("W5-2 Retro Fixes Specification (R10-R11: Task Card Product Line, Fixture Marker in Titlebar)", async (t) => {
  const desktopWorkbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
  const desktopWorkbenchContent = fs.readFileSync(desktopWorkbenchPath, "utf-8");

  const desktopAgentWorkbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
  const desktopAgentWorkbenchContent = fs.readFileSync(desktopAgentWorkbenchPath, "utf-8");

  const desktopTitlebarPath = path.resolve(__dirname, "../../components/desktop/DesktopTitlebar.tsx");
  const desktopTitlebarContent = fs.readFileSync(desktopTitlebarPath, "utf-8");

  await t.test("R10: buildTaskCardModels productLine logic (empty -> null, 1 -> id, >1 -> first 等 N 景)", () => {
    // 基础 mock 任务模版
    const baseTask: ApplicationTask = {
      task_id: "task-001",
      name: "Sentinel-1 SLC 批次",
      status: "running",
      progress_percent: 50,
      done_bytes: 500,
      total_bytes: 1000,
      bytes_per_second: 50,
      output_dir: "/data",
    };

    // Case 1: scene_ids 为 undefined 或空数组 -> productLine 为 null
    const taskNoScenes: ApplicationTask = { ...baseTask, task_id: "t-none", scene_ids: [] };
    const taskUndefScenes: ApplicationTask = { ...baseTask, task_id: "t-undef", scene_ids: undefined };
    const m1 = buildTaskCardModels([{ id: "t-none" }], [taskNoScenes]);
    const m2 = buildTaskCardModels([{ id: "t-undef" }], [taskUndefScenes]);
    assert.strictEqual(m1[0]?.productLine, null, "empty scene_ids must yield productLine === null");
    assert.strictEqual(m2[0]?.productLine, null, "undefined scene_ids must yield productLine === null");

    // Case 2: 恰好 1 景 -> 该景完整 ID
    const taskSingleScene: ApplicationTask = {
      ...baseTask,
      task_id: "t-single",
      scene_ids: ["S1A_IW_SLC__1SDV_20240501T103000_20240501T103027_053676_0682BD_C726"],
    };
    const m3 = buildTaskCardModels([{ id: "t-single" }], [taskSingleScene]);
    assert.strictEqual(
      m3[0]?.productLine,
      "S1A_IW_SLC__1SDV_20240501T103000_20240501T103027_053676_0682BD_C726",
      "single scene must yield exactly that scene id"
    );

    // Case 3: 超过 1 景 -> `${first} 等 ${n} 景`
    const taskMultiScenes: ApplicationTask = {
      ...baseTask,
      task_id: "t-multi",
      scene_ids: [
        "S1A_IW_SLC__1SDV_20240501T103000_20240501T103027_053676_0682BD_C726",
        "S1A_IW_SLC__1SDV_20240513T103000_20240513T103027_053851_0688F5_1234",
        "S1A_IW_SLC__1SDV_20240525T103000_20240525T103027_054026_068EFB_5678",
      ],
    };
    const m4 = buildTaskCardModels([{ id: "t-multi" }], [taskMultiScenes]);
    assert.strictEqual(
      m4[0]?.productLine,
      "S1A_IW_SLC__1SDV_20240501T103000_20240501T103027_053676_0682BD_C726 等 3 景",
      "multiple scenes must yield '${first} 等 ${n} 景'"
    );
  });

  await t.test("R10-R11: source assertions on DesktopWorkbench.tsx and DesktopAgentWorkbench.tsx", () => {
    // DesktopWorkbench.tsx: DEV 演示数据角标并入标题栏 leadingBadge，无 text-[11px]，无 fixed top-2.5 left-32
    assert.match(
      desktopWorkbenchContent,
      /leadingBadge=/,
      "DesktopWorkbench.tsx must pass leadingBadge to DesktopTitlebar"
    );
    assert.doesNotMatch(
      desktopWorkbenchContent,
      /fixed top-2\.5 left-32/,
      "DesktopWorkbench.tsx must no longer contain 'fixed top-2.5 left-32'"
    );
    assert.match(
      desktopTitlebarContent,
      /\bleadingBadge\b/,
      "DesktopTitlebar.tsx must declare and render leadingBadge"
    );
    assert.match(
      desktopTitlebarContent,
      /grid-cols-\[minmax\(max-content,1fr\)_minmax\(0,auto\)_minmax\(max-content,1fr\)\]/,
      "DesktopTitlebar.tsx header grid must use grid-cols-[minmax(max-content,1fr)_minmax(0,auto)_minmax(max-content,1fr)]"
    );
    assert.doesNotMatch(
      desktopWorkbenchContent,
      /text-\[11px\]/,
      "DesktopWorkbench.tsx must not contain font size text-[11px]"
    );

    // DesktopAgentWorkbench.tsx: 不包含「底层执行」，不包含 scene_ids[0]) || m.task.name
    assert.doesNotMatch(
      desktopAgentWorkbenchContent,
      /底层执行/,
      "DesktopAgentWorkbench.tsx must not contain '底层执行'"
    );
    assert.doesNotMatch(
      desktopAgentWorkbenchContent,
      /scene_ids\[0\]\)\s*\|\|\s*m\.task\.name/,
      "DesktopAgentWorkbench.tsx must not contain fallback 'scene_ids[0]) || m.task.name'"
    );
  });
});

test("W5-2e Specification (E1: Harness Scenes & AgentHistoryPage Implementation Term Clean)", async (t) => {
  const harnessPath = path.resolve(__dirname, "../../components/desktop/DesktopHarness.tsx");
  const harnessContent = fs.readFileSync(harnessPath, "utf-8");

  const historyPagePath = path.resolve(__dirname, "../../components/desktop/AgentHistoryPage.tsx");
  const historyPageContent = fs.readFileSync(historyPagePath, "utf-8");

  await t.test("E1a: DesktopHarness.tsx contains 'conversation-full' and '<AgentHistoryPage'", () => {
    assert.match(
      harnessContent,
      /"conversation-full"/,
      "DesktopHarness.tsx must contain subscene 'conversation-full'"
    );
    assert.match(
      harnessContent,
      /<AgentHistoryPage/,
      "DesktopHarness.tsx must render <AgentHistoryPage"
    );
  });

  await t.test("E1b: AgentHistoryPage.tsx does not contain '桥接'", () => {
    assert.doesNotMatch(
      historyPageContent,
      /桥接/,
      "AgentHistoryPage.tsx must not contain implementation term '桥接'"
    );
  });
});

test("W5-2e Specification (E2: Titlebar Panel Toggle, Edge Handle Z-Index, Scroll Bottom Button Placement)", async (t) => {
  const titlebarPath = path.resolve(__dirname, "../../components/desktop/DesktopTitlebar.tsx");
  const titlebarContent = fs.readFileSync(titlebarPath, "utf-8");

  const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
  const workbenchContent = fs.readFileSync(workbenchPath, "utf-8");

  await t.test("E2a: DesktopTitlebar.tsx panel toggle className contains whitespace-nowrap and shrink-0", () => {
    assert.match(
      titlebarContent,
      /panelToggle[\s\S]*?className=\{cn\([\s\S]*?whitespace-nowrap/,
      "DesktopTitlebar.tsx panel toggle className must contain 'whitespace-nowrap'"
    );
    assert.match(
      titlebarContent,
      /panelToggle[\s\S]*?className=\{cn\([\s\S]*?shrink-0/,
      "DesktopTitlebar.tsx panel toggle className must contain 'shrink-0'"
    );
  });

  await t.test("E2b: DesktopAgentWorkbench.tsx handle className contains z-[1001] and no longer z-10 w-6 h-12", () => {
    assert.match(
      workbenchContent,
      /handle[\s\S]*?z-\[1001\]/,
      "DesktopAgentWorkbench.tsx handle className must contain 'z-[1001]'"
    );
    assert.doesNotMatch(
      workbenchContent,
      /z-10 w-6 h-12/,
      "DesktopAgentWorkbench.tsx handle className must no longer contain 'z-10 w-6 h-12'"
    );
  });

  await t.test("E2c: DesktopAgentWorkbench.tsx 跳至最新 container no longer contains bottom-20 right-8", () => {
    assert.doesNotMatch(
      workbenchContent,
      /bottom-20 right-8/,
      "DesktopAgentWorkbench.tsx 跳至最新 container must no longer contain 'bottom-20 right-8'"
    );
    assert.match(
      workbenchContent,
      /bottom-4 right-6/,
      "DesktopAgentWorkbench.tsx 跳至最新 container must contain 'bottom-4 right-6'"
    );
  });
});

test("W5-2e Specification (E3: Dark Theme Danger Token Contrast Fix)", async (t) => {
  const cssPath = path.resolve(__dirname, "../../index.css");
  const cssContent = fs.readFileSync(cssPath, "utf-8");

  const rootMatch = cssContent.match(/:root\s*\{([\s\S]*?)\n\}/);
  const darkMatch = cssContent.match(/\.dark\s*\{([\s\S]*?)\n\}/);

  assert.ok(rootMatch, ":root block must exist in index.css");
  assert.ok(darkMatch, ".dark block must exist in index.css");

  const rootBlock = rootMatch ? rootMatch[1] : "";
  const darkBlock = darkMatch ? darkMatch[1] : "";

  await t.test("E3a: :root block retains --sp-danger-text: #b42318", () => {
    assert.match(
      rootBlock,
      /--sp-danger-text:\s*#b42318;/,
      ":root block must retain '--sp-danger-text: #b42318;'"
    );
  });

  await t.test("E3b: .dark block contains --sp-danger-text: #ff7b72 and --sp-danger-strong: #d92d20", () => {
    assert.match(
      darkBlock,
      /--sp-danger-text:\s*#ff7b72;/,
      ".dark block must contain '--sp-danger-text: #ff7b72;'"
    );
    assert.match(
      darkBlock,
      /--sp-danger-strong:\s*#d92d20;/,
      ".dark block must contain '--sp-danger-strong: #d92d20;'"
    );
  });
});

test("Owner §8.1 accent fill token", async (t) => {
  const cssPath = path.resolve(__dirname, "../../index.css");
  const cssContent = fs.readFileSync(cssPath, "utf-8");

  const rootMatch = cssContent.match(/:root\s*\{([\s\S]*?)\n\}/);
  const darkMatch = cssContent.match(/\.dark\s*\{([\s\S]*?)\n\}/);
  const themeMatch = cssContent.match(/@theme\s+inline\s*\{([\s\S]*?)\n\}/);

  const rootBlock = rootMatch ? rootMatch[1] : "";
  const darkBlock = darkMatch ? darkMatch[1] : "";
  const themeBlock = themeMatch ? themeMatch[1] : "";

  await t.test("index.css token definitions for --sp-accent-fill", () => {
    assert.match(
      rootBlock,
      /--sp-accent-fill:\s*#0066d6;/,
      ":root block must contain '--sp-accent-fill: #0066d6;'"
    );
    // Owner 2026-11-12 B4 decision: EOStage dark mode uses crisp light fill (#f4f4f5) + dark ink (--sp-accent-ink: #09090b)
    // instead of bright blue (#0a6fd6) for primary action buttons.
    assert.match(
      darkBlock,
      /--sp-accent-fill:\s*#f4f4f5;/,
      ".dark block must contain '--sp-accent-fill: #f4f4f5;' (EOStage dark primary fill)"
    );
    assert.match(
      darkBlock,
      /--sp-accent-ink:\s*#09090b;/,
      ".dark block must contain '--sp-accent-ink: #09090b;'"
    );
    assert.match(
      themeBlock,
      /--color-sp-accent-fill:\s*var\(--sp-accent-fill\);/,
      "@theme inline block must contain '--color-sp-accent-fill: var(--sp-accent-fill);'"
    );
  });

  const buttonFiles = [
    "../../components/desktop/AgentComposer.tsx",
    "../../components/desktop/DesktopAgentWorkbench.tsx",
    "../../components/desktop/DesktopAssistantWidget.tsx",
    "../../components/desktop/AdminBoundaryModal.tsx",
    "../components/settings/AccountsSettingsTab.tsx",
    "../../components/desktop/DesktopHarness.tsx",
  ];

  await t.test("listed button files must use bg-sp-accent-fill and no longer use bg-sp-accent-strong or bg-sp-accent with text-white", () => {
    for (const relPath of buttonFiles) {
      const fullPath = path.resolve(__dirname, relPath);
      const content = fs.readFileSync(fullPath, "utf-8");

      assert.doesNotMatch(
        content,
        /bg-sp-accent-strong\s+text-white/,
        `${relPath} must not contain 'bg-sp-accent-strong text-white'`
      );
      assert.doesNotMatch(
        content,
        /bg-sp-accent-strong\s+px-3\s+text-xs\s+font-semibold\s+text-white/,
        `${relPath} must not contain 'bg-sp-accent-strong px-3 text-xs font-semibold text-white'`
      );
      assert.doesNotMatch(
        content,
        /"bg-sp-accent\s+text-white"/,
        `${relPath} must not contain '"bg-sp-accent text-white"'`
      );
      assert.match(
        content,
        /\bbg-sp-accent-fill\b/,
        `${relPath} must contain 'bg-sp-accent-fill'`
      );
    }
  });
});
