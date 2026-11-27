import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor, within } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "../../lib/desktopShellBridge";
import type { AgentSessionSummary, AgentWorkspace } from "../../lib/generated/v3-contracts";
import { SidebarSessionTree } from "./SidebarSessionTree";
import { AgentHistoryPage } from "./AgentHistoryPage";

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  try {
    window.localStorage.clear();
  } catch {
    // 忽略
  }
});

const NOW = new Date();
function at(minutesAgo: number): string {
  return new Date(NOW.getTime() - minutesAgo * 60_000).toISOString();
}

/** 内存版后端：语义与 agent_sessions_* / agent_workspaces_* 一致 */
function fakeBackend(initial: AgentSessionSummary[], initialWorkspaces: AgentWorkspace[] = []) {
  let sessions = initial.map((s) => ({ ...s }));
  let workspaces = initialWorkspaces.map((w) => ({ ...w }));
  let seq = 0;
  const calls: string[] = [];
  const stamp = () => new Date(NOW.getTime() + ++seq * 1000).toISOString();
  const find = (id: string) => {
    const found = sessions.find((s) => s.session_id === id);
    if (!found) throw new Error("not found");
    return found;
  };
  const ordered = (list: AgentSessionSummary[]) =>
    [...list].sort((a, b) => {
      if (Boolean(a.pinned_at) !== Boolean(b.pinned_at)) return a.pinned_at ? -1 : 1;
      return Date.parse(b.updated_at) - Date.parse(a.updated_at);
    });

  const agent = {
    sessionsList: async (options?: { includeArchived?: boolean }) => {
      calls.push(`list:${options?.includeArchived ? "all" : "active"}`);
      return ordered(options?.includeArchived ? sessions : sessions.filter((s) => !s.archived_at)).map((s) => ({ ...s }));
    },
    sessionsCreate: async (title?: string | null, workspaceId?: string | null) => {
      const created: AgentSessionSummary = {
        session_id: `new-${++seq}`,
        title: title ?? "",
        created_at: stamp(),
        updated_at: stamp(),
        run_ids: [],
        workspace_id: workspaceId ?? null,
      };
      sessions.push(created);
      return { ...created };
    },
    sessionsDelete: async (id: string) => {
      calls.push(`delete:${id}`);
      sessions = sessions.filter((s) => s.session_id !== id);
    },
    sessionsRename: async (id: string, title: string) => {
      calls.push(`rename:${id}:${title}`);
      const s = find(id);
      s.title = title;
      return { ...s };
    },
    sessionsSetPinned: async (id: string, pinned: boolean) => {
      const s = find(id);
      s.pinned_at = pinned ? stamp() : null;
      return { ...s };
    },
    sessionsSetArchived: async (id: string, archived: boolean) => {
      const s = find(id);
      s.archived_at = archived ? stamp() : null;
      return { ...s };
    },
    sessionsMove: async (id: string, workspaceId: string | null) => {
      calls.push(`move:${id}:${workspaceId}`);
      const s = find(id);
      s.workspace_id = workspaceId;
      return { ...s };
    },
    sessionsExportTo: async (id: string, format: string, directory: string) => {
      calls.push(`export:${id}:${format}:${directory}`);
      return `${directory}\\${id}.${format === "json" ? "json" : "md"}`;
    },
    workspacesList: async () => workspaces.map((w) => ({ ...w })),
    workspacesCreate: async (name: string, folder?: string | null) => {
      const created: AgentWorkspace = { workspace_id: `ws-${++seq}`, name, folder: folder ?? null, created_at: stamp() };
      workspaces.push(created);
      calls.push(`ws-create:${name}`);
      return { ...created };
    },
    workspacesUpdate: async (id: string, change: { name?: string; folder?: string | null }) => {
      const ws = workspaces.find((w) => w.workspace_id === id)!;
      if (change.name !== undefined) ws.name = change.name;
      if (change.folder !== undefined) ws.folder = change.folder;
      calls.push(`ws-update:${id}:${JSON.stringify(change)}`);
      return { ...ws };
    },
    workspacesDelete: async (id: string) => {
      calls.push(`ws-delete:${id}`);
      workspaces = workspaces.filter((w) => w.workspace_id !== id);
      sessions = sessions.map((s) => (s.workspace_id === id ? { ...s, workspace_id: null } : s));
    },
  };

  const bridge = {
    kind: "tauri",
    agent,
    dialog: {
      pickDirectory: async () => ({ ok: true, path: "D:\\exports" }),
      pickOpenFile: async () => ({ ok: true, path: "" }),
    },
  } as unknown as DesktopShellBridge;
  registerTestDesktopShellBridge(bridge);
  return { calls, sessions: () => sessions, workspaces: () => workspaces };
}

function seed(): AgentSessionSummary[] {
  return [
    { session_id: "s-old", title: "九寨沟 NISAR", created_at: at(60), updated_at: at(60), run_ids: [] },
    { session_id: "s-mid", title: "茂县 Sentinel-1", created_at: at(30), updated_at: at(30), run_ids: [] },
    { session_id: "s-new", title: "花莲 ALOS-2", created_at: at(5), updated_at: at(5), run_ids: [] },
  ];
}

function renderSidebar() {
  const opened: string[] = [];
  const view = render(
    <SidebarSessionTree
      activeRoute="agent"
      activeSessionId={null}
      onOpenSession={(id) => opened.push(id)}
      onNavigate={() => {}}
    />
  );
  return { view, opened };
}

function unfiledTitles(container: HTMLElement): string[] {
  const list = container.querySelector('[data-testid="sidebar-unfiled"]') as HTMLElement;
  return Array.from(list.querySelectorAll("[data-session-row]")).map((row) =>
    (row.querySelector("button[title]") as HTMLElement).getAttribute("title") ?? ""
  );
}

function row(container: HTMLElement, sessionId: string): HTMLElement {
  const el = container.querySelector(`[data-session-row="${sessionId}"]`) as HTMLElement | null;
  assert.ok(el, `row ${sessionId} should be rendered`);
  return el;
}

async function openMenuAndPick(trigger: HTMLElement, ...labels: string[]) {
  fireEvent.click(trigger);
  let menu = await waitFor(() => {
    const menus = document.querySelectorAll('[role="menu"]');
    assert.ok(menus.length > 0, "menu should open");
    return menus[menus.length - 1] as HTMLElement;
  });
  for (let i = 0; i < labels.length; i += 1) {
    const item = within(menu).getByRole("menuitem", { name: labels[i] });
    fireEvent.click(item);
    if (i < labels.length - 1) {
      menu = await waitFor(() => {
        const menus = document.querySelectorAll('[role="menu"]');
        assert.ok(menus.length > i + 1, "submenu should open");
        return menus[menus.length - 1] as HTMLElement;
      });
    }
  }
}

test("会话整理交互（侧栏 + 历史页，fake bridge）", async (suite) => {
  await suite.test("1. 行内重命名：Enter 保存，Esc 取消", async () => {
    const backend = fakeBackend(seed());
    const { view } = renderSidebar();
    await waitFor(() => assert.deepEqual(unfiledTitles(view.container), ["花莲 ALOS-2", "茂县 Sentinel-1", "九寨沟 NISAR"]));
    // 没有工作区时保持「最近对话」标签
    assert.ok(view.getByText("最近对话"), "expectation at line 189");
    assert.equal(view.queryByText("未归入工作区") === null, true);

    await openMenuAndPick(within(row(view.container, "s-mid")).getByRole("button", { name: "会话操作" }), "重命名");
    const input = await view.findByRole("textbox", { name: "会话名称" });
    fireEvent.change(input, { target: { value: "  茂县 形变监测 " } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => assert.ok(unfiledTitles(view.container).includes("茂县 形变监测"), "expectation at line 196"));
    assert.ok(backend.calls.includes("rename:s-mid:茂县 形变监测"), "expectation at line 197");

    await openMenuAndPick(within(row(view.container, "s-old")).getByRole("button", { name: "会话操作" }), "重命名");
    const input2 = await view.findByRole("textbox", { name: "会话名称" });
    fireEvent.change(input2, { target: { value: "不应保存" } });
    fireEvent.keyDown(input2, { key: "Escape" });
    await waitFor(() => assert.ok(view.queryByRole("textbox", { name: "会话名称" }) === null, "rename input closed"));
    assert.ok(unfiledTitles(view.container).includes("九寨沟 NISAR"), "expectation at line 204");
    assert.ok(!backend.calls.some((c) => c.startsWith("rename:s-old")), "expectation at line 205");
  });

  await suite.test("2. 置顶后移到列表顶部并显示置顶图标；菜单 Esc 关闭并把焦点还给触发按钮", async () => {
    fakeBackend(seed());
    const { view } = renderSidebar();
    await waitFor(() => assert.equal(unfiledTitles(view.container)[0], "花莲 ALOS-2"));

    const trigger = within(row(view.container, "s-old")).getByRole("button", { name: "会话操作" });
    fireEvent.click(trigger);
    const menu = await waitFor(() => document.querySelector('[role="menu"]') as HTMLElement);
    assert.ok(menu, "menu should open");
    assert.ok(menu.parentElement?.parentElement === document.body, "menu is portaled to body, not inside the sidebar");
    fireEvent.keyDown(menu, { key: "Escape" });
    await waitFor(() => assert.ok(document.querySelector('[role="menu"]') === null, "menu closed"));
    assert.ok(document.activeElement === trigger, "focus returns to the menu trigger");

    await openMenuAndPick(trigger, "置顶");
    await waitFor(() => assert.equal(unfiledTitles(view.container)[0], "九寨沟 NISAR"));
    assert.ok(within(row(view.container, "s-old")).getByLabelText("已置顶"));
  });

  await suite.test("3. 归档后从侧栏消失，历史页「已归档」里出现并带标签", async () => {
    fakeBackend(seed());
    const sidebar = renderSidebar();
    const history = render(<AgentHistoryPage onOpenSession={() => {}} />);
    await waitFor(() => assert.equal(unfiledTitles(sidebar.view.container).length, 3));

    await openMenuAndPick(within(row(sidebar.view.container, "s-new")).getByRole("button", { name: "会话操作" }), "归档");
    await waitFor(() => assert.ok(!unfiledTitles(sidebar.view.container).includes("花莲 ALOS-2"), "expectation at line 234"));

    fireEvent.click(await history.findByRole("radio", { name: "已归档" }));
    await waitFor(() => {
      const rows = history.container.querySelectorAll("[data-history-row]");
      assert.equal(rows.length, 1);
      assert.equal(rows[0].getAttribute("data-history-row"), "s-new");
      assert.ok(within(rows[0] as HTMLElement).getByText("已归档"), "expectation at line 241");
    });

    // 「全部」仍可见并保留标签；取消归档后回到侧栏
    fireEvent.click(history.getByRole("radio", { name: "全部" }));
    const archivedRow = await waitFor(() => {
      const el = history.container.querySelector('[data-history-row="s-new"]') as HTMLElement;
      assert.ok(el, "expectation at line 248");
      return el;
    });
    await openMenuAndPick(within(archivedRow).getByRole("button", { name: "会话操作" }), "取消归档");
    await waitFor(() => assert.ok(unfiledTitles(sidebar.view.container).includes("花莲 ALOS-2"), "expectation at line 252"));
  });

  await suite.test("4. 新建工作区并把会话移进去；标签变为「未归入工作区」", async () => {
    const backend = fakeBackend(seed());
    const { view } = renderSidebar();
    await waitFor(() => assert.equal(unfiledTitles(view.container).length, 3));

    fireEvent.click(view.getByRole("button", { name: "新建工作区" }));
    const nameInput = await view.findByRole("textbox", { name: "工作区名称" });
    fireEvent.change(nameInput, { target: { value: "川西滑坡" } });
    fireEvent.keyDown(nameInput, { key: "Enter" });
    await waitFor(() => assert.ok(view.getByText("川西滑坡"), "expectation at line 264"));
    assert.ok(view.getByText("未归入工作区"), "expectation at line 265");
    const wsId = backend.workspaces()[0].workspace_id;
    assert.ok(backend.calls.some((c) => c.startsWith(`ws-update:${wsId}:`) && c.includes("川西滑坡")), "expectation at line 267");

    await openMenuAndPick(
      within(row(view.container, "s-mid")).getByRole("button", { name: "会话操作" }),
      "移到工作区",
      "川西滑坡"
    );
    await waitFor(() => {
      assert.ok(!unfiledTitles(view.container).includes("茂县 Sentinel-1"), "expectation at line 275");
      const group = view.container.querySelector(`[data-workspace="${wsId}"]`) as HTMLElement;
      assert.ok(group.querySelector('[data-session-row="s-mid"]'), "expectation at line 277");
    });
    assert.ok(backend.calls.includes(`move:s-mid:${wsId}`), "expectation at line 279");
  });

  await suite.test("5. 删除工作区（确认说明会话保留）后会话回到未归入列表", async () => {
    const backend = fakeBackend(
      seed().map((s) => (s.session_id === "s-old" ? { ...s, workspace_id: "ws-a" } : s)),
      [{ workspace_id: "ws-a", name: "九寨沟", folder: "D:\\insar\\jzg", created_at: at(120) }]
    );
    const { view } = renderSidebar();
    await waitFor(() => {
      const group = view.container.querySelector('[data-workspace="ws-a"]') as HTMLElement;
      assert.ok(group?.querySelector('[data-session-row="s-old"]'), "expectation at line 290");
    });
    assert.ok(!unfiledTitles(view.container).includes("九寨沟 NISAR"), "expectation at line 292");

    const group = view.container.querySelector('[data-workspace="ws-a"]') as HTMLElement;
    const wsTrigger = within(group).getByRole("button", { name: "工作区操作" });
    fireEvent.click(wsTrigger);
    const menu = await waitFor(() => document.querySelector('[role="menu"]') as HTMLElement);
    assert.ok(within(menu).getByRole("menuitem", { name: "更换文件夹" }), "expectation at line 298");
    assert.ok(within(menu).getByRole("menuitem", { name: "解除文件夹" }), "expectation at line 299");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "删除工作区" }));

    const dialog = await view.findByRole("alertdialog", { name: "删除这个工作区？" });
    assert.ok(within(dialog).getByText(/会话会回到未归入工作区/), "expectation at line 303");
    fireEvent.click(within(dialog).getByRole("button", { name: "删除" }));

    await waitFor(() => {
      assert.equal(view.container.querySelector('[data-workspace="ws-a"]') === null, true);
      assert.ok(unfiledTitles(view.container).includes("九寨沟 NISAR"), "expectation at line 308");
    });
    assert.ok(backend.calls.includes("ws-delete:ws-a"), "expectation at line 310");
    assert.ok(view.getByText("最近对话"), "no workspaces left: label returns to 最近对话");
  });

  await suite.test("6. 导出：选文件夹后提示文件名", async () => {
    const backend = fakeBackend(seed());
    const { view } = renderSidebar();
    await waitFor(() => assert.equal(unfiledTitles(view.container).length, 3));
    await openMenuAndPick(within(row(view.container, "s-new")).getByRole("button", { name: "会话操作" }), "导出为 Markdown");
    await waitFor(() => assert.ok(view.getByText("已导出 s-new.md"), "expectation at line 319"));
    assert.ok(backend.calls.includes("export:s-new:markdown:D:\\exports"), "expectation at line 320");
  });

  await suite.test("7. ChatGPT 风格侧栏：项目与最近独立对话分栏，支持将项目外对话拖拽进项目或拖回最近", async () => {
    const backend = fakeBackend(
      seed(),
      [{ workspace_id: "ws-proj", name: "EOStage开发", folder: null, created_at: at(100) }]
    );
    const { view } = renderSidebar();
    await waitFor(() => assert.equal(unfiledTitles(view.container).length, 3));

    // 验证「项目」与「最近」两个独立区域同时可见
    const projectsSection = view.getByTestId("sidebar-projects-section");
    const recentSection = view.getByTestId("sidebar-recent-section");
    assert.ok(projectsSection, "projects section rendered");
    assert.ok(recentSection, "recent section rendered");

    // 从「最近」把 s-new (花莲 ALOS-2) 拖拽进项目「EOStage开发」
    const sNewRow = row(view.container, "s-new");
    const projGroup = view.container.querySelector('[data-workspace="ws-proj"]') as HTMLElement;
    assert.ok(projGroup, "project group rendered");

    fireEvent.dragStart(sNewRow);
    fireEvent.dragOver(projGroup);
    fireEvent.drop(projGroup);

    await waitFor(() => {
      assert.ok(!unfiledTitles(view.container).includes("花莲 ALOS-2"), "s-new moved out of recent");
      assert.ok(projGroup.querySelector('[data-session-row="s-new"]'), "s-new now inside project");
    });
    assert.ok(backend.calls.includes("move:s-new:ws-proj"));

    // 再把项目内的 s-new 拖回「最近」区域
    const sNewInProj = projGroup.querySelector('[data-session-row="s-new"]') as HTMLElement;
    fireEvent.dragStart(sNewInProj);
    fireEvent.dragOver(recentSection);
    fireEvent.drop(recentSection);

    await waitFor(() => {
      assert.ok(unfiledTitles(view.container).includes("花莲 ALOS-2"), "s-new returned to recent");
      assert.equal(projGroup.querySelector('[data-session-row="s-new"]'), null);
    });
    assert.ok(backend.calls.includes("move:s-new:null"));
  });

  await suite.test("SidebarSessionTree 按运行态与持久态真实着色状态圆点，绝不把失败任务标绿", async () => {
    const statusSessions: AgentSessionSummary[] = [
      {
        session_id: "s-running",
        title: "运行中任务",
        created_at: at(50),
        updated_at: at(5),
        run_ids: ["r1"],
        permission_mode: "confirm_changes",
        pinned_at: null,
        archived_at: null,
        workspace_id: null,
        last_run_status: "completed",
      },
      {
        session_id: "s-approval",
        title: "等待确认方案",
        created_at: at(45),
        updated_at: at(10),
        run_ids: ["r2"],
        permission_mode: "confirm_changes",
        pinned_at: null,
        archived_at: null,
        workspace_id: null,
        last_run_status: "awaiting_approval",
      },
      {
        session_id: "s-failed",
        title: "模型服务不可达",
        created_at: at(40),
        updated_at: at(15),
        run_ids: ["r3"],
        permission_mode: "confirm_changes",
        pinned_at: null,
        archived_at: null,
        workspace_id: null,
        last_run_status: "failed",
      },
      {
        session_id: "s-cancelled",
        title: "已取消执行",
        created_at: at(35),
        updated_at: at(20),
        run_ids: ["r4"],
        permission_mode: "confirm_changes",
        pinned_at: null,
        archived_at: null,
        workspace_id: null,
        last_run_status: "cancelled",
      },
      {
        session_id: "s-completed",
        title: "已完成导出",
        created_at: at(30),
        updated_at: at(25),
        run_ids: ["r5"],
        permission_mode: "confirm_changes",
        pinned_at: null,
        archived_at: null,
        workspace_id: null,
        last_run_status: "completed",
      },
      {
        session_id: "s-idle",
        title: "空白新建会话",
        created_at: at(20),
        updated_at: at(30),
        run_ids: [],
        permission_mode: "confirm_changes",
        pinned_at: null,
        archived_at: null,
        workspace_id: null,
        last_run_status: null,
      },
    ];
    fakeBackend(statusSessions, []);

    const view = render(
      <SidebarSessionTree
        activeRoute="agent"
        activeSessionId="s-running"
        activeSessionRunStatus="running"
        onOpenSession={() => {}}
        onNavigate={() => {}}
      />
    );

    await waitFor(() => {
      assert.ok(view.getByTestId("session-status-dot-s-failed"));
    });

    const runningDot = view.getByTestId("session-status-dot-s-running");
    assert.equal(runningDot.getAttribute("data-run-status"), "running");
    assert.ok(runningDot.className.includes("bg-[#2563eb]"));
    assert.ok(runningDot.className.includes("animate-pulse"));

    const approvalDot = view.getByTestId("session-status-dot-s-approval");
    assert.equal(approvalDot.getAttribute("data-run-status"), "awaiting_approval");
    assert.ok(approvalDot.className.includes("bg-[#d97706]"));

    const failedDot = view.getByTestId("session-status-dot-s-failed");
    assert.equal(failedDot.getAttribute("data-run-status"), "failed");
    assert.ok(failedDot.className.includes("bg-[#dc2626]"));
    assert.ok(!failedDot.className.includes("bg-[#10b981]"));

    assert.equal(
      view.queryByTestId("session-status-dot-s-cancelled"),
      null,
      "cancelled session should stay clean with no status dot",
    );
    assert.equal(
      view.queryByTestId("session-status-dot-s-completed"),
      null,
      "completed session should stay clean with no status dot (atomic rename pattern)",
    );
    assert.equal(
      view.queryByTestId("session-status-dot-s-idle"),
      null,
      "idle session should stay clean with no status dot",
    );
  });

  await suite.test("Pointer 拖拽：支持在桌面 WebView 中将会话拖入文件夹及拖出回最近列表", async () => {
    cleanup();
    const wsList = [{ workspace_id: "ws-1", name: "川西滑坡专题", created_at: at(100), updated_at: at(100) }];
    const backend = fakeBackend(seed(), wsList);
    const { view, opened } = renderSidebar();
    await waitFor(() => assert.ok(view.container.querySelector('[data-workspace="ws-1"]')));

    // 1. 将独立会话 s-new 按住拖动移入文件夹 ws-1
    const sessionRow = row(view.container, "s-new");
    const wsTarget = view.container.querySelector('[data-workspace="ws-1"]') as HTMLElement;
    fireEvent.pointerDown(sessionRow, { button: 0, clientX: 40, clientY: 220 });
    fireEvent.pointerMove(wsTarget, { clientX: 45, clientY: 110 });
    assert.ok(document.querySelector('[data-testid="sidebar-drag-ghost"]'), "拖拽中渲染跟随胶囊");
    fireEvent.pointerUp(wsTarget, { clientX: 45, clientY: 110 });

    await waitFor(() => assert.ok(backend.calls.includes("move:s-new:ws-1")));
    assert.deepEqual(opened, [], "拖拽松开不得误触打开会话点击");

    // 2. 将文件夹内的会话 s-new 拖回「移出文件夹」区域
    const movedRow = await waitFor(() => row(view.container, "s-new"));
    fireEvent.pointerDown(movedRow, { button: 0, clientX: 45, clientY: 120 });
    fireEvent.pointerMove(movedRow, { clientX: 45, clientY: 180 });
    const unfileZone = await waitFor(() => view.getByTestId("sidebar-unfile-dropzone"));
    fireEvent.pointerMove(unfileZone, { clientX: 45, clientY: 200 });
    fireEvent.pointerUp(unfileZone, { clientX: 45, clientY: 200 });

    await waitFor(() => assert.ok(backend.calls.includes("move:s-new:null")));
  });

  await suite.test("空项目与空独立对话不再渲染冗余占位行；支持视图筛选与项目置顶", async () => {
    cleanup();
    window.localStorage.removeItem("eostage.shell.pinnedWorkspaces");
    fakeBackend([], []);
    const emptyView = renderSidebar().view;
    await waitFor(() => assert.ok(emptyView.getByTestId("sidebar-projects-section")));
    assert.ok(emptyView.queryByText(/新建项目（归档课题对话）/) === null);
    assert.ok(emptyView.queryByText(/暂无独立对话/) === null);

    cleanup();
    const wsList = [
      { workspace_id: "ws-1", name: "川西滑坡专题", created_at: at(100), updated_at: at(100) },
      { workspace_id: "ws-2", name: "鲜水河断裂带", created_at: at(110), updated_at: at(110) },
    ];
    const sessionsWithPin = [
      ...seed().map((s) => (s.session_id === "s-old" ? { ...s, workspace_id: "ws-2" } : s)),
      {
        session_id: "s-pin",
        title: "重点跟踪区域",
        created_at: at(300),
        updated_at: at(300),
        run_ids: [],
        permission_mode: "confirm_changes" as const,
        pinned_at: at(10),
        archived_at: null,
        workspace_id: "ws-1",
      },
    ];
    fakeBackend(sessionsWithPin, wsList);
    const { view } = renderSidebar();
    await waitFor(() => assert.ok(view.container.querySelector('[data-workspace="ws-2"]')));

    // 1. 置顶第二个项目 ws-2，应立即排到第一位
    const ws2 = view.container.querySelector('[data-workspace="ws-2"]') as HTMLElement;
    await openMenuAndPick(within(ws2).getByRole("button", { name: "工作区操作" }), "置顶项目");
    await waitFor(() => {
      const orderedIds = Array.from(view.container.querySelectorAll("[data-workspace]")).map((el) =>
        el.getAttribute("data-workspace")
      );
      assert.deepEqual(orderedIds, ["ws-2", "ws-1"]);
    });

    // 2. 切换「视图与排序」菜单：仅置顶 -> 最近更新 -> 按项目
    fireEvent.click(view.getByTestId("sidebar-session-filter-btn"));
    fireEvent.click(await waitFor(() => view.getByRole("menuitemradio", { name: "仅置顶" })));
    await waitFor(() => assert.deepEqual(unfiledTitles(view.container), ["重点跟踪区域"]));
    assert.ok(view.queryByTestId("sidebar-projects-section") === null);

    fireEvent.click(view.getByTestId("sidebar-session-filter-btn"));
    fireEvent.click(await waitFor(() => view.getByRole("menuitemradio", { name: "最近更新" })));
    await waitFor(() => assert.equal(unfiledTitles(view.container)[0], "花莲 ALOS-2"));

    fireEvent.click(view.getByTestId("sidebar-session-filter-btn"));
    fireEvent.click(await waitFor(() => view.getByRole("menuitemradio", { name: "按项目" })));
    await waitFor(() => assert.ok(view.getByTestId("sidebar-projects-section")));
  });
});
