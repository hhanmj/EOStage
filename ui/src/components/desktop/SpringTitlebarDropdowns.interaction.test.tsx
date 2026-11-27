import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { DesktopTitlebar } from "./DesktopTitlebar";
import type { NotificationItem } from "../../shell/api/notificationBell";
import type { ApplicationTask } from "../../shell/api/types";
import type { ShellRoute } from "../../shell/api/shellRoutes";

test.afterEach(() => {
  cleanup();
});

const SAMPLE_TASKS: ApplicationTask[] = [
  {
    task_id: "spring-dl-1",
    revision: 1,
    category: "scene_download",
    name: "Landsat-9 合肥市 C2L2 光学数据",
    status: "running",
    done_bytes: 180 * 1024 * 1024,
    total_bytes: 400 * 1024 * 1024,
  },
  {
    task_id: "spring-dl-2",
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1 IW SLC 茂县批次",
    status: "completed",
    done_bytes: 900 * 1024 * 1024,
    total_bytes: 900 * 1024 * 1024,
  },
];

const SAMPLE_NOTIFICATIONS: NotificationItem[] = [
  {
    id: "notif-spring-1",
    task_id: "spring-dl-2",
    kind: "completed",
    title: "Sentinel-1 IW SLC 下载完成",
    detail: ["已保存到本地工作目录"],
    occurred_at: 1763230000000,
    route: "downloads",
  },
  {
    id: "notif-spring-2",
    task_id: "spring-dl-fail",
    kind: "failed",
    title: "ALOS-2 数据检索超时",
    detail: ["请检查网络代理配置后重试"],
    occurred_at: 1763229000000,
    route: "downloads",
  },
];

test("DesktopTitlebar 顶栏三组下拉卡 Spring Dropdown 交互与状态机测试", async (suite) => {
  await suite.test("1. 场景切换下拉卡：打开、悬停/键盘上下切换与高亮层、Escape 收起并恢复焦点、切路由槽位更新", () => {
    function SwitcherHarness() {
      const [route, setRoute] = React.useState<ShellRoute>("agent");
      return (
        <DesktopTitlebar
          activeRoute={route}
          onNavigate={(next) => setRoute(next)}
          onStartNewChat={() => {}}
          onOpenDataCatalog={() => {}}
        />
      );
    }

    const view = render(<SwitcherHarness />);
    const trigger = view.getByTestId("titlebar-workspace-switcher") as HTMLButtonElement;
    assert.strictEqual(trigger.getAttribute("aria-expanded"), "false");
    assert.ok(view.getByText("智能检索"));

    fireEvent.click(trigger);
    assert.strictEqual(trigger.getAttribute("aria-expanded"), "true");

    const menu = view.getByTestId("titlebar-workspace-menu");
    assert.strictEqual(menu.getAttribute("role"), "menu");
    assert.ok(menu.querySelector('[data-testid="spring-dropdown-highlight"]'));

    const items = Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'));
    assert.strictEqual(items.length, 7);

    // Current route item carries aria-current="page" and SVG checkmark
    const currentItem = items.find((el) => el.getAttribute("aria-current") === "page");
    assert.ok(currentItem);
    assert.ok(currentItem?.textContent?.includes("智能检索"));
    assert.ok(currentItem?.querySelector("svg path"));

    // Keyboard navigation moves focus across menu items
    fireEvent.keyDown(window, { key: "ArrowDown" });
    assert.strictEqual(document.activeElement, items[0]);
    fireEvent.keyDown(window, { key: "ArrowDown" });
    assert.strictEqual(document.activeElement, items[1]);
    fireEvent.keyDown(window, { key: "ArrowDown" });
    assert.strictEqual(document.activeElement, items[2]);
    fireEvent.keyDown(window, { key: "ArrowUp" });
    assert.strictEqual(document.activeElement, items[1]);

    // Click workbench option -> closes menu & updates trigger label cleanly without duplicate text
    fireEvent.click(items[2]);
    assert.strictEqual(view.queryByTestId("titlebar-workspace-menu"), null);
    assert.strictEqual(view.getAllByText("地图工作台").length, 1);

    // Reopen and press Escape -> closes and restores focus to trigger
    fireEvent.click(trigger);
    assert.ok(view.getByTestId("titlebar-workspace-menu"));
    fireEvent.keyDown(window, { key: "Escape" });
    assert.strictEqual(view.queryByTestId("titlebar-workspace-menu"), null);
    assert.strictEqual(document.activeElement, trigger);
  });

  await suite.test("2. 右上角下载浮窗与通知浮窗：渲染高亮层、点击回调、互斥切换与 Escape 恢复焦点", () => {
    let clickedNotifId = "";
    let drawerTaskId = "";

    const view = render(
      <DesktopTitlebar
        activeRoute="workbench"
        activeDownloadCount={1}
        downloadTasks={SAMPLE_TASKS}
        onOpenDownloadDrawer={(id) => {
          drawerTaskId = id ?? "";
        }}
        onOpenDownloadCenter={() => {}}
        sidebarExpanded={false}
        unreadCount={1}
        notifications={SAMPLE_NOTIFICATIONS}
        unreadIds={["notif-spring-1"]}
        onNotificationClick={(item) => {
          clickedNotifId = item.id;
        }}
      />
    );

    const dlTrigger = view.getByTestId("titlebar-download-trigger") as HTMLButtonElement;
    const bellTrigger = view.getByLabelText("通知") as HTMLButtonElement;

    // Open download popover
    fireEvent.click(dlTrigger);
    const dlPopover = view.getByTestId("titlebar-download-popover");
    assert.ok(dlPopover.querySelector('[data-testid="spring-dropdown-highlight"]'));
    const dlRows = view.getAllByTestId("titlebar-download-popover-item");
    assert.strictEqual(dlRows.length, 2);
    fireEvent.mouseEnter(dlRows[1]);
    fireEvent.click(dlRows[0]);
    assert.strictEqual(drawerTaskId, "spring-dl-1");
    assert.strictEqual(view.queryByTestId("titlebar-download-popover"), null);

    // Open notification popover -> renders spring surface + highlight layer
    fireEvent.click(bellTrigger);
    const notifPopover = view.getByTestId("titlebar-notification-popover");
    assert.strictEqual(notifPopover.getAttribute("role"), "dialog");
    assert.ok(notifPopover.querySelector('[data-testid="spring-dropdown-highlight"]'));
    const notifRows = view.getAllByTestId("titlebar-notification-item");
    assert.strictEqual(notifRows.length, 2);

    // Opening workspace switcher automatically closes notification popover (mutually exclusive)
    fireEvent.click(view.getByTestId("titlebar-workspace-switcher"));
    assert.strictEqual(view.queryByTestId("titlebar-notification-popover"), null);
    assert.ok(view.getByTestId("titlebar-workspace-menu"));

    // Escape closes workspace switcher cleanly; reopen bell and test click item
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.click(bellTrigger);
    fireEvent.click(view.getAllByTestId("titlebar-notification-item")[1]);
    assert.strictEqual(clickedNotifId, "notif-spring-2");
    assert.strictEqual(view.queryByTestId("titlebar-notification-popover"), null);
  });
});
