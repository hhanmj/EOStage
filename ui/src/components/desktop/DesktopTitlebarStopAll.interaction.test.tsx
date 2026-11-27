import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { DesktopTitlebar } from "./DesktopTitlebar";
import type { NotificationItem } from "../../shell/api/notificationBell";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  selectDesktopShellBridge,
} from "../../lib/desktopShellBridge";

test.afterEach(() => {
  cleanup();
});

test("DesktopTitlebar 全部停止与通知交互测试套件", async (suite) => {
  await suite.test("1. 禁用时按钮 disabled 且 title 为 reason_zh", () => {
    const { getByRole } = render(
      <DesktopTitlebar
        stopAll={{
          enabled: false,
          reason_zh: "没有进行中的 Agent 运行或 Agent 创建的进行中任务",
          busy: false,
          onClick: () => {},
        }}
      />
    );

    const btn = getByRole("button", { name: "全部停止" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, true);
    assert.strictEqual(
      btn.title,
      "没有进行中的 Agent 运行或 Agent 创建的进行中任务"
    );
  });

  await suite.test("2. 启用时点击调用 onClick", () => {
    let clicked = false;
    const { getByRole } = render(
      <DesktopTitlebar
        stopAll={{
          enabled: true,
          reason_zh: "",
          busy: false,
          onClick: () => {
            clicked = true;
          },
        }}
      />
    );

    const btn = getByRole("button", { name: "全部停止" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, false);
    assert.strictEqual(btn.title, "全部停止");

    fireEvent.click(btn);
    assert.strictEqual(clicked, true);
  });

  await suite.test("3. busy 时禁用且 title 为「正在停止...」", () => {
    let clicked = false;
    const { getByRole } = render(
      <DesktopTitlebar
        stopAll={{
          enabled: true,
          reason_zh: "",
          busy: true,
          onClick: () => {
            clicked = true;
          },
        }}
      />
    );

    const btn = getByRole("button", { name: "全部停止" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, true);
    assert.strictEqual(btn.title, "正在停止...");

    fireEvent.click(btn);
    assert.strictEqual(clicked, false);
  });

  await suite.test("4. 通知弹层里 stop_all 项显示「部分未能停止」与失败任务名", async () => {
    const mockNotifs: NotificationItem[] = [
      {
        id: "notif-stop-all-01",
        kind: "stop_all",
        title: "部分未能停止",
        occurred_at: 1700000000000,
        route: "agent",
        detail: [
          "取消了 1 个运行、0 个任务",
          "未能停止的任务：哨兵1号下载、task-xyz",
        ],
      },
    ];

    const { getByRole, findByText, findByRole } = render(
      <DesktopTitlebar
        unreadCount={1}
        notifications={mockNotifs}
        unreadIds={["notif-stop-all-01"]}
      />
    );

    // 点击铃图标打开通知弹层
    const bellBtn = getByRole("button", { name: "通知" });
    fireEvent.click(bellBtn);

    // 弹层展示
    const popover = await findByRole("dialog", { name: "通知" });
    assert.ok(popover);

    // 验证 stop_all 徽章与标题
    assert.ok(await findByText("全部停止"));
    assert.ok(await findByText("部分未能停止"));

    // 验证 detail 各行
    assert.ok(await findByText("取消了 1 个运行、0 个任务"));
    assert.ok(await findByText("未能停止的任务：哨兵1号下载、task-xyz"));
  });
  await suite.test("5. 启用且计数已知时 title 显示进行中运行数与 Agent 活动任务数", () => {
    const { getByRole } = render(
      <DesktopTitlebar
        stopAll={{
          enabled: true,
          reason_zh: "",
          busy: false,
          onClick: () => {},
          live_runs: 2,
          agent_active_tasks: 3,
        }}
      />
    );

    const btn = getByRole("button", { name: "全部停止" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, false);
    assert.strictEqual(
      btn.title,
      "全部停止：进行中的运行 2，Agent 活动任务 3"
    );
  });

  await suite.test("6. 启用但计数未知时 title 为「全部停止」", () => {
    const { getByRole } = render(
      <DesktopTitlebar
        stopAll={{
          enabled: true,
          reason_zh: "",
          busy: false,
          onClick: () => {},
        }}
      />
    );

    const btn = getByRole("button", { name: "全部停止" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, false);
    assert.strictEqual(btn.title, "全部停止");
  });

  await suite.test("7. 禁用时即使传入计数仍显示 reason_zh", () => {
    const { getByRole } = render(
      <DesktopTitlebar
        stopAll={{
          enabled: false,
          reason_zh: "没有进行中的 Agent 运行或 Agent 创建的进行中任务",
          busy: false,
          onClick: () => {},
          live_runs: 0,
          agent_active_tasks: 0,
        }}
      />
    );

    const btn = getByRole("button", { name: "全部停止" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, true);
    assert.strictEqual(
      btn.title,
      "没有进行中的 Agent 运行或 Agent 创建的进行中任务"
    );

  });

  await suite.test("8. 标题栏空白处双击只触发一次 toggleMaximize（消除 mouseDown detail=2 与 onDoubleClick 双重反转），且交互按钮不误触", async () => {
    let toggleCalls = 0;
    const base = selectDesktopShellBridge({});
    registerTestDesktopShellBridge({
      ...base,
      window: {
        ...base.window,
        toggleMaximize: async () => {
          toggleCalls += 1;
          return { ok: true };
        },
      },
    });

    try {
      const view = render(<DesktopTitlebar sidebarExpanded={false} onToggleSidebar={() => {}} />);
      const header = view.container.querySelector("header[data-tauri-drag-region]") as HTMLElement;
      assert.ok(header, "header[data-tauri-drag-region] must exist");

      // Double-clicking an interactive button inside the titlebar must NOT trigger maximize
      const sidebarBtn = view.container.querySelector("header button") as HTMLButtonElement;
      assert.ok(sidebarBtn, "sidebar toggle button must exist");
      fireEvent.mouseDown(sidebarBtn, { button: 0, detail: 2 });
      fireEvent.dblClick(sidebarBtn);
      assert.strictEqual(toggleCalls, 0, "interactive buttons must not trigger window maximize");

      // Real browser double-click sequence: mouseDown(detail=2) + mouseUp(detail=2) followed by dblClick;
      // document-level drag listeners (e.g. Tauri drag.js) must NOT see propagated header mousedown/mouseup.
      let leakedDocumentEvents = 0;
      const onDocMouseDown = () => {
        leakedDocumentEvents += 1;
      };
      const onDocMouseUp = () => {
        leakedDocumentEvents += 1;
      };
      document.addEventListener("mousedown", onDocMouseDown);
      document.addEventListener("mouseup", onDocMouseUp);
      try {
        fireEvent.mouseDown(header, { button: 0, detail: 2 });
        fireEvent.mouseUp(header, { button: 0, detail: 2 });
        fireEvent.dblClick(header);
        assert.strictEqual(toggleCalls, 1, "first double-click gesture must call toggleMaximize once, not twice");
        assert.strictEqual(leakedDocumentEvents, 0, "header drag/dblclick must stopPropagation before document Tauri drag.js");

        // Second double-click after dedup window (>280ms) restores window cleanly
        await new Promise((r) => setTimeout(r, 310));
        fireEvent.mouseDown(header, { button: 0, detail: 2 });
        fireEvent.mouseUp(header, { button: 0, detail: 2 });
        fireEvent.dblClick(header);
        assert.strictEqual(toggleCalls, 2, "second double-click gesture after >280ms must call toggleMaximize again");
        assert.strictEqual(leakedDocumentEvents, 0, "second double-click must also isolate document drag listeners");
      } finally {
        document.removeEventListener("mousedown", onDocMouseDown);
        document.removeEventListener("mouseup", onDocMouseUp);
      }
    } finally {
      clearTestDesktopShellBridge();
    }
  });
});
