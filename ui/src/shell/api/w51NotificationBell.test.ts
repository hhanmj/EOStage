import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");
const desktopDir = path.resolve(uiSrcDir, "components/desktop");

import {
  deriveTaskEvents,
  readNotificationState,
  writeNotificationState,
  markRead,
  markAllRead,
  getNotificationKindMeta,
  formatNotificationTime,
  NOTIFICATION_STORAGE_KEY,
  MAX_NOTIFICATIONS_CAP,
  type BellEventKind,
  type NotificationItem,
  type NotificationState,
} from "./notificationBell.ts";
import type { ApplicationTask } from "./types.ts";
import type { SimpleStorage } from "./shellRoutes.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

function makeMockTask(id: string, status: any, name = `Task ${id}`, doneBytes = 100): ApplicationTask {
  return {
    task_id: id,
    revision: 1,
    category: "scene_download",
    name,
    status,
    done_bytes: doneBytes,
    total_bytes: 1000,
  };
}

test("W5-1c Notification Bell Specification", async (t) => {
  // ==========================================================================
  // 1. deriveTaskEvents: lifecycle transitions
  // ==========================================================================
  await t.test("1. deriveTaskEvents: running -> completed emits one event", () => {
    const prev = [makeMockTask("t1", "running")];
    const next = [makeMockTask("t1", "completed")];

    const events = deriveTaskEvents(prev, next);
    assert.strictEqual(events.length, 1);
    assert.strictEqual(events[0].task_id, "t1");
    assert.strictEqual(events[0].kind, "completed");
    assert.strictEqual(events[0].route, "downloads");
  });

  await t.test("2. deriveTaskEvents: running -> failed emits one event", () => {
    const prev = [makeMockTask("t2", "running")];
    const next = [makeMockTask("t2", "failed")];

    const events = deriveTaskEvents(prev, next);
    assert.strictEqual(events.length, 1);
    assert.strictEqual(events[0].task_id, "t2");
    assert.strictEqual(events[0].kind, "failed");
    assert.strictEqual(events[0].route, "downloads");
  });

  await t.test("3. deriveTaskEvents: queued -> completed emits one event", () => {
    const prev = [makeMockTask("t3", "queued")];
    const next = [makeMockTask("t3", "completed")];

    const events = deriveTaskEvents(prev, next);
    assert.strictEqual(events.length, 1);
    assert.strictEqual(events[0].task_id, "t3");
    assert.strictEqual(events[0].kind, "completed");
  });

  await t.test("4. deriveTaskEvents: first observation of finished task emits event", () => {
    const prev: ApplicationTask[] = [];
    const next = [makeMockTask("t4", "completed")];

    const events = deriveTaskEvents(prev, next);
    assert.strictEqual(events.length, 1);
    assert.strictEqual(events[0].task_id, "t4");
    assert.strictEqual(events[0].kind, "completed");
  });

  await t.test("5. deriveTaskEvents: repeated identical or terminal snapshots emit nothing", () => {
    const prev = [makeMockTask("t5", "completed")];
    const next = [makeMockTask("t5", "completed")];

    const events = deriveTaskEvents(prev, next);
    assert.strictEqual(events.length, 0);

    // 仅非状态字段改变（如进度）不发出通知
    const prevRunning = [makeMockTask("t6", "running", "T6", 100)];
    const nextRunning = [makeMockTask("t6", "running", "T6", 500)];
    assert.strictEqual(deriveTaskEvents(prevRunning, nextRunning).length, 0);
  });

  await t.test("6. deriveTaskEvents: does not emit needs_confirmation in this slice", () => {
    const prev = [makeMockTask("t7", "running")];
    const next = [makeMockTask("t7", "paused")];

    const events = deriveTaskEvents(prev, next);
    assert.strictEqual(events.length, 0);
  });

  // ==========================================================================
  // 2. Persistence and Cap of 50
  // ==========================================================================
  await t.test("7. Persistence round-trip and cap at 50 items", () => {
    const memory = new Map<string, string>();
    const storage: SimpleStorage = {
      getItem: (k) => memory.get(k) ?? null,
      setItem: (k, v) => memory.set(k, v),
    };

    assert.deepStrictEqual(readNotificationState(storage), { items: [], unreadIds: [] });

    // 构造 60 条测试通知
    const sixtyItems: NotificationItem[] = Array.from({ length: 60 }, (_, i) => ({
      id: `notif-${i}`,
      task_id: `task-${i}`,
      kind: "completed",
      title: `Task ${i}`,
      occurred_at: 1000 + i,
      route: "downloads",
    }));

    const state: NotificationState = {
      items: sixtyItems,
      unreadIds: sixtyItems.map((item) => item.id),
    };

    writeNotificationState(storage, state);
    const loaded = readNotificationState(storage);

    assert.strictEqual(loaded.items.length, 50);
    assert.strictEqual(loaded.unreadIds.length, 50);
    assert.strictEqual(loaded.items[0].id, "notif-0");
  });

  // ==========================================================================
  // 3. Mark Read and Mark All Read
  // ==========================================================================
  await t.test("8. markRead & markAllRead state transitions", () => {
    const state: NotificationState = {
      items: [
        { id: "n1", task_id: "t1", kind: "completed", title: "T1", occurred_at: 1, route: "downloads" },
        { id: "n2", task_id: "t2", kind: "failed", title: "T2", occurred_at: 2, route: "downloads" },
      ],
      unreadIds: ["n1", "n2"],
    };

    const afterOne = markRead(state, "n1");
    assert.deepStrictEqual(afterOne.unreadIds, ["n2"]);

    const afterAll = markAllRead(state);
    assert.deepStrictEqual(afterAll.unreadIds, []);
  });

  // ==========================================================================
  // 4. Kind Metadata and Time Formatting
  // ==========================================================================
  await t.test("9. UI meta mapping: kind labels and styling", () => {
    const compMeta = getNotificationKindMeta("completed");
    assert.strictEqual(compMeta.label, "已完成");
    assert.strictEqual(compMeta.textClass, "text-sp-success-text");
    assert.strictEqual(compMeta.bgClass, "bg-sp-success-bg");

    const failMeta = getNotificationKindMeta("failed");
    assert.strictEqual(failMeta.label, "已失败");
    assert.strictEqual(failMeta.textClass, "text-sp-danger-text");
    assert.strictEqual(failMeta.bgClass, "bg-sp-danger-bg");
  });

  // ==========================================================================
  // 5. DesktopTitlebar Bell Popover Source Assertions
  // ==========================================================================
  await t.test("10. DesktopTitlebar: renders bell dropdown popover with accessibility and empty state", () => {
    const titlebarPath = path.join(desktopDir, "DesktopTitlebar.tsx");
    const popoverPath = path.join(desktopDir, "NotificationPopover.tsx");
    const titlebarContent = fs.readFileSync(titlebarPath, "utf8");
    const popoverContent = fs.readFileSync(popoverPath, "utf8");
    // deduplication: DesktopTitlebar and DesktopActivityRail share NotificationPopover.tsx
    const combinedContent = `${titlebarContent}\n${popoverContent}`;

    assert.ok(
      titlebarContent.includes('aria-haspopup="true"'),
      "DesktopTitlebar must include aria-haspopup on bell trigger"
    );
    assert.ok(
      titlebarContent.includes("aria-expanded="),
      "DesktopTitlebar must include aria-expanded on bell trigger"
    );
    assert.ok(
      titlebarContent.includes("<NotificationPopover"),
      "DesktopTitlebar must render shared NotificationPopover"
    );
    assert.ok(
      popoverContent.includes('role="dialog"') || popoverContent.includes('role="menu"'),
      "NotificationPopover must render popover with dialog or menu role"
    );
    assertLocalizedSource(combinedContent, "shell.titlebar.notifications", "通知");
    assertLocalizedSource(combinedContent, "shell.titlebar.noNotifications", "暂无新通知");
    assertLocalizedSource(combinedContent, "shell.titlebar.notificationsDescription", "任务完成、失败与需要确认的事项");
    assert.ok(
      !popoverContent.includes("text-[10px]") && !popoverContent.includes("text-[11px]"),
      "NotificationPopover bell dropdown must not contain <12px font sizes like text-[10px] or text-[11px]"
    );
  });

  // ==========================================================================
  // 6. DesktopWorkbench Event Subscription Wiring
  // ==========================================================================
  await t.test("11. DesktopWorkbench: subscribes to task events and integrates notification bell", () => {
    const workbenchPath = path.join(desktopDir, "DesktopWorkbench.tsx");
    const content = fs.readFileSync(workbenchPath, "utf8");

    assert.ok(
      content.includes("deriveTaskEvents"),
      "DesktopWorkbench must call deriveTaskEvents"
    );
    assert.ok(
      content.includes("mergeNotificationEvents"),
      "DesktopWorkbench must merge notification events"
    );
    assert.ok(
      content.includes("hasBaselinedTasksRef"),
      "DesktopWorkbench must guard cold start with hasBaselinedTasksRef to prevent first-snapshot notification flooding"
    );
    assert.ok(
      content.includes("isFixtureBackend()"),
      "DesktopWorkbench must guard notification persistence with isFixtureBackend"
    );
    assert.ok(
      content.includes("onNotificationClick="),
      "DesktopWorkbench must pass onNotificationClick to DesktopTitlebar"
    );
  });
});
