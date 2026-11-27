import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { useAgentActiveSession, useAgentSessionShell } from "./agentSessionHooks";
import { useAgentActivityNotifier } from "./agentActivityHooks";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { AgentPlanCard } from "./AgentPlanCard";
import { DesktopTitlebar } from "./DesktopTitlebar";
import type {
  StreamConfirmCardItem,
  StreamPlanCardItem,
  StreamProgressCardItem,
} from "../../shell/api/agentStreamLogic";
import type { ApplicationTask } from "../../shell/api/types";
import type { AgentStreamEvent } from "../../lib/generated/v3-contracts";

test.afterEach(() => {
  cleanup();
});

async function settle(ms = 30) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

function ChatChild({
  openSessionId,
  newChatNonce,
  onActiveSessionChange,
}: {
  openSessionId: string | null;
  newChatNonce: number;
  onActiveSessionChange: (sessionId: string | null) => void;
}) {
  const [activeSessionId, setActiveSessionId] = useAgentActiveSession({
    openSessionId,
    newChatNonce,
    onActiveSessionChange,
  });
  return (
    <div>
      <span data-testid="active">{activeSessionId || "(空对话)"}</span>
      <button type="button" onClick={() => setActiveSessionId("sess-1")}>
        建会话1
      </button>
      <button type="button" onClick={() => setActiveSessionId("sess-3")}>
        建会话3
      </button>
    </div>
  );
}

/** 与 DesktopWorkbench 相同的父子接线：openSessionId = 打开的会话 ?? 会话记忆 */
function ChatShell({ inlineCallback }: { inlineCallback: boolean }) {
  const shell = useAgentSessionShell();
  return (
    <div>
      <button type="button" disabled={!shell.hasActiveAgentSession} onClick={shell.startNewChat}>
        新对话
      </button>
      <button type="button" onClick={() => shell.setAgentOpenSessionId("sess-2")}>
        打开历史会话2
      </button>
      <span data-testid="memory">{shell.agentSessionMemory.recall() ?? "(无)"}</span>
      <ChatChild
        openSessionId={shell.agentOpenSessionId ?? shell.agentSessionMemory.recall()}
        newChatNonce={shell.newChatNonce}
        onActiveSessionChange={
          inlineCallback
            ? (sessionId) => shell.handleActiveSessionChange(sessionId)
            : shell.handleActiveSessionChange
        }
      />
    </div>
  );
}

test("W5-8 R9 P0：有会话时点「新对话」得到空对话且不回弹", async (t) => {
  for (const inlineCallback of [false, true]) {
    await t.test(inlineCallback ? "父组件传内联回调" : "父组件传稳定回调", async () => {
      const view = render(<ChatShell inlineCallback={inlineCallback} />);
      fireEvent.click(view.getByText("建会话1"));
      await settle();
      assert.equal(view.getByTestId("active").textContent, "sess-1");
      const newChat = view.getByRole("button", { name: "新对话" }) as HTMLButtonElement;
      assert.equal(newChat.disabled, false);

      fireEvent.click(newChat);
      await settle(80);
      assert.equal(
        view.getByTestId("active").textContent,
        "(空对话)",
        "点「新对话」后不得把原会话重新打开",
      );
      assert.equal((view.getByRole("button", { name: "新对话" }) as HTMLButtonElement).disabled, true);

      // 新对话里再建会话，照常上报
      fireEvent.click(view.getByText("建会话3"));
      await settle();
      assert.equal(view.getByTestId("active").textContent, "sess-3");
      assert.equal((view.getByRole("button", { name: "新对话" }) as HTMLButtonElement).disabled, false);
    });
  }

  await t.test("从历史打开会话后点「新对话」同样不回弹", async () => {
    const view = render(<ChatShell inlineCallback />);
    fireEvent.click(view.getByText("打开历史会话2"));
    await settle();
    assert.equal(view.getByTestId("active").textContent, "sess-2");
    fireEvent.click(view.getByRole("button", { name: "新对话" }));
    await settle(80);
    assert.equal(view.getByTestId("active").textContent, "(空对话)");
  });

  await t.test("活动会话不变时不重复上报", async () => {
    const reports: Array<string | null> = [];
    function Probe({ tick }: { tick: number }) {
      useAgentActiveSession({
        openSessionId: "sess-9",
        newChatNonce: 0,
        onActiveSessionChange: (id) => reports.push(id),
      });
      return <span>{tick}</span>;
    }
    const view = render(<Probe tick={0} />);
    view.rerender(<Probe tick={1} />);
    view.rerender(<Probe tick={2} />);
    await settle();
    assert.deepEqual(reports, ["sess-9"], `实际：${JSON.stringify(reports)}`);
  });
});

const RUN_ID = "run-r9-1";
const PLAN_ID = "plan-r9-1";
const TASK_ID = "task-unified-0bc1f674-2a95-49d6-b390-6616c0e62719";

const confirmItem: StreamConfirmCardItem = {
  kind: "confirm_card",
  seq: 50,
  run_id: RUN_ID,
  plan_id: PLAN_ID,
  plan_hash: "hash-r9-1",
  snapshot: {
    source: "asf.catalog",
    range: { aoi_name: "地图绘制矩形", area_km2: 125.6, product: "1 个产品", time_range: null },
    estimated_bytes: 4664020214,
    output_dir: null,
    output_dir_source: null,
    already_queued_count: 0,
  },
  approval: {
    decision: "manual",
    mode: "confirm_changes",
    card_reasons: ["output_dir_unknown", "selection_not_from_run"],
  },
  auto_executed: false,
};

const doneProgress: StreamProgressCardItem = {
  kind: "progress_card",
  seq: 53,
  run_id: RUN_ID,
  plan_id: PLAN_ID,
  step_index: 0,
  tool_id: "create_task",
  status: "done",
  code: null,
  created_task_ids: [TASK_ID],
};

function cancelledTask(outputDir: string | null): ApplicationTask {
  return {
    task_id: TASK_ID,
    revision: 4,
    category: "scene_download",
    name: "Sentinel-1",
    provider: "asf",
    dataset: "s1_slc",
    status: "cancelled",
    items_count: 1,
    done_bytes: 0,
    total_bytes: null,
    output_dir: outputDir,
    created_at: "2026-10-27T04:05:31Z",
    updated_at: "2026-10-27T04:07:00Z",
  } as ApplicationTask;
}

function dirRowText(view: ReturnType<typeof render>): string {
  const dt = view.getByText("下载目录");
  return dt.nextElementSibling?.textContent ?? "";
}

test("W5-8 R9 P2-1：已批准卡显示批准时使用的目录", async (t) => {
  await t.test("批准时卡上选了目录 → 重新挂载后仍显示该目录，不再显示「下载目录未知」", () => {
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        sourceLabel="Sentinel-1"
        approvedOutputDir={"D:\\ScenePort\\downloads"}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[]}
      />,
    );
    assert.ok(view.getByText("已批准"));
    const row = dirRowText(view);
    assert.ok(row.includes("D:\\ScenePort\\downloads"), `下载目录行实际：${row}`);
    assert.equal(row.includes("未选择"), false, `下载目录行实际：${row}`);
    assert.equal(
      (view.container.textContent ?? "").includes("下载目录未知"),
      false,
      `实际：${view.container.textContent}`,
    );
    assert.ok(view.getByText("所选产品不是本次运行检索出的"), "其他原因保留");
  });

  await t.test("没有批准记录（如重启后）、任务也不带目录（生产数据形状）→「未知」而非「未选择」", () => {
    // production.ts 的任务映射不设 output_dir（W5-8 R10 F1）
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        sourceLabel="Sentinel-1"
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[cancelledTask(null)]}
      />,
    );
    const row = dirRowText(view);
    assert.ok(row.includes("未知"), `下载目录行实际：${row}`);
    assert.equal(row.includes("未选择"), false, `下载目录行实际：${row}`);
    assert.equal(
      (view.container.textContent ?? "").includes("下载目录未知，请在卡上选择"),
      false,
      `已批准卡不得列「下载目录未知，请在卡上选择」，实际：${view.container.textContent}`,
    );
    assert.ok(view.getByText("所选产品不是本次运行检索出的"), "其他原因保留");
  });

  await t.test("待批卡不受批准记录影响", () => {
    const view = render(
      <AgentConfirmCard item={confirmItem} sourceLabel="Sentinel-1" approvedOutputDir={"D:\\X"} />,
    );
    assert.ok(dirRowText(view).includes("未选择"));
    assert.ok(view.getByText("下载目录未知，请在卡上选择"));
  });
});

test("W5-8 R9 P2-3：取消后的内嵌任务卡不显示任务 ID", () => {
  const view = render(
    <AgentConfirmCard
      item={confirmItem}
      sourceLabel="Sentinel-1"
      subsequentProgress={doneProgress}
      createdTaskIds={[TASK_ID]}
      agentTasks={[cancelledTask(null)]}
    />,
  );
  assert.ok(view.getByText("已取消"));
  assert.equal(
    (view.container.textContent ?? "").includes(TASK_ID),
    false,
    `实际：${view.container.textContent}`,
  );
});

test("W5-8 R9 P2-2：建任务步骤完成显示「已完成」", () => {
  const item: StreamPlanCardItem = {
    kind: "plan_card",
    seq: 49,
    run_id: RUN_ID,
    plan_id: PLAN_ID,
    plan_hash: "hash-r9-0",
    max_tier: 2,
    steps: [{ tool_id: "create_task", label_zh: "为 2026-10-06 的 Sentinel-1 SLC 建下载任务", tier: 2, status: "pending" }],
  };
  const view = render(<AgentPlanCard item={item} progressStatus="done" />);
  assert.ok(view.getByText("已完成"));
  assert.equal(view.queryByText("已就绪") === null, true);
});

test("W5-8 R9 P2-4：禁用态「全部停止」文字用对比度足够的颜色", () => {
  const view = render(
    <DesktopTitlebar
      stopAll={{ enabled: false, reason_zh: "没有进行中的 Agent 运行或 Agent 创建的进行中任务", busy: false, onClick: () => {} }}
    />,
  );
  const btn = view.getByRole("button", { name: "全部停止" });
  assert.ok(btn.className.includes("text-sp-text-muted"), `实际 class：${btn.className}`);
  assert.equal(btn.className.includes("text-sp-text-disabled"), false, `实际 class：${btn.className}`);
  assert.equal(/\bopacity-\d/.test(btn.className), false, `禁用态不得再叠加透明度，实际 class：${btn.className}`);
});

function activityEvent(seq: number, liveRuns: number): AgentStreamEvent {
  return {
    seq,
    kind: "activity_changed",
    payload: { live_runs: liveRuns, agent_active_tasks: 0, sessions_with_live_runs: [] },
  } as unknown as AgentStreamEvent;
}

function plainEvent(seq: number, kind: string): AgentStreamEvent {
  return { seq, kind, payload: {} } as unknown as AgentStreamEvent;
}

function NotifierProbe(props: {
  sessionId: string;
  activeRunId: string | null;
  events: AgentStreamEvent[];
  onActivityChanged: () => void;
}) {
  useAgentActivityNotifier(props);
  return null;
}

test("W5-8 R9 P1：流里新到的 activity_changed 与运行开始/结束都通知上层重读活动状态", async (t) => {
  await t.test("L4 E10 序列：发送后同一页带回 activity_changed(live_runs 1) → 通知；同批不重复", async () => {
    let calls = 0;
    const onActivityChanged = () => {
      calls += 1;
    };
    const base = { sessionId: "sess-e10", onActivityChanged };
    const view = render(<NotifierProbe {...base} activeRunId={null} events={[]} />);
    await settle();
    const afterMount = calls;

    // 发送后：运行开始（本地 activeRunId 变化）
    view.rerender(<NotifierProbe {...base} activeRunId="run-21" events={[]} />);
    await settle();
    assert.equal(calls, afterMount + 1, "运行开始（本地 liveRunIds 变化）必须通知重读");

    // 立即拉取的一页：message + activity_changed(live 1)
    const page1 = [plainEvent(9, "message"), activityEvent(10, 1)];
    view.rerender(<NotifierProbe {...base} activeRunId="run-21" events={page1} />);
    await settle();
    assert.equal(calls, afterMount + 2, "新到的 activity_changed 必须通知重读");

    // 同一批事件再渲染、或只来了别的事件：不再通知
    const page2 = [...page1, plainEvent(11, "tool_round"), plainEvent(12, "tool_round")];
    view.rerender(<NotifierProbe {...base} activeRunId="run-21" events={page2} />);
    await settle();
    assert.equal(calls, afterMount + 2, "没有新的 activity_changed 时不得重复通知");

    // 运行结束：activity_changed(live 0) + conclusion，本地 activeRunId 清空
    const page3 = [...page2, activityEvent(15, 0), plainEvent(16, "conclusion")];
    view.rerender(<NotifierProbe {...base} activeRunId={null} events={page3} />);
    await settle();
    assert.equal(calls >= afterMount + 3, true, `运行结束必须通知重读，实际 ${calls - afterMount}`);
  });

  await t.test("换会话后按会话重新计序号", async () => {
    let calls = 0;
    const onActivityChanged = () => {
      calls += 1;
    };
    const view = render(
      <NotifierProbe sessionId="sess-a" activeRunId={null} events={[activityEvent(20, 0)]} onActivityChanged={onActivityChanged} />,
    );
    await settle();
    view.rerender(<NotifierProbe sessionId="sess-b" activeRunId={null} events={[]} onActivityChanged={onActivityChanged} />);
    await settle();
    const beforeHistory = calls;
    // 新会话的事件序号比旧会话小，也必须通知
    view.rerender(
      <NotifierProbe sessionId="sess-b" activeRunId={null} events={[activityEvent(2, 1)]} onActivityChanged={onActivityChanged} />,
    );
    await settle();
    assert.equal(calls, beforeHistory + 1, "新会话里的 activity_changed 序号从头计，不能被旧会话的序号挡住");
  });
});
