import assert from "node:assert/strict";
import test from "node:test";
import type { AgentSessionSummary, AgentWorkspace } from "../../lib/generated/v3-contracts.ts";
import {
  buildSessionMenu,
  buildSessionTree,
  buildWorkspaceMenu,
  defaultWorkspaceName,
  detachWorkspaceSessions,
  exportNoteText,
  filterHistorySessions,
  flattenMenu,
  normalizeRenameInput,
  orderSessions,
  orderWorkspaces,
  replaceSession,
  sidebarSessions,
} from "./sessionOrganize.ts";

function s(id: string, updated: string, extra: Partial<AgentSessionSummary> = {}): AgentSessionSummary {
  return { session_id: id, title: id, created_at: updated, updated_at: updated, run_ids: [], ...extra };
}

function w(id: string, name: string, created: string, folder: string | null = null): AgentWorkspace {
  return { workspace_id: id, name, folder, created_at: created };
}

test("sessionOrganize 纯逻辑", async (t) => {
  await t.test("orderSessions：置顶在前（置顶时间新者在前），其余按更新时间倒序，坏时间排末尾", () => {
    const list = [
      s("a", "2026-10-01T00:00:00Z"),
      s("b", "2026-10-03T00:00:00Z"),
      s("bad", "not-a-date"),
      s("p1", "2026-09-01T00:00:00Z", { pinned_at: "2026-10-02T00:00:00Z" }),
      s("p2", "2026-09-02T00:00:00Z", { pinned_at: "2026-10-05T00:00:00Z" }),
    ];
    assert.deepEqual(orderSessions(list).map((x) => x.session_id), ["p2", "p1", "b", "a", "bad"]);
    assert.equal(list[0].session_id, "a", "不修改输入数组");
  });

  await t.test("orderWorkspaces：按创建时间升序，同时间按名称", () => {
    const out = orderWorkspaces([
      w("2", "C", "2026-10-02T00:00:00Z"),
      w("1", "甲", "2026-10-01T00:00:00Z"),
      w("3", "B", "2026-10-02T00:00:00Z"),
    ]);
    assert.deepEqual(out.map((x) => x.workspace_id), ["1", "3", "2"]);
  });

  await t.test("buildSessionTree：按工作区分组，空工作区保留，未知 workspace_id 归入未归入", () => {
    const tree = buildSessionTree(
      [
        s("x", "2026-10-01T00:00:00Z", { workspace_id: "w1" }),
        s("y", "2026-10-02T00:00:00Z", { workspace_id: "w1" }),
        s("z", "2026-10-03T00:00:00Z", { workspace_id: "gone" }),
        s("u", "2026-10-04T00:00:00Z"),
      ],
      [w("w1", "滑坡", "2026-10-01T00:00:00Z"), w("w2", "空", "2026-10-02T00:00:00Z")]
    );
    assert.deepEqual(tree.groups.map((g) => [g.workspace.workspace_id, g.sessions.map((x) => x.session_id)]), [
      ["w1", ["y", "x"]],
      ["w2", []],
    ]);
    assert.deepEqual(tree.unfiled.map((x) => x.session_id), ["u", "z"]);
    assert.equal(tree.unfiledLabel, "未归入工作区");
  });

  await t.test("buildSessionTree：没有工作区时保留「最近对话」标签", () => {
    const tree = buildSessionTree([s("a", "2026-10-01T00:00:00Z")], []);
    assert.equal(tree.groups.length, 0);
    assert.equal(tree.unfiledLabel, "最近对话");
  });

  await t.test("sidebarSessions / filterHistorySessions：归档过滤", () => {
    const list = [s("a", "2026-10-01T00:00:00Z"), s("b", "2026-10-01T00:00:00Z", { archived_at: "2026-10-02T00:00:00Z" })];
    assert.deepEqual(sidebarSessions(list).map((x) => x.session_id), ["a"]);
    assert.deepEqual(filterHistorySessions(list, "all").map((x) => x.session_id), ["a", "b"]);
    assert.deepEqual(filterHistorySessions(list, "archived").map((x) => x.session_id), ["b"]);
  });

  await t.test("buildSessionMenu：未置顶/未归档/未归入工作区", () => {
    const menu = buildSessionMenu(s("a", "2026-10-01T00:00:00Z"), [w("w1", "滑坡", "2026-10-01T00:00:00Z")]);
    assert.deepEqual(menu.map((m) => m.label), ["重命名", "置顶", "移到工作区", "归档", "导出为 Markdown", "导出为 JSON"]);
    const move = menu.find((m) => m.id === "move")!;
    assert.deepEqual(move.items!.map((m) => m.action), [{ kind: "move", workspaceId: "w1" }]);
    assert.deepEqual(menu.find((m) => m.id === "export:json")!.action, { kind: "export", format: "json" });
  });

  await t.test("buildSessionMenu：已置顶/已归档/已归入时切换文案并提供移出工作区与删除", () => {
    const session = s("a", "2026-10-01T00:00:00Z", {
      pinned_at: "2026-10-01T00:00:00Z",
      archived_at: "2026-10-01T00:00:00Z",
      workspace_id: "w1",
    });
    const menu = buildSessionMenu(
      session,
      [w("w1", "滑坡", "2026-10-01T00:00:00Z"), w("w2", "地震", "2026-10-02T00:00:00Z")],
      { includeDelete: true }
    );
    assert.deepEqual(menu.map((m) => m.label), [
      "重命名",
      "取消置顶",
      "移到工作区",
      "取消归档",
      "导出为 Markdown",
      "导出为 JSON",
      "删除",
    ]);
    const move = menu.find((m) => m.id === "move")!;
    assert.deepEqual(move.items!.map((m) => m.label), ["地震", "移出工作区"], "不列出当前所在工作区");
    assert.equal(menu.at(-1)!.danger, true);
    assert.ok(flattenMenu(menu).some((m) => m.id === "move:none"));
  });

  await t.test("buildSessionMenu：没有可移动目标时省略移到工作区；演示模式只留删除", () => {
    const menu = buildSessionMenu(s("a", "2026-10-01T00:00:00Z"), []);
    assert.ok(!menu.some((m) => m.id === "move"));
    assert.deepEqual(
      buildSessionMenu(s("a", "x"), [], { organize: false, includeDelete: true }).map((m) => m.id),
      ["delete"]
    );
    assert.deepEqual(buildSessionMenu(s("a", "x"), [], { organize: false }), []);
  });

  await t.test("buildWorkspaceMenu：未绑定/已绑定文件夹", () => {
    assert.deepEqual(buildWorkspaceMenu(w("1", "甲", "x")).map((m) => m.label), ["重命名", "设置文件夹", "删除工作区"]);
    assert.deepEqual(buildWorkspaceMenu(w("1", "甲", "x", "D:/data")).map((m) => m.label), [
      "重命名",
      "更换文件夹",
      "解除文件夹",
      "删除工作区",
    ]);
  });

  await t.test("replaceSession / detachWorkspaceSessions", () => {
    const list = [s("a", "1", { workspace_id: "w1" }), s("b", "1", { workspace_id: "w2" })];
    assert.equal(replaceSession(list, { ...list[0], title: "新" })[0].title, "新");
    assert.equal(replaceSession(list, s("c", "1")).length, 3);
    const detached = detachWorkspaceSessions(list, "w1");
    assert.equal(detached[0].workspace_id, null);
    assert.equal(detached[1].workspace_id, "w2");
  });

  await t.test("normalizeRenameInput：空白与未改动不提交", () => {
    assert.equal(normalizeRenameInput("   ", "旧"), null);
    assert.equal(normalizeRenameInput(" 旧 ", "旧"), null);
    assert.deepEqual(normalizeRenameInput("  新标题 ", "旧"), { commit: "新标题" });
  });

  await t.test("exportNoteText 只报文件名；defaultWorkspaceName 避免重名", () => {
    assert.equal(exportNoteText("C:\\out\\会话 1.md"), "已导出 会话 1.md");
    assert.equal(exportNoteText("/tmp/a.json"), "已导出 a.json");
    assert.equal(defaultWorkspaceName([]), "新工作区");
    assert.equal(defaultWorkspaceName([w("1", "新工作区", "x"), w("2", "新工作区 2", "x")]), "新工作区 3");
  });
});
