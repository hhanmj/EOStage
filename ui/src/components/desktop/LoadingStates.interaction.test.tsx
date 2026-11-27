import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, waitFor, within } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider } from "../../i18n";
import {
  useCancellableAction,
  useDelayedFlag,
  useFullTurnSpin,
} from "../../hooks/useLoadingPrimitives";
import { TopCenterToastHost, clearAllTopToastsForTest, emitTopToast } from "../ui/TopCenterToast";
import { ProgressNotificationHost } from "../ui/ProgressNotification";
import { clearAllLongWorkForTest, completeLongWork, startLongWork } from "../../lib/longWorkRegistry";
import { AgentToolGroupCard } from "./AgentToolGroupCard";
import { AgentMarkdownRenderer } from "./agent/AgentMarkdownRenderer";
import { SidebarSessionTree } from "./SidebarSessionTree";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "../../lib/desktopShellBridge";
import type { AgentSessionSummary } from "../../lib/generated/v3-contracts";
import type { ApplicationTask } from "../../shell/api/types";

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

test.afterEach(() => {
  cleanup();
  clearAllTopToastsForTest();
  clearAllLongWorkForTest();
  clearTestDesktopShellBridge();
});

function DelayedFlagProbe({ active }: { active: boolean }) {
  const show = useDelayedFlag(active, 80, 120);
  return <div data-testid="probe-state">{show ? "visible" : "hidden"}</div>;
}

function FullTurnSpinProbe({ active }: { active: boolean }) {
  const { spinning, spinClass } = useFullTurnSpin(active, 120);
  return (
    <button type="button" data-testid="spin-probe" data-spinning={spinning ? "true" : "false"} className={spinClass}>
      同步
    </button>
  );
}

test("useDelayedFlag suppresses fast sub-delay waits and holds minimum visible duration once shown", async () => {
  // Fast operation finishes before delay -> never flickers visible.
  const view = render(<DelayedFlagProbe active={true} />);
  assert.equal(view.getByTestId("probe-state").textContent, "hidden");
  await sleep(40);
  view.rerender(<DelayedFlagProbe active={false} />);
  await sleep(70);
  assert.equal(view.getByTestId("probe-state").textContent, "hidden");

  // Longer operation crosses delay threshold -> becomes visible, then holds >= minVisibleMs after stop.
  view.rerender(<DelayedFlagProbe active={true} />);
  await waitFor(() => {
    assert.equal(view.getByTestId("probe-state").textContent, "visible");
  });
  view.rerender(<DelayedFlagProbe active={false} />);
  await sleep(40);
  assert.equal(view.getByTestId("probe-state").textContent, "visible");
  await waitFor(() => {
    assert.equal(view.getByTestId("probe-state").textContent, "hidden");
  });
});

test("useFullTurnSpin completes full rotation and TopCenterToastHost renders + auto-dismisses", async () => {
  const spinView = render(<FullTurnSpinProbe active={true} />);
  assert.equal(spinView.getByTestId("spin-probe").dataset.spinning, "true");
  assert.ok(spinView.getByTestId("spin-probe").className.includes("sp-refresh-full-turn"));
  await sleep(30);
  spinView.rerender(<FullTurnSpinProbe active={false} />);
  // Still mid-turn at 30ms -> continues spinning until turn completes at 120ms.
  assert.equal(spinView.getByTestId("spin-probe").dataset.spinning, "true");
  await waitFor(() => {
    assert.equal(spinView.getByTestId("spin-probe").dataset.spinning, "false");
  });

  const toastView = render(<TopCenterToastHost />);
  act(() => {
    emitTopToast({ text: "模型清单已同步", tone: "success", durationMs: 120 });
  });
  const toast = await waitFor(() => toastView.getByTestId("top-center-toast"));
  assert.equal(toast.getAttribute("role"), "status");
  assert.ok(toast.textContent?.includes("模型清单已同步"));
  await waitFor(() => {
    assert.equal(toastView.queryByTestId("top-center-toast") === null, true);
  });
});

test("ProgressNotificationHost hides short tasks (<400ms), shows ETA, supports cancel and click-to-open", async () => {
  let cancelledTaskId: string | null = null;
  let openedTaskId: string | null = null;
  let openedWork = false;

  const shortTask: ApplicationTask = {
    task_id: "fast-1",
    revision: 1,
    category: "download",
    name: "快速小文件",
    status: "running",
    done_bytes: 50,
    total_bytes: 100,
    created_at: new Date().toISOString(),
  };

  const view = render(
    <ProgressNotificationHost
      tasks={[shortTask]}
      pendingPreparations={[]}
      onCancelTask={(t) => {
        cancelledTaskId = t.task_id;
      }}
      onOpenTask={(id) => {
        openedTaskId = id;
      }}
    />
  );

  // <400ms running task is suppressed silently.
  assert.equal(view.queryByTestId("progress-notification-card") === null, true);

  // Elapsed >400ms & <1s shows "估算中…".
  const warmingTask: ApplicationTask = {
    ...shortTask,
    task_id: "warm-1",
    name: "Landsat 景集下载",
    created_at: new Date(Date.now() - 550).toISOString(),
    bytes_per_second: 10,
  };
  view.rerender(
    <ProgressNotificationHost
      tasks={[warmingTask]}
      pendingPreparations={[]}
      onCancelTask={(t) => {
        cancelledTaskId = t.task_id;
      }}
      onOpenTask={(id) => {
        openedTaskId = id;
      }}
    />
  );
  const warmingCard = await waitFor(() => view.getByTestId("progress-notification-card"));
  assert.ok(warmingCard.textContent?.includes("估算中…"));

  // Elapsed >1s switches to remaining ETA text and exposes Cancel button.
  const countedTask: ApplicationTask = {
    ...warmingTask,
    task_id: "long-1",
    done_bytes: 200,
    total_bytes: 1000,
    bytes_per_second: 100,
    created_at: new Date(Date.now() - 2500).toISOString(),
  };
  view.rerender(
    <ProgressNotificationHost
      tasks={[countedTask]}
      pendingPreparations={[]}
      onCancelTask={(t) => {
        cancelledTaskId = t.task_id;
      }}
      onOpenTask={(id) => {
        openedTaskId = id;
      }}
    />
  );
  await waitFor(() => {
    assert.ok(view.getByTestId("progress-notification-card").textContent?.includes("约 8 秒剩余"));
  });
  const cardHtml = view.getByTestId("progress-notification-card").innerHTML;
  assert.ok(cardHtml.includes("text-[12px]"), "ProgressNotificationHost body/cancel/eta copy must use >=12px");
  assert.equal(cardHtml.includes("text-[11px]"), false, "ProgressNotificationHost must not contain 11px text");
  fireEvent.click(within(view.getByTestId("progress-notification-card")).getByRole("button", { name: "取消" }));
  assert.equal(cancelledTaskId, "long-1");

  // LongWork completion morphs in place and opens result when clicked.
  act(() => {
    const workId = startLongWork({
      title: "导出会话 (MD)",
      category: "export",
      current: 1,
      total: 4,
      startedAtMs: Date.now() - 1200,
    });
    completeLongWork(workId, {
      doneSubtitle: "已导出 session.md",
      onOpen: () => {
        openedWork = true;
      },
    });
  });
  await waitFor(() => {
    const cards = view.getAllByTestId("progress-notification-card");
    const completed = cards.find((el) => el.getAttribute("data-notification-status") === "completed");
    assert.ok(completed);
    fireEvent.click(within(completed!).getByRole("button", { name: "导出会话 (MD)" }));
  });
  assert.equal(openedWork, true);
  assert.equal(openedTaskId, null);

  // Completed LongWork notifications auto-dismiss after COMPLETED_AUTO_DISMISS_MS (4500ms)
  // rather than remaining pinned permanently over top-right action buttons.
  const realNow = Date.now;
  try {
    const futureMs = realNow() + 5000;
    Date.now = () => futureMs;
    view.rerender(<ProgressNotificationHost tasks={[]} pendingPreparations={[]} />);
    await waitFor(() => {
      assert.equal(view.queryAllByTestId("progress-notification-card").length, 0);
    });
  } finally {
    Date.now = realNow;
  }
});

test("SidebarSessionTree delays skeleton (>=300ms), staggers rows, and rolls back failed rename with retry", async () => {
  const nowIso = new Date().toISOString();
  let sessionsState: AgentSessionSummary[] = [
    {
      session_id: "s-1",
      title: "原始会话标题",
      created_at: nowIso,
      updated_at: nowIso,
      run_ids: [],
    },
  ];
  let resolveInitial!: (list: AgentSessionSummary[]) => void;
  const initialPromise = new Promise<AgentSessionSummary[]>((res) => {
    resolveInitial = res;
  });
  let renameAttempts = 0;

  registerTestDesktopShellBridge({
    kind: "tauri",
    agent: {
      sessionsList: () => initialPromise,
      workspacesList: async () => [],
      sessionsRename: async (sessionId, title) => {
        renameAttempts += 1;
        if (renameAttempts === 1) {
          throw new Error("network error");
        }
        sessionsState = sessionsState.map((s) =>
          s.session_id === sessionId ? { ...s, title, updated_at: new Date().toISOString() } : s
        );
        return sessionsState[0];
      },
      sessionsSetPinned: async () => sessionsState[0],
      sessionsSetArchived: async () => sessionsState[0],
      sessionsMove: async () => sessionsState[0],
      sessionsCreate: async () => sessionsState[0],
      sessionsExportTo: async () => "out.md",
      sessionsDelete: async () => undefined,
      workspacesCreate: async () => {
        throw new Error("unused");
      },
      workspacesUpdate: async () => {
        throw new Error("unused");
      },
      workspacesDelete: async () => undefined,
    } as unknown as DesktopShellBridge["agent"],
    dialog: {} as DesktopShellBridge["dialog"],
  });

  const view = render(
    <I18nProvider>
      <SidebarSessionTree activeRoute="agent" activeSessionId="s-1" onOpenSession={() => {}} onNavigate={() => {}} />
    </I18nProvider>
  );

  // Immediately (<300ms): header stays visible, neither skeleton nor premature empty state renders.
  assert.ok(view.getByText("最近"));
  assert.equal(view.queryByTestId("session-row-skeletons") === null, true);
  assert.equal(view.queryByText(/暂无独立对话/) === null, true);

  // After 300ms while still pending: skeleton shapes appear.
  await sleep(340);
  await waitFor(() => {
    assert.ok(view.getByTestId("session-row-skeletons"));
  });

  // Resolve initial sessions -> staggered row renders.
  await act(async () => {
    resolveInitial(sessionsState);
  });
  const rowEl = await waitFor(() => view.container.querySelector('[data-session-row="s-1"]') as HTMLElement);
  assert.ok(rowEl.className.includes("sp-stagger-fade-row"));

  // Trigger inline rename, submit "失败后保留的草稿": first backend attempt fails, restores draft & offers Retry.
  fireEvent.click(within(rowEl).getByRole("button", { name: "会话操作" }));
  fireEvent.click(await waitFor(() => view.getByRole("menuitem", { name: "重命名" })));
  const input = await waitFor(() => view.getByRole("textbox"));
  fireEvent.change(input, { target: { value: "重试保存的新标题" } });
  fireEvent.keyDown(input, { key: "Enter" });

  await waitFor(() => {
    const status = view.getByTestId("sidebar-session-mutation-status");
    assert.ok(within(status).getByRole("button", { name: "重试" }));
    const restoredInput = view.getByRole("textbox") as HTMLInputElement;
    assert.equal(restoredInput.value, "重试保存的新标题");
  });

  // Click Retry -> succeeds and commits new title cleanly.
  fireEvent.click(within(view.getByTestId("sidebar-session-mutation-status")).getByRole("button", { name: "重试" }));
  await waitFor(() => {
    assert.equal(renameAttempts, 2);
    assert.ok(view.getByText("重试保存的新标题"));
  });
});

test("AgentToolGroupCard morphs step indicator and AgentMarkdownRenderer renders inline stream caret", () => {
  const runningItem = {
    kind: "tool_group" as const,
    seq: 1,
    run_id: "run-morph",
    status: "running" as const,
    rounds: [
      {
        run_id: "run-morph",
        call_id: "call-1",
        tool_id: "search_scenes",
        status: "running" as const,
        summary_zh: "正在检索 Sentinel-1 影像",
      },
    ],
  };

  const view = render(
    <I18nProvider>
      <AgentToolGroupCard item={runningItem} isExpanded />
      <AgentMarkdownRenderer content="正在输出流式总结。" streaming />
    </I18nProvider>
  );

  const stepIcon = view.getByTestId("agent-tool-step-morph");
  assert.equal(stepIcon.dataset.state, "running");
  assert.ok(view.getByTestId("agent-stream-caret"));

  view.rerender(
    <I18nProvider>
      <AgentToolGroupCard
        item={{
          ...runningItem,
          status: "done",
          rounds: [{ ...runningItem.rounds[0], status: "done" }],
        }}
        isExpanded
      />
      <AgentMarkdownRenderer content="输出已完成。" streaming={false} />
    </I18nProvider>
  );

  assert.equal(view.getByTestId("agent-tool-step-morph").dataset.state, "done");
  assert.equal(view.queryByTestId("agent-stream-caret") === null, true);
});

function CancellableProbe({
  onRun,
  onCanceledLog,
}: {
  onRun: (signal: AbortSignal) => Promise<string>;
  onCanceledLog: (msg: string) => void;
}) {
  const { pending, elapsedSeconds, elapsedLabel, run, cancel } = useCancellableAction({
    tickMs: 40,
  });
  const [result, setResult] = React.useState("idle");

  return (
    <div>
      <span data-testid="cancel-pending">{pending ? "true" : "false"}</span>
      <span data-testid="cancel-elapsed">{elapsedSeconds}</span>
      <span data-testid="cancel-label">{elapsedLabel}</span>
      <span data-testid="cancel-result">{result}</span>
      <button
        type="button"
        data-testid="cancel-start-btn"
        onClick={() => {
          void run(onRun).then((res) => {
            if (res === undefined) onCanceledLog("cancelled");
            else setResult(res);
          });
        }}
      >
        开始
      </button>
      <button type="button" data-testid="cancel-abort-btn" onClick={cancel}>
        取消
      </button>
    </div>
  );
}

test("useCancellableAction tracks pending, elapsedSeconds/elapsedLabel, and aborts stale completions cleanly", async () => {
  const canceledLog: string[] = [];
  let abortedFlag = false;
  let resolveSlow!: (v: string) => void;

  const view = render(
    <CancellableProbe
      onCanceledLog={(msg) => canceledLog.push(msg)}
      onRun={(signal) => {
        signal.addEventListener("abort", () => {
          abortedFlag = true;
        });
        return new Promise<string>((resolve) => {
          resolveSlow = resolve;
        });
      }}
    />
  );

  assert.equal(view.getByTestId("cancel-pending").textContent, "false");
  assert.equal(view.getByTestId("cancel-elapsed").textContent, "0");

  fireEvent.click(view.getByTestId("cancel-start-btn"));
  assert.equal(view.getByTestId("cancel-pending").textContent, "true");
  assert.equal(abortedFlag, false);

  // Elapsed label updates with formatted seconds (`已用时 X.Xs`)
  assert.match(view.getByTestId("cancel-label").textContent ?? "", /已用时 \d+(\.\d+)?s/);

  fireEvent.click(view.getByTestId("cancel-abort-btn"));
  assert.equal(abortedFlag, true);
  assert.equal(view.getByTestId("cancel-pending").textContent, "false");

  // Resolve slow action AFTER cancel -> must not leak result into state
  await act(async () => {
    resolveSlow("late-value");
    await sleep(10);
  });
  assert.equal(view.getByTestId("cancel-result").textContent, "idle");
  assert.deepEqual(canceledLog, ["cancelled"]);
});
