import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

import type { AgentSessionSummary } from "../../lib/generated/v3-contracts.ts";
import {
  groupSessionsByDay,
  formatHistoryTime,
  historyTitle,
  sessionTitleFromRequest,
  resolveActiveSession,
  nextHistoryDeleteState,
  HISTORY_DELETE_CONFIRM_TEXT,
  type HistoryDeleteState,
} from "./agentHistoryLogic.ts";

function makeSession(
  sessionId: string,
  updatedAt: string,
  title: string = "测试会话"
): AgentSessionSummary {
  return {
    session_id: sessionId,
    title,
    created_at: updatedAt,
    updated_at: updatedAt,
    run_ids: [],
  };
}

test("W5-2d Agent History Logic Specification", async (t) => {
  // 1. groupSessionsByDay
  await t.test(
    "groupSessionsByDay: groups sessions into today, yesterday, earlier in order and sorts descending within group",
    () => {
      const now = new Date(2026, 9, 25, 15, 0); // 2026-10-25 15:00 local time
      const todayNewer = makeSession("s-today-2", new Date(2026, 9, 25, 14, 30).toISOString(), "今天较新");
      const todayOlder = makeSession("s-today-1", new Date(2026, 9, 25, 9, 15).toISOString(), "今天较旧");
      const yesterdayNewer = makeSession("s-yest-2", new Date(2026, 9, 24, 23, 10).toISOString(), "昨天深夜");
      const yesterdayOlder = makeSession("s-yest-1", new Date(2026, 9, 24, 8, 5).toISOString(), "昨天早晨");
      const earlierNewer = makeSession("s-earlier-2", new Date(2026, 9, 20, 16, 45).toISOString(), "更早几天前");
      const earlierOlder = makeSession("s-earlier-1", new Date(2025, 11, 30, 10, 0).toISOString(), "去年旧会话");

      // Pass in mixed order
      const input = [earlierOlder, todayOlder, yesterdayNewer, earlierNewer, todayNewer, yesterdayOlder];
      const groups = groupSessionsByDay(input, now);

      assert.strictEqual(groups.length, 3);
      assert.strictEqual(groups[0].key, "today");
      assert.strictEqual(groups[0].label, "今天");
      assert.deepStrictEqual(
        groups[0].items.map((i) => i.sessionId),
        ["s-today-2", "s-today-1"]
      );

      assert.strictEqual(groups[1].key, "yesterday");
      assert.strictEqual(groups[1].label, "昨天");
      assert.deepStrictEqual(
        groups[1].items.map((i) => i.sessionId),
        ["s-yest-2", "s-yest-1"]
      );

      assert.strictEqual(groups[2].key, "earlier");
      assert.strictEqual(groups[2].label, "更早");
      assert.deepStrictEqual(
        groups[2].items.map((i) => i.sessionId),
        ["s-earlier-2", "s-earlier-1"]
      );
    }
  );

  await t.test("groupSessionsByDay: omits empty groups and returns [] for empty input", () => {
    const now = new Date(2026, 9, 25, 15, 0);

    // Empty input
    assert.deepStrictEqual(groupSessionsByDay([], now), []);

    // Only today and earlier present (yesterday empty)
    const today = makeSession("s-today", new Date(2026, 9, 25, 10, 0).toISOString());
    const earlier = makeSession("s-earlier", new Date(2026, 8, 1, 10, 0).toISOString());
    const groups = groupSessionsByDay([today, earlier], now);

    assert.strictEqual(groups.length, 2);
    assert.strictEqual(groups[0].key, "today");
    assert.strictEqual(groups[1].key, "earlier");
  });

  await t.test("groupSessionsByDay: handles month boundary correctly for yesterday", () => {
    // now is 2026-03-01 10:00, yesterday is 2026-02-28 23:30
    const now = new Date(2026, 2, 1, 10, 0);
    const session = makeSession("s-month-boundary", new Date(2026, 1, 28, 23, 30).toISOString());
    const groups = groupSessionsByDay([session], now);

    assert.strictEqual(groups.length, 1);
    assert.strictEqual(groups[0].key, "yesterday");
    assert.strictEqual(groups[0].label, "昨天");
    assert.strictEqual(groups[0].items[0].sessionId, "s-month-boundary");
  });

  await t.test("groupSessionsByDay: handles year boundary correctly for yesterday", () => {
    // now is 2026-01-01 10:00, yesterday is 2025-12-31 23:45
    const now = new Date(2026, 0, 1, 10, 0);
    const session = makeSession("s-year-boundary", new Date(2025, 11, 31, 23, 45).toISOString());
    const groups = groupSessionsByDay([session], now);

    assert.strictEqual(groups.length, 1);
    assert.strictEqual(groups[0].key, "yesterday");
    assert.strictEqual(groups[0].label, "昨天");
    assert.strictEqual(groups[0].items[0].sessionId, "s-year-boundary");
  });

  await t.test("groupSessionsByDay: classifies future timestamp as today", () => {
    // now is 2026-10-25 15:00, future session is 2026-10-25 18:00 (or tomorrow)
    const now = new Date(2026, 9, 25, 15, 0);
    const futureToday = makeSession("s-future-today", new Date(2026, 9, 25, 18, 0).toISOString());
    const futureTomorrow = makeSession("s-future-tomorrow", new Date(2026, 9, 26, 9, 0).toISOString());

    const groups = groupSessionsByDay([futureToday, futureTomorrow], now);
    assert.strictEqual(groups.length, 1);
    assert.strictEqual(groups[0].key, "today");
    assert.strictEqual(groups[0].items.length, 2);
  });

  await t.test("groupSessionsByDay: places unparsable timestamp at the end of earlier group with 时间未知", () => {
    const now = new Date(2026, 9, 25, 15, 0);
    const validEarlier = makeSession("s-valid", new Date(2026, 8, 15, 10, 0).toISOString(), "有效历史");
    const invalidText = makeSession("s-invalid-1", "刚刚", "无效日期文字");
    const invalidEmpty = makeSession("s-invalid-2", "", "空日期字符串");

    const groups = groupSessionsByDay([invalidText, validEarlier, invalidEmpty], now);
    assert.strictEqual(groups.length, 1);
    assert.strictEqual(groups[0].key, "earlier");
    assert.strictEqual(groups[0].label, "更早");
    assert.strictEqual(groups[0].items.length, 3);
    // Valid item first
    assert.strictEqual(groups[0].items[0].sessionId, "s-valid");
    // Invalid items sort after all valid items, with timeLabel "时间未知"
    assert.strictEqual(groups[0].items[1].timeLabel, "时间未知");
    assert.strictEqual(groups[0].items[2].timeLabel, "时间未知");
    const invalidIds = [groups[0].items[1].sessionId, groups[0].items[2].sessionId];
    assert.ok(invalidIds.includes("s-invalid-1") && invalidIds.includes("s-invalid-2"));
  });

  // 2. formatHistoryTime
  await t.test("formatHistoryTime: formats today and yesterday as HH:mm with zero padding", () => {
    const now = new Date(2026, 9, 25, 15, 0);
    const todayMorning = new Date(2026, 9, 25, 8, 5).toISOString();
    const yesterdayNight = new Date(2026, 9, 24, 20, 9).toISOString();

    assert.strictEqual(formatHistoryTime(todayMorning, now), "08:05");
    assert.strictEqual(formatHistoryTime(yesterdayNight, now), "20:09");
  });

  await t.test("formatHistoryTime: formats earlier date in same year as M月D日 HH:mm without zero-padded month/day", () => {
    const now = new Date(2026, 9, 25, 15, 0);
    const earlierSameYear = new Date(2026, 8, 3, 16, 5).toISOString(); // 9月3日 16:05

    assert.strictEqual(formatHistoryTime(earlierSameYear, now), "9月3日 16:05");
  });

  await t.test("formatHistoryTime: formats earlier date in different year as YYYY年M月D日", () => {
    const now = new Date(2026, 9, 25, 15, 0);
    const earlierDiffYear = new Date(2025, 11, 30, 9, 0).toISOString(); // 2025年12月30日

    assert.strictEqual(formatHistoryTime(earlierDiffYear, now), "2025年12月30日");
  });

  await t.test("formatHistoryTime: returns 时间未知 for unparsable timestamp", () => {
    const now = new Date(2026, 9, 25, 15, 0);
    assert.strictEqual(formatHistoryTime("刚刚", now), "时间未知");
    assert.strictEqual(formatHistoryTime("", now), "时间未知");
    assert.strictEqual(formatHistoryTime("invalid-date", now), "时间未知");
  });

  // 3. historyTitle
  await t.test("historyTitle: trims valid title and falls back to 未命名会话 for null, undefined, empty or whitespace", () => {
    assert.strictEqual(historyTitle("  我的检索会话  "), "我的检索会话");
    assert.strictEqual(historyTitle("新会话"), "新会话");
    assert.strictEqual(historyTitle(null), "未命名会话");
    assert.strictEqual(historyTitle(undefined), "未命名会话");
    assert.strictEqual(historyTitle(""), "未命名会话");
    assert.strictEqual(historyTitle("   \n\t  "), "未命名会话");
  });

  // 4. sessionTitleFromRequest
  await t.test("sessionTitleFromRequest: collapses whitespace and newlines, returning null for empty result", () => {
    assert.strictEqual(sessionTitleFromRequest(""), null);
    assert.strictEqual(sessionTitleFromRequest("   \t\n   "), null);
    assert.strictEqual(
      sessionTitleFromRequest("  下载 Sentinel-1 \n\t SLC 数据  "),
      "下载 Sentinel-1 SLC 数据"
    );
  });

  await t.test("sessionTitleFromRequest: preserves exactly 40 characters unchanged", () => {
    // 40 ASCII chars
    const ascii40 = "A".repeat(40);
    assert.strictEqual(sessionTitleFromRequest(ascii40), ascii40);

    // 40 CJK chars
    const cjk40 = "景".repeat(40);
    assert.strictEqual(sessionTitleFromRequest(cjk40), cjk40);
  });

  await t.test(
    "sessionTitleFromRequest: truncates 41 characters to 40 characters with ellipsis, counting CJK and emoji as 1 character",
    () => {
      // 41 ASCII chars
      const ascii41 = "A".repeat(41);
      assert.strictEqual(sessionTitleFromRequest(ascii41), "A".repeat(40) + "…");

      // 41 CJK chars
      const cjk41 = "测".repeat(41);
      assert.strictEqual(sessionTitleFromRequest(cjk41), "测".repeat(40) + "…");

      // 41 Emojis (each emoji is 2 UTF-16 code units, but 1 code point)
      const rocket41 = "🚀".repeat(41);
      assert.strictEqual(sessionTitleFromRequest(rocket41), "🚀".repeat(40) + "…");
    }
  );

  // 5. nextHistoryDeleteState state machine
  await t.test(
    "nextHistoryDeleteState: request from idle/failed transitions to confirming(id); unchanged if already confirming/deleting",
    () => {
      // request from idle -> confirming
      const fromIdle = nextHistoryDeleteState(
        { phase: "idle" },
        { type: "request", sessionId: "sess-1" }
      );
      assert.deepStrictEqual(fromIdle, { phase: "confirming", sessionId: "sess-1" });

      // request from failed -> confirming
      const fromFailed = nextHistoryDeleteState(
        { phase: "failed", sessionId: "sess-old", message: "fail" },
        { type: "request", sessionId: "sess-2" }
      );
      assert.deepStrictEqual(fromFailed, { phase: "confirming", sessionId: "sess-2" });

      // request while confirming -> unchanged
      const whileConfirming: HistoryDeleteState = { phase: "confirming", sessionId: "sess-1" };
      assert.strictEqual(
        nextHistoryDeleteState(whileConfirming, { type: "request", sessionId: "sess-2" }),
        whileConfirming
      );

      // request while deleting -> unchanged
      const whileDeleting: HistoryDeleteState = { phase: "deleting", sessionId: "sess-1" };
      assert.strictEqual(
        nextHistoryDeleteState(whileDeleting, { type: "request", sessionId: "sess-2" }),
        whileDeleting
      );
    }
  );

  await t.test(
    "nextHistoryDeleteState: cancel from confirming/failed transitions to idle; unchanged while deleting",
    () => {
      // cancel from confirming -> idle
      assert.deepStrictEqual(
        nextHistoryDeleteState({ phase: "confirming", sessionId: "sess-1" }, { type: "cancel" }),
        { phase: "idle" }
      );

      // cancel from failed -> idle
      assert.deepStrictEqual(
        nextHistoryDeleteState(
          { phase: "failed", sessionId: "sess-1", message: "err" },
          { type: "cancel" }
        ),
        { phase: "idle" }
      );

      // cancel while deleting -> unchanged
      const whileDeleting: HistoryDeleteState = { phase: "deleting", sessionId: "sess-1" };
      assert.strictEqual(
        nextHistoryDeleteState(whileDeleting, { type: "cancel" }),
        whileDeleting
      );

      // cancel from idle -> idle
      const idleState: HistoryDeleteState = { phase: "idle" };
      assert.strictEqual(
        nextHistoryDeleteState(idleState, { type: "cancel" }),
        idleState
      );
    }
  );

  await t.test(
    "nextHistoryDeleteState: confirm only allowed from confirming -> deleting(same id); unchanged in other phases",
    () => {
      // confirm from confirming -> deleting
      assert.deepStrictEqual(
        nextHistoryDeleteState({ phase: "confirming", sessionId: "sess-1" }, { type: "confirm" }),
        { phase: "deleting", sessionId: "sess-1" }
      );

      // confirm from idle -> unchanged
      const idleState: HistoryDeleteState = { phase: "idle" };
      assert.strictEqual(
        nextHistoryDeleteState(idleState, { type: "confirm" }),
        idleState
      );

      // confirm from deleting -> unchanged
      const deletingState: HistoryDeleteState = { phase: "deleting", sessionId: "sess-1" };
      assert.strictEqual(
        nextHistoryDeleteState(deletingState, { type: "confirm" }),
        deletingState
      );

      // confirm from failed -> unchanged
      const failedState: HistoryDeleteState = { phase: "failed", sessionId: "sess-1", message: "err" };
      assert.strictEqual(
        nextHistoryDeleteState(failedState, { type: "confirm" }),
        failedState
      );
    }
  );

  await t.test(
    "nextHistoryDeleteState: succeeded transitions from deleting to idle; unchanged in other phases",
    () => {
      assert.deepStrictEqual(
        nextHistoryDeleteState({ phase: "deleting", sessionId: "sess-1" }, { type: "succeeded" }),
        { phase: "idle" }
      );

      const confirmingState: HistoryDeleteState = { phase: "confirming", sessionId: "sess-1" };
      assert.strictEqual(
        nextHistoryDeleteState(confirmingState, { type: "succeeded" }),
        confirmingState
      );

      const idleState: HistoryDeleteState = { phase: "idle" };
      assert.strictEqual(
        nextHistoryDeleteState(idleState, { type: "succeeded" }),
        idleState
      );

      const failedState: HistoryDeleteState = { phase: "failed", sessionId: "sess-1", message: "err" };
      assert.strictEqual(
        nextHistoryDeleteState(failedState, { type: "succeeded" }),
        failedState
      );
    }
  );

  await t.test(
    "nextHistoryDeleteState: failed transitions from deleting to failed(id, message); unchanged in other phases",
    () => {
      assert.deepStrictEqual(
        nextHistoryDeleteState(
          { phase: "deleting", sessionId: "sess-1" },
          { type: "failed", message: "网络故障" }
        ),
        { phase: "failed", sessionId: "sess-1", message: "网络故障" }
      );

      const confirmingState: HistoryDeleteState = { phase: "confirming", sessionId: "sess-1" };
      assert.strictEqual(
        nextHistoryDeleteState(confirmingState, { type: "failed", message: "err" }),
        confirmingState
      );

      const idleState: HistoryDeleteState = { phase: "idle" };
      assert.strictEqual(
        nextHistoryDeleteState(idleState, { type: "failed", message: "err" }),
        idleState
      );
    }
  );

  await t.test("HISTORY_DELETE_CONFIRM_TEXT: verified exact wording", () => {
    assert.strictEqual(HISTORY_DELETE_CONFIRM_TEXT.title, "删除这个会话？");
    assert.strictEqual(
      HISTORY_DELETE_CONFIRM_TEXT.body,
      "对话与运行记录会一并删除，已建任务和已下载文件不受影响。"
    );
    assert.strictEqual(HISTORY_DELETE_CONFIRM_TEXT.confirm, "删除");
    assert.strictEqual(HISTORY_DELETE_CONFIRM_TEXT.cancel, "取消");
  });

  // 6. Source text assertions for D3 integration
  await t.test(
    "D3 source assertions: AgentHistoryPage.tsx exists, imports logic, calls sessionsDelete once, no bulk delete strings or checkboxes",
    () => {
      const historyPagePath = path.resolve(__dirname, "../../components/desktop/AgentHistoryPage.tsx");
      assert.ok(fs.existsSync(historyPagePath), "AgentHistoryPage.tsx must exist");
      const content = fs.readFileSync(historyPagePath, "utf-8");

      assert.match(
        content,
        /import\s+[^;]*\bgroupSessionsByDay\b[^;]*from/,
        "AgentHistoryPage.tsx must import groupSessionsByDay"
      );
      assert.match(
        content,
        /import\s+[^;]*\bnextHistoryDeleteState\b[^;]*from/,
        "AgentHistoryPage.tsx must import nextHistoryDeleteState"
      );

      const deleteMatches = content.match(/sessionsDelete\(/g);
      assert.strictEqual(
        deleteMatches ? deleteMatches.length : 0,
        1,
        "AgentHistoryPage.tsx must call sessionsDelete( exactly once"
      );

      for (const forbidden of ["全选", "批量", "清空", "全部删除"]) {
        assert.ok(
          !content.includes(forbidden),
          `AgentHistoryPage.tsx must not contain '${forbidden}'`
        );
      }
      assert.doesNotMatch(
        content,
        /type=["']checkbox["']/,
        "AgentHistoryPage.tsx must not contain type='checkbox'"
      );
    }
  );

  await t.test(
    "D3 source assertions: DesktopWorkbench.tsx renders <AgentHistoryPage and no longer contains 历史尚未接入",
    () => {
      const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
      const content = fs.readFileSync(workbenchPath, "utf-8");
      assert.match(
        content,
        /<AgentHistoryPage\b/,
        "DesktopWorkbench.tsx must render <AgentHistoryPage"
      );
      assert.doesNotMatch(
        content,
        /历史尚未接入/,
        "DesktopWorkbench.tsx must no longer contain '历史尚未接入'"
      );
    }
  );

  await t.test(
    "D3 source assertions: DesktopAgentWorkbench.tsx calls sessionsCreate(sessionTitleFromRequest, removes dead delete code, contains openSessionId prop",
    () => {
      const agentWorkbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
      const content = fs.readFileSync(agentWorkbenchPath, "utf-8");
      assert.match(
        content,
        /sessionsCreate\(\s*sessionTitleFromRequest\(/,
        "DesktopAgentWorkbench.tsx must call sessionsCreate(sessionTitleFromRequest(...))"
      );
      assert.doesNotMatch(
        content,
        /handleDeleteSession/,
        "DesktopAgentWorkbench.tsx must no longer contain handleDeleteSession"
      );
      assert.doesNotMatch(
        content,
        /handleCreateSession/,
        "DesktopAgentWorkbench.tsx must no longer contain handleCreateSession"
      );
      assert.doesNotMatch(
        content,
        /pendingDeleteId/,
        "DesktopAgentWorkbench.tsx must no longer contain pendingDeleteId"
      );
      assert.match(
        content,
        /openSessionId\??\s*:\s*string/,
        "DesktopAgentWorkbenchProps must contain openSessionId"
      );
    }
  );

  // 7. resolveActiveSession pure logic
  await t.test(
    "resolveActiveSession: returns matching session when found; returns null when id is null, undefined, empty, or not found (never falls back to sessions[0])",
    () => {
      const s1 = makeSession("sess-1", new Date().toISOString(), "第一会话");
      const s2 = makeSession("sess-2", new Date().toISOString(), "第二会话");
      const list = [s1, s2];

      // Found
      assert.strictEqual(resolveActiveSession(list, "sess-2"), s2);
      assert.strictEqual(resolveActiveSession(list, "sess-1"), s1);

      // Not found -> must be null, never fall back to s1
      assert.strictEqual(resolveActiveSession(list, "sess-unknown"), null);

      // Null, undefined, empty string -> must be null
      assert.strictEqual(resolveActiveSession(list, null), null);
      assert.strictEqual(resolveActiveSession(list, undefined), null);
      assert.strictEqual(resolveActiveSession(list, ""), null);

      // Empty list -> null
      assert.strictEqual(resolveActiveSession([], "sess-1"), null);
    }
  );

  // 8. D4 source assertions: resolveActiveSession and DEV-gated fixture export
  await t.test(
    "D4 source assertions: DesktopAgentWorkbench uses resolveActiveSession, removes || sessions[0], and uses gated fixtureSessionsFactory",
    () => {
      const agentWorkbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
      const agentWorkbenchContent = fs.readFileSync(agentWorkbenchPath, "utf-8");

      assert.match(
        agentWorkbenchContent,
        /resolveActiveSession\(/,
        "DesktopAgentWorkbench.tsx must use resolveActiveSession("
      );
      assert.doesNotMatch(
        agentWorkbenchContent,
        /\|\|\s*sessions\[0\]/,
        "DesktopAgentWorkbench.tsx must no longer contain '|| sessions[0]'"
      );
      assert.doesNotMatch(
        agentWorkbenchContent,
        /export\s*\{\s*createFixtureSessions__fixture\s*\}/,
        "DesktopAgentWorkbench.tsx must not ungatedly export { createFixtureSessions__fixture }"
      );

      const historyPagePath = path.resolve(__dirname, "../../components/desktop/AgentHistoryPage.tsx");
      const historyPageContent = fs.readFileSync(historyPagePath, "utf-8");
      assert.doesNotMatch(
        historyPageContent,
        /createFixtureSessions__fixture/,
        "AgentHistoryPage.tsx must not contain createFixtureSessions__fixture"
      );
    }
  );
});
