import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { groupSessionsByDay } from "./agentHistoryLogic.ts";
import { projectConfirmCard } from "./agentConversationLogic.ts";
import {
  createAgentSessionMemory,
  resolveAgentRailNavigation,
} from "./agentSessionMemory.ts";
import type {
  AgentSessionSummary,
  StreamConfirmCardItem,
} from "../../lib/generated/v3-contracts.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-8 R4 (条目 6+7): 会话标题保留用户原文与返回对话页恢复原会话测试套件", async (suite) => {
  await suite.test(
    "1. (条目 6 / L4 P2-7) groupSessionsByDay 必须逐字节保留会话标题用户原文（「升轨」不改写为 Ascending），条件徽章仍使用 formatOrbitTerms",
    () => {
      const now = new Date("2026-10-27T08:00:00Z");
      const sessions: AgentSessionSummary[] = [
        {
          session_id: "sess-p2-7-a",
          title: "茂县叠溪升轨（Ascending）数据",
          created_at: "2026-10-27T07:30:00Z",
          updated_at: "2026-10-27T07:30:00Z",
          run_ids: [],
        },
        {
          session_id: "sess-p2-7-b",
          title: "台湾花莲 ALOS-2 升轨数据检索",
          created_at: "2026-10-27T07:00:00Z",
          updated_at: "2026-10-27T07:00:00Z",
          run_ids: [],
        },
      ];

      const groups = groupSessionsByDay(sessions, now);
      assert.strictEqual(groups.length, 1);
      assert.strictEqual(
        groups[0].items[0].title,
        "茂县叠溪升轨（Ascending）数据",
        "会话标题必须保留用户原文「茂县叠溪升轨（Ascending）数据」，严禁改写为「Ascending（Ascending）」"
      );
      assert.strictEqual(
        groups[0].items[1].title,
        "台湾花莲 ALOS-2 升轨数据检索",
        "会话标题中的「升轨」必须原封不动保留"
      );

      // 确认卡条件徽章仍保持英文轨道词（简报：术语英文化只用于徽章/条件）
      const confirmItem: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 1,
        run_id: "run-1",
        plan_id: "plan-1",
        plan_hash: "hash-1",
        snapshot: {
          source: "Sentinel-1",
          range: {
            aoi_name: "茂县叠溪",
            area_km2: 120,
            time_range: { start: "2026-07-26", end: "2026-10-26" },
            product: "SLC · 升轨",
          },
          estimated_bytes: null,
          output_dir: "D:/downloads",
        },
      };
      const projected = projectConfirmCard(confirmItem, null);
      assert.ok(
        projected.conditionBadges.includes("SLC · Ascending"),
        "确认卡条件徽章仍应将轨道词规范化为英文 Ascending"
      );

      // 源码断言：DesktopAgentWorkbench.tsx 的 onActiveSessionTitleChange 不得套用 formatOrbitTerms
      const agentWbSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
        "utf-8"
      );
      assert.doesNotMatch(
        agentWbSrc,
        /onActiveSessionTitleChange\?\.\([\s\S]{0,120}formatOrbitTerms/,
        "DesktopAgentWorkbench.tsx 的 onActiveSessionTitleChange 不得对会话标题调用 formatOrbitTerms"
      );
    }
  );

  await suite.test(
    "2. (条目 7 / L4 P2-6) createAgentSessionMemory: remember / recall / clear 与空值处理",
    () => {
      const memory = createAgentSessionMemory();
      assert.strictEqual(memory.recall(), null, "初始状态应为 null");

      memory.remember("sess-maoxian-01");
      assert.strictEqual(
        memory.recall(),
        "sess-maoxian-01",
        "remember 后 recall 应返回记住的会话 ID"
      );

      memory.remember("  sess-maoxian-02  ");
      assert.strictEqual(
        memory.recall(),
        "sess-maoxian-02",
        "remember 应去除首尾空白"
      );

      // 传入空串或 null 视为清空当前活动会话（如点击新对话）
      memory.remember("");
      assert.strictEqual(memory.recall(), null, "空字符串应归一化为 null");

      memory.remember("sess-maoxian-03");
      assert.strictEqual(memory.recall(), "sess-maoxian-03");
      memory.clear();
      assert.strictEqual(memory.recall(), null, "clear() 后 recall 应返回 null");

      memory.remember("sess-maoxian-04");
      memory.remember(null);
      assert.strictEqual(memory.recall(), null, "remember(null) 后 recall 应返回 null");
    }
  );

  await suite.test(
    "3. (条目 7 R4b) resolveAgentRailNavigation: 覆盖 downloads/history/workbench/agent 各情形",
    () => {
      assert.deepStrictEqual(
        resolveAgentRailNavigation({
          currentPageRoute: "downloads",
          rememberedSessionId: "sess-d-01",
        }),
        { kind: "restore", sessionId: "sess-d-01" },
        "downloads + 有记忆 -> restore"
      );

      assert.deepStrictEqual(
        resolveAgentRailNavigation({
          currentPageRoute: "history",
          rememberedSessionId: "sess-h-02",
        }),
        { kind: "restore", sessionId: "sess-h-02" },
        "history + 有记忆 -> restore"
      );

      assert.deepStrictEqual(
        resolveAgentRailNavigation({
          currentPageRoute: "workbench",
          rememberedSessionId: null,
        }),
        { kind: "open_empty" },
        "workbench + 无记忆 -> open_empty"
      );

      assert.deepStrictEqual(
        resolveAgentRailNavigation({
          currentPageRoute: "agent",
          rememberedSessionId: "sess-a-03",
        }),
        { kind: "stay" },
        "agent + 有记忆 -> stay"
      );

      assert.deepStrictEqual(
        resolveAgentRailNavigation({
          currentPageRoute: "agent",
          rememberedSessionId: null,
        }),
        { kind: "stay" },
        "agent + 无记忆 -> stay"
      );
    }
  );

  await suite.test(
    "4. (条目 7 R4b 源码接线断言) 活动栏改名「对话」、handleRailNavigate 不清空会话记忆、标题栏提供 newChat 按钮",
    () => {
      const railSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopActivityRail.tsx"),
        "utf-8"
      );
      const historyPageSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/AgentHistoryPage.tsx"),
        "utf-8"
      );
      const titlebarSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopTitlebar.tsx"),
        "utf-8"
      );
      const agentWbSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
        "utf-8"
      );
      const shellWbSrc = fs.readFileSync(
        path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx"),
        "utf-8"
      );

      // Owner 2026-10-29: 对话/历史与新对话和最近对话重复，侧栏去掉这两项
      assertLocalizedSource(railSrc, "shell.nav.mapWorkbench", "地图工作台");
      assert.match(
        railSrc,
        /\{\s*route:\s*"workbench",\s*labelKey:\s*"shell\.nav\.mapWorkbench",\s*icon:\s*Map\s*\}/,
        "DesktopActivityRail.tsx 的 NAV_ITEMS 中必须保留地图工作台项"
      );
      assertLocalizedSource(railSrc, "shell.nav.downloads", "下载中心");
      assert.match(
        railSrc,
        /\{\s*route:\s*"downloads",\s*labelKey:\s*"shell\.nav\.downloads",\s*icon:\s*Download\s*\}/,
        "DesktopActivityRail.tsx 的 NAV_ITEMS 中必须保留下载中心项"
      );

      // 历史页空态文案改为「在「对话」里描述…」
      assertLocalizedSource(
        historyPageSrc,
        "agent.history.emptyDescription",
        "在「对话」里描述需要的数据，会话会保存在这里。"
      );

      // DesktopTitlebar 声明 newChat 可选 prop
      assert.match(
        titlebarSrc,
        /newChat\?\s*:\s*\{\s*enabled:\s*boolean;\s*onClick:\s*\(\)\s*=>\s*void\s*\}/,
        "DesktopTitlebarProps 必须声明可选 newChat prop"
      );

      // DesktopAgentWorkbench 上报 onActiveSessionChange
      assert.match(
        agentWbSrc,
        /onActiveSessionChange\?\s*:\s*\(sessionId:\s*string\s*\|\s*null\)\s*=>\s*void/,
        "DesktopAgentWorkbenchProps 必须声明可选回调 onActiveSessionChange"
      );
      // W5-8 R9：上报移入 useAgentActiveSession（只在活动会话真正变化时回调）
      assert.match(
        agentWbSrc,
        /useAgentActiveSession\(\{[\s\S]{0,200}?onActiveSessionChange,/,
        "DesktopAgentWorkbench 必须把 onActiveSessionChange 交给 useAgentActiveSession 上报活动会话变更"
      );

      // DesktopWorkbench 调用 resolveAgentRailNavigation，且 handleRailNavigate 体内不含 clear() 或 setNewChatNonce
      const railNavStart = shellWbSrc.indexOf("const handleRailNavigate =");
      assert.ok(railNavStart >= 0, "应在 DesktopWorkbench.tsx 中找到 handleRailNavigate");
      const railNavEnd = shellWbSrc.indexOf("const [scientificMode", railNavStart);
      const railNavSlice = shellWbSrc.slice(
        railNavStart,
        railNavEnd > railNavStart ? railNavEnd : railNavStart + 800
      );
      assert.match(
        railNavSlice,
        /resolveAgentRailNavigation\(/,
        "handleRailNavigate 必须调用 resolveAgentRailNavigation"
      );
      assert.doesNotMatch(
        railNavSlice,
        /agentSessionMemory\.clear\(\)/,
        "handleRailNavigate 函数体内不得调用 agentSessionMemory.clear()"
      );
      assert.doesNotMatch(
        railNavSlice,
        /setNewChatNonce\(/,
        "handleRailNavigate 函数体内不得调用 setNewChatNonce"
      );

      assert.match(
        shellWbSrc,
        /openSessionId=\{agentOpenSessionId\s*\?\?\s*agentSessionMemory\.recall\(\)\}/,
        "DesktopWorkbench 挂载 DesktopAgentWorkbench 时必须以 agentOpenSessionId ?? agentSessionMemory.recall() 传入 openSessionId"
      );
    }
  );
});
