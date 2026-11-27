import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { useAgentActiveSession, useAgentSessionShell, type SessionGuard } from "./agentSessionHooks";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { ScenePopup } from "../WorkbenchMap";
import type {
  StreamConfirmCardItem,
  StreamProgressCardItem,
} from "../../shell/api/agentStreamLogic";
import type { SceneRow } from "../../lib/bridge";

test.afterEach(() => {
  cleanup();
});

async function settle(ms = 30) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

/** 模拟 loadSessionHistory / 轮询 / refreshSessions：先记下会话，等异步结果回来再按核对结果写入 */
function LateWriterChild({
  openSessionId,
  newChatNonce,
  onActiveSessionChange,
  pending,
}: {
  openSessionId: string | null;
  newChatNonce: number;
  onActiveSessionChange: (sessionId: string | null) => void;
  pending: { resolve: (() => void) | null };
}) {
  const [activeSessionId, setActiveSessionId, guard] = useAgentActiveSession({
    openSessionId,
    newChatNonce,
    onActiveSessionChange,
  });
  const [history, setHistory] = React.useState<string>("(无历史)");
  const startLateLoad = (g: SessionGuard) => {
    const token = g.capture();
    new Promise<void>((resolve) => {
      pending.resolve = resolve;
    }).then(() => {
      if (!g.stillCurrent(token)) return;
      setHistory(`${token.sessionId} 的历史`);
      setActiveSessionId(token.sessionId);
    });
  };
  return (
    <div>
      <span data-testid="active">{activeSessionId || "(空对话)"}</span>
      <span data-testid="history">{history}</span>
      <button type="button" onClick={() => setActiveSessionId("sess-1")}>
        建会话1
      </button>
      <button type="button" onClick={() => startLateLoad(guard)}>
        开始慢加载
      </button>
    </div>
  );
}

function LateWriterShell({ pending }: { pending: { resolve: (() => void) | null } }) {
  const shell = useAgentSessionShell();
  return (
    <div>
      <button type="button" onClick={shell.startNewChat}>
        新对话
      </button>
      <LateWriterChild
        openSessionId={shell.agentOpenSessionId ?? shell.agentSessionMemory.recall()}
        newChatNonce={shell.newChatNonce}
        onActiveSessionChange={shell.handleActiveSessionChange}
        pending={pending}
      />
    </div>
  );
}

test("W5-8 R10 F3：新对话后迟到的旧会话结果不得写进空对话", async (t) => {
  await t.test("慢加载在点「新对话」之后才返回 → 丢弃", async () => {
    const pending: { resolve: (() => void) | null } = { resolve: null };
    const view = render(<LateWriterShell pending={pending} />);
    fireEvent.click(view.getByText("建会话1"));
    await settle();
    fireEvent.click(view.getByText("开始慢加载"));
    fireEvent.click(view.getByText("新对话"));
    await settle();
    assert.equal(view.getByTestId("active").textContent, "(空对话)");

    await act(async () => {
      pending.resolve?.();
      await new Promise((resolve) => setTimeout(resolve, 30));
    });
    assert.equal(view.getByTestId("active").textContent, "(空对话)", "迟到的旧会话结果不得重新打开旧会话");
    assert.equal(view.getByTestId("history").textContent, "(无历史)", "迟到的旧会话历史不得写进空对话");
  });

  await t.test("没有切换时照常写入", async () => {
    const pending: { resolve: (() => void) | null } = { resolve: null };
    const view = render(<LateWriterShell pending={pending} />);
    fireEvent.click(view.getByText("建会话1"));
    await settle();
    fireEvent.click(view.getByText("开始慢加载"));
    await act(async () => {
      pending.resolve?.();
      await new Promise((resolve) => setTimeout(resolve, 30));
    });
    assert.equal(view.getByTestId("history").textContent, "sess-1 的历史");
  });
});

const confirmItem: StreamConfirmCardItem = {
  kind: "confirm_card",
  seq: 60,
  run_id: "run-r10",
  plan_id: "plan-r10",
  plan_hash: "hash-r10",
  snapshot: {
    source: "asf.catalog",
    range: { aoi_name: "地图绘制矩形", area_km2: 125.6, product: "1 个产品", time_range: null },
    estimated_bytes: null,
    output_dir: null,
    output_dir_source: null,
    already_queued_count: 0,
  },
  approval: { decision: "manual", mode: "confirm_changes", card_reasons: ["output_dir_unknown"] },
  auto_executed: false,
};

const doneProgress: StreamProgressCardItem = {
  kind: "progress_card",
  seq: 61,
  run_id: "run-r10",
  plan_id: "plan-r10",
  step_index: 0,
  tool_id: "create_task",
  status: "done",
  code: null,
  created_task_ids: [],
};

test("W5-8 R10 F1：已批准卡目录未知时写「未知」，且不列「下载目录未知」", () => {
  const view = render(
    <AgentConfirmCard item={confirmItem} sourceLabel="Sentinel-1" subsequentProgress={doneProgress} />,
  );
  assert.ok(view.getByText("已批准"));
  const row = view.getByText("下载目录").nextElementSibling?.textContent ?? "";
  assert.ok(row.includes("未知"), `下载目录行实际：${row}`);
  assert.equal(row.includes("未选择"), false, `下载目录行实际：${row}`);
  const text = view.container.textContent ?? "";
  // 只查原因全文：「下载目录」行写「未知」时文本会拼成「下载目录未知」
  assert.equal(text.includes("下载目录未知，请在卡上选择"), false, `实际：${text}`);
  assert.equal(text.includes("需要你确认的原因"), false, `只有该原因时整块不显示，实际：${text}`);
});

const bareScene: SceneRow = {
  scene_id: "G1234567890-ASF",
  product_key: "G1234567890-ASF",
  product_id: "G1234567890-ASF",
  provider: "asf.nisar",
  file_name: "NISAR_L1_PR_RSLC_001.h5",
  has_url: false,
} as SceneRow;

test("W5-8 R10 F5：新外壳足迹弹窗缺失的时间与大小写「未知」", async (t) => {
  await t.test("新外壳模式", () => {
    const view = render(
      <ScenePopup scene={bareScene} index={0} titleMode="display_name" hideDownloadEntry />,
    );
    const time = view.getByText("时间").nextElementSibling?.textContent ?? "";
    const size = view.getByText("大小").nextElementSibling?.textContent ?? "";
    assert.equal(time, "未知");
    assert.equal(size, "未知");
  });

  await t.test("旧界面默认模式不变", () => {
    const view = render(<ScenePopup scene={bareScene} index={0} />);
    assert.equal(view.getByText("时间").nextElementSibling?.textContent, "-");
    assert.equal(view.getByText("大小").nextElementSibling?.textContent, "-");
  });
});
