import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { AgentActivityStatus } from "../../lib/generated/v3-contracts.ts";
import { stopAllAvailability } from "./stopAllLogic.ts";
import { taskListSignature } from "./taskListSignature.ts";
import type { ApplicationTask } from "./types.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-7 R2 全部停止全局活动状态契约与接线测试", async (suite) => {
  await suite.test("1. status (0,0) with stale stream liveRunIds ['run-1'] -> disabled", () => {
    const status: AgentActivityStatus = {
      live_runs: 0,
      agent_active_tasks: 0,
      sessions_with_live_runs: [],
    };
    const res = stopAllAvailability({
      liveRunIds: ["run-1"],
      agentTaskIds: [],
      status,
    });
    assert.strictEqual(res.enabled, false);
    assert.strictEqual(res.reason_zh, "没有进行中的 Agent 运行或 Agent 创建的进行中任务");
    assert.strictEqual(res.live_runs, 0);
    assert.strictEqual(res.agent_active_tasks, 0);
  });

  await suite.test("2. status (0,2) with no stream info -> enabled", () => {
    const status: AgentActivityStatus = {
      live_runs: 0,
      agent_active_tasks: 2,
      sessions_with_live_runs: [],
    };
    const res = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: [],
      status,
    });
    assert.strictEqual(res.enabled, true);
    assert.strictEqual(res.reason_zh, "");
    assert.strictEqual(res.live_runs, 0);
    assert.strictEqual(res.agent_active_tasks, 2);
  });

  await suite.test("3. status (1,0) -> enabled", () => {
    const status: AgentActivityStatus = {
      live_runs: 1,
      agent_active_tasks: 0,
      sessions_with_live_runs: ["sess-1"],
    };
    const res = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: [],
      status,
    });
    assert.strictEqual(res.enabled, true);
    assert.strictEqual(res.reason_zh, "");
    assert.strictEqual(res.live_runs, 1);
    assert.strictEqual(res.agent_active_tasks, 0);
  });

  await suite.test("4. status null -> legacy behavior (stream-derived)", () => {
    const resEnabled = stopAllAvailability({
      liveRunIds: ["run-1"],
      agentTaskIds: [],
      status: null,
    });
    assert.strictEqual(resEnabled.enabled, true);
    assert.strictEqual(resEnabled.reason_zh, "");
    assert.strictEqual(resEnabled.live_runs, undefined);
    assert.strictEqual(resEnabled.agent_active_tasks, undefined);

    const resDisabled = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: [],
      status: null,
    });
    assert.strictEqual(resDisabled.enabled, false);
    assert.strictEqual(resDisabled.reason_zh, "没有进行中的 Agent 运行或 Agent 创建的进行中任务");
  });

  await suite.test("5. result carries counts when status is present", () => {
    const status: AgentActivityStatus = {
      live_runs: 3,
      agent_active_tasks: 5,
      sessions_with_live_runs: ["s1", "s2"],
    };
    const res = stopAllAvailability({
      liveRunIds: [],
      agentTaskIds: [],
      status,
    });
    assert.strictEqual(res.live_runs, 3);
    assert.strictEqual(res.agent_active_tasks, 5);
  });

  await suite.test("6. 源码静态检查：桌面桥接与各端代理边界", () => {
    const tauriBridgePath = path.resolve(here, "../../lib/desktopShellBridgeTauri.ts");
    const agentBridgeTypesPath = path.resolve(here, "agentBridgeTypes.ts");
    const fixtureBridgePath = path.resolve(here, "../../lib/desktopShellBridgeFixture.ts");
    const pywebviewBridgePath = path.resolve(here, "../../lib/desktopShellBridgePywebview.ts");

    const tauriSrc = fs.readFileSync(tauriBridgePath, "utf-8");
    const typesSrc = fs.readFileSync(agentBridgeTypesPath, "utf-8");
    const fixtureSrc = fs.readFileSync(fixtureBridgePath, "utf-8");
    const pywebviewSrc = fs.readFileSync(pywebviewBridgePath, "utf-8");

    assert.ok(
      tauriSrc.includes('invoke("agent_activity_status")'),
      'desktopShellBridgeTauri.ts must contain invoke("agent_activity_status")'
    );

    assert.ok(
      typesSrc.includes("activityStatus("),
      "agentBridgeTypes.ts must contain activityStatus("
    );

    assert.ok(
      !fixtureSrc.includes("activityStatus"),
      "desktopShellBridgeFixture.ts must contain no activityStatus"
    );
    assert.ok(
      !pywebviewSrc.includes("activityStatus"),
      "desktopShellBridgePywebview.ts must contain no activityStatus"
    );
  });

  await suite.test("7. 源码静态检查：DesktopWorkbench 与 DesktopAgentWorkbench 接线标记", () => {
    const workbenchPath = path.resolve(here, "../../components/desktop/DesktopWorkbench.tsx");
    const agentWorkbenchPath = path.resolve(here, "../../components/desktop/DesktopAgentWorkbench.tsx");

    const workbenchSrc = fs.readFileSync(workbenchPath, "utf-8");
    const agentWorkbenchSrc = fs.readFileSync(agentWorkbenchPath, "utf-8");

    // DesktopWorkbench contains three wiring markers:
    // (a) mount-time call
    assert.match(
      workbenchSrc,
      /useEffect\(\s*\(\)\s*=>\s*\{[\s\S]*?refreshAgentActivityStatus\(\)[\s\S]*?\},\s*\[.*?refreshAgentActivityStatus.*?\]\)/,
      "DesktopWorkbench.tsx must contain mount-time call for refreshAgentActivityStatus"
    );

    // (b) onActivityChanged-triggered call
    assert.match(
      workbenchSrc,
      /<DesktopAgentWorkbench[\s\S]*?onActivityChanged=\{refreshAgentActivityStatus\}/,
      "DesktopWorkbench.tsx must pass onActivityChanged={refreshAgentActivityStatus} to DesktopAgentWorkbench"
    );

    // (c) call after refreshTasksList
    const refreshTasksMatch = workbenchSrc.match(
      /const refreshTasksList = useCallback\(async \(\) => \{([\s\S]*?)\r?\n  \},/
    );
    assert.ok(refreshTasksMatch, "DesktopWorkbench.tsx must define refreshTasksList");
    assert.ok(
      refreshTasksMatch[1].includes("refreshAgentActivityStatus()"),
      "DesktopWorkbench.tsx must call refreshAgentActivityStatus() after refreshing tasks in refreshTasksList"
    );

    // (c2) the task-overview snapshot handler re-reads activity status when the task list signature changes
    const pollerMatch = workbenchSrc.match(
      /const handleTaskSnapshot = \(list: ApplicationTask\[\]\) => \{([\s\S]*?)\r?\n    \};/
    );
    assert.ok(pollerMatch, "DesktopWorkbench.tsx must define handleTaskSnapshot");
    const pollerBody = pollerMatch[1];
    const setTasksAt = pollerBody.indexOf("setC1Tasks(list)");
    const signatureAt = pollerBody.indexOf("taskListSignature(list)");
    const refreshAt = pollerBody.indexOf("refreshAgentActivityStatus()");
    assert.ok(setTasksAt >= 0, "handleTaskSnapshot must call setC1Tasks(list)");
    assert.ok(
      signatureAt > setTasksAt,
      "handleTaskSnapshot must compute taskListSignature(list) after setC1Tasks(list)"
    );
    assert.ok(
      refreshAt > signatureAt,
      "handleTaskSnapshot must call refreshAgentActivityStatus() when the task list signature changes"
    );

    // DesktopAgentWorkbench contains activity_changed handling wired to callback
    assert.ok(
      agentWorkbenchSrc.includes("onActivityChanged"),
      "DesktopAgentWorkbench.tsx must define onActivityChanged prop"
    );
    // W5-8 R9：所有拉取路径收到的 activity_changed 统一经 useAgentActivityNotifier 通知（布尔断言，避免失败时打出整份源码）
    assert.equal(
      /useAgentActivityNotifier\(\{[\s\S]{0,200}?onActivityChanged,/.test(agentWorkbenchSrc),
      true,
      "DesktopAgentWorkbench.tsx must pass onActivityChanged to useAgentActivityNotifier"
    );
    const notifierSrc = fs.readFileSync(
      path.resolve(here, "../../components/desktop/agentActivityHooks.ts"),
      "utf-8"
    );
    assert.equal(
      notifierSrc.includes('"activity_changed"'),
      true,
      "agentActivityHooks.ts must react to activity_changed events"
    );
  });
});

test("W5-7 任务列表签名 taskListSignature", async (suite) => {
  const task = (task_id: string, status: ApplicationTask["status"], revision: number) =>
    ({ task_id, status, revision, done_bytes: 0 }) as unknown as ApplicationTask;

  await suite.test("same (task_id, status, revision) in any order -> same signature", () => {
    const a = [task("t1", "running", 3), task("t2", "queued", 1)];
    const b = [task("t2", "queued", 1), task("t1", "running", 3)];
    assert.strictEqual(taskListSignature(a), taskListSignature(b));
  });

  await suite.test("fields outside (task_id, status, revision) do not change the signature", () => {
    const a = [task("t1", "running", 3)];
    const b = [{ ...task("t1", "running", 3), done_bytes: 999, name: "x" } as ApplicationTask];
    assert.strictEqual(taskListSignature(a), taskListSignature(b));
  });

  await suite.test("status, revision, added or removed task -> different signature", () => {
    const base = taskListSignature([task("t1", "running", 3)]);
    assert.notStrictEqual(taskListSignature([task("t1", "completed", 3)]), base);
    assert.notStrictEqual(taskListSignature([task("t1", "running", 4)]), base);
    assert.notStrictEqual(
      taskListSignature([task("t1", "running", 3), task("t2", "queued", 1)]),
      base
    );
    assert.notStrictEqual(taskListSignature([]), base);
  });

  await suite.test("empty, null and undefined lists give the empty signature", () => {
    assert.strictEqual(taskListSignature([]), "");
    assert.strictEqual(taskListSignature(null), "");
    assert.strictEqual(taskListSignature(undefined), "");
  });
});
