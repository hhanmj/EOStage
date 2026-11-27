import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { AgentRunListStrip } from "./AgentRunListStrip";
import { AgentTurnUsageLine } from "./AgentTurnUsageLine";
import { useRunUsageDetail } from "./agentRunUsageHooks";
import { TaskCard, TaskDetailView } from "./DesktopDownloadDrawer";
import type {
  StreamConfirmCardItem,
  StreamProgressCardItem,
} from "../../shell/api/agentStreamLogic";
import type { ApplicationTask } from "../../shell/api/types";
import type { AgentRunDetail, AgentRunSummary } from "../../lib/generated/v3-contracts";

test.afterEach(() => {
  cleanup();
});

async function settle(ms = 30) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

const TASK_ID = "task-unified-w59";

const confirmItem: StreamConfirmCardItem = {
  kind: "confirm_card",
  seq: 50,
  run_id: "run-w59",
  plan_id: "plan-w59",
  plan_hash: "hash-w59",
  snapshot: {
    source: "asf.catalog",
    range: { aoi_name: "地图绘制矩形", area_km2: 125.6, product: "1 个产品", time_range: null },
    estimated_bytes: null,
    output_dir: null,
    output_dir_source: null,
    already_queued_count: 0,
    source_display_name_zh: null,
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
  run_id: "run-w59",
  plan_id: "plan-w59",
  step_index: 0,
  tool_id: "create_task",
  status: "done",
  code: null,
  created_task_ids: [TASK_ID],
};

function task(outputDir: string | undefined, status: ApplicationTask["status"] = "cancelled"): ApplicationTask {
  return {
    task_id: TASK_ID,
    revision: 4,
    category: "scene_download",
    name: "Sentinel-1",
    provider: "asf",
    dataset: "s1_slc",
    status,
    items_count: 1,
    done_bytes: 10,
    total_bytes: 10,
    output_dir: outputDir,
    created_at: "2026-10-27T04:05:31Z",
    updated_at: "2026-10-27T04:07:00Z",
  } as ApplicationTask;
}

function dirRowText(view: ReturnType<typeof render>): string {
  return view.getByText("下载目录").nextElementSibling?.textContent ?? "";
}

test("W5-9 条目 2：已批准卡没有批准记录时用已建任务的保存目录", async (t) => {
  await t.test("任务带目录 → 显示该目录，不写「未知」", () => {
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[task("D:\\ScenePort\\downloads")]}
      />,
    );
    const row = dirRowText(view);
    assert.equal(row.includes("D:\\ScenePort\\downloads"), true, `下载目录行实际：${row}`);
    assert.equal(row.includes("未知"), false, `下载目录行实际：${row}`);
    assert.equal(row.includes("卡上选择"), false, `任务目录不是卡上选择的，实际：${row}`);
  });

  await t.test("任务目录为空字符串 → 「未知」", () => {
    const view = render(
      <AgentConfirmCard
        item={confirmItem}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[task("")]}
      />,
    );
    const row = dirRowText(view);
    assert.equal(row.includes("未知"), true, `下载目录行实际：${row}`);
  });
});

test("W5-9 R3 P2-1：已批准卡目录 = 批准记录 → 本卡任务目录 → 快照", async (t) => {
  const withSnapshotDir: StreamConfirmCardItem = {
    ...confirmItem,
    snapshot: { ...confirmItem.snapshot, output_dir: "D:\\old", output_dir_source: "default" },
  };

  await t.test("任务目录优先于快照（卡上改过目录后重启，快照是旧值）", () => {
    const view = render(
      <AgentConfirmCard
        item={withSnapshotDir}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[task("E:\\real")]}
      />,
    );
    const row = dirRowText(view);
    assert.equal(row.includes("E:\\real"), true, `任务的保存目录才是真实位置，实际：${row}`);
    assert.equal(row.includes("D:\\old"), false, `实际：${row}`);
  });

  await t.test("批准记录仍最优先", () => {
    const view = render(
      <AgentConfirmCard
        item={withSnapshotDir}
        approvedOutputDir={"F:\\chosen"}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[task("E:\\real")]}
      />,
    );
    assert.equal(dirRowText(view).includes("F:\\chosen"), true, dirRowText(view));
  });

  await t.test("R4：任务目录与快照目录相同 → 保留快照的来源徽章", async (tt) => {
    for (const [source, label] of [
      ["default", "默认目录"],
      ["from_message", "消息中指定的目录"],
    ] as const) {
      await tt.test(label, () => {
        const item: StreamConfirmCardItem = {
          ...confirmItem,
          snapshot: { ...confirmItem.snapshot, output_dir: "D:\\ScenePort\\downloads", output_dir_source: source },
        };
        const view = render(
          <AgentConfirmCard
            item={item}
            subsequentProgress={doneProgress}
            createdTaskIds={[TASK_ID]}
            agentTasks={[task("D:\\ScenePort\\downloads")]}
          />,
        );
        const row = dirRowText(view);
        assert.equal(row.includes("D:\\ScenePort\\downloads"), true, row);
        assert.equal(row.includes(label), true, `相同目录必须保留徽章「${label}」，实际：${row}`);
        cleanup();
      });
    }
  });

  await t.test("R4：任务目录与快照不同 → 不沿用快照徽章", () => {
    const view = render(
      <AgentConfirmCard
        item={withSnapshotDir}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[task("E:\\real")]}
      />,
    );
    assert.equal(dirRowText(view).includes("默认目录"), false, dirRowText(view));
  });

  await t.test("没有任务目录时才用快照", () => {
    const view = render(
      <AgentConfirmCard
        item={withSnapshotDir}
        subsequentProgress={doneProgress}
        createdTaskIds={[TASK_ID]}
        agentTasks={[task("")]}
      />,
    );
    assert.equal(dirRowText(view).includes("D:\\old"), true, dirRowText(view));
  });
});

test("W5-9 条目 1：卡上「来源」只读显示名，快照 source 不出现", () => {
  const view = render(<AgentConfirmCard item={confirmItem} sourceLabel="未知" />);
  const source = view.getByText("来源").nextElementSibling?.textContent ?? "";
  assert.equal(source, "未知");
  assert.equal((view.container.textContent ?? "").includes("asf.catalog"), false);
});

test("W5-9 条目 4：未批准卡在卡上选了目录后，「下载目录未知」原因随之消失", () => {
  const view = render(
    <AgentConfirmCard item={confirmItem} sourceLabel="Sentinel-1" defaultDownloadDir={"D:\\ScenePort\\downloads"} />,
  );
  assert.ok(view.getByText("下载目录未知，请在卡上选择"), "选目录前应列出该原因");
  fireEvent.click(view.getByText("使用默认目录"));
  const text = view.container.textContent ?? "";
  assert.equal(text.includes("下载目录未知，请在卡上选择"), false, `选了目录后不得再列该原因，实际：${text}`);
  assert.ok(view.getByText("所选产品不是本次运行检索出的"), "其他原因保留");
  assert.ok(dirRowText(view).includes("D:\\ScenePort\\downloads"));
});

function run(overrides: Partial<AgentRunSummary>): AgentRunSummary {
  return {
    run_id: "run-a",
    session_id: "sess",
    status: "completed",
    started_at: "2026-10-27 15:00:00",
    ended_at: "2026-10-27 15:01:00",
    model_route: null,
    ...overrides,
  };
}

test("W5-9 条目 3：运行记录条显示用量", async (t) => {
  await t.test("有用量 → 「用量 输入 X · 输出 Y」", () => {
    const view = render(
      <AgentRunListStrip
        defaultExpanded
        runs={[run({ usage_total: { prompt: 12345, completion: 678, total: 13023 } })]}
      />,
    );
    assert.ok(view.getByText("用量 输入 12,345 · 输出 678"));
  });

  await t.test("usage_total 为 null → 不显示用量，也不显示 0", () => {
    const view = render(<AgentRunListStrip defaultExpanded runs={[run({ usage_total: null })]} />);
    const text = view.container.textContent ?? "";
    assert.equal(text.includes("用量"), false, `实际：${text}`);
  });

  await t.test("一部分缺失 → 「不完整」，无英文 token", () => {
    const view = render(
      <AgentRunListStrip defaultExpanded runs={[run({ usage_total: { prompt: null, completion: 50, total: 50 } })]} />,
    );
    assert.ok(view.getByText("用量 输入 不完整 · 输出 50"));
    assert.equal(/token/i.test(view.container.textContent ?? ""), false);
  });

  await t.test("只有用量、没有标记 → 没有异常徽章", () => {
    const view = render(
      <AgentRunListStrip
        defaultExpanded
        runs={[run({ reasoning_off_refused: false, truncated: false, usage_total: { prompt: 1, completion: 1, total: 2 } })]}
      />,
    );
    assert.equal(view.queryByText("关闭思考被拒") === null, true);
    assert.equal(view.queryByText("回复被截断") === null, true);
  });

  await t.test("R3 P2-2：徽章排在用量前，用量可截断", () => {
    const view = render(
      <AgentRunListStrip
        defaultExpanded
        runs={[run({ truncated: true, usage_total: { prompt: 123456, completion: 7890, total: 131346 } })]}
      />,
    );
    const text = view.container.textContent ?? "";
    const badgeAt = text.indexOf("回复被截断");
    assert.equal(badgeAt >= 0 && badgeAt < text.indexOf("用量"), true, `实际：${text}`);
    const usage = view.getByText("用量 输入 123,456 · 输出 7,890");
    assert.equal(usage.className.includes("truncate") && usage.className.includes("min-w-0"), true, usage.className);
  });
});

function UsageHarness({
  runs,
  openRun,
}: {
  runs: AgentRunSummary[];
  openRun: (runId: string) => Promise<AgentRunDetail>;
}) {
  const [details, requestDetail] = useRunUsageDetail(openRun);
  return (
    <div>
      {runs.map((r) => (
        <div key={r.run_id} data-testid={r.run_id}>
          <AgentTurnUsageLine
            usageTotal={r.usage_total}
            detail={details[r.run_id]}
            onRequestDetail={() => requestDetail(r.run_id)}
          />
        </div>
      ))}
    </div>
  );
}

function detail(runId: string, usage: Record<string, unknown> | null): AgentRunDetail {
  const reply: Record<string, unknown> = { reasoning_off_refused: false, truncated: false, think_block_unclosed: false };
  if (usage) reply.usage = usage;
  return {
    run: run({ run_id: runId }),
    events: [
      { sequence: 0, previous_sha256: "0", kind: "model_call", payload: { file: "a.json", sha256: "x" }, sha256: "a" },
      { sequence: 1, previous_sha256: "a", kind: "model_reply", payload: reply, sha256: "b" },
    ],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

test("W5-9 R3 P1-2：本轮用量读 usage_total，逐次明细只在悬停/展开时按需读取", async (t) => {
  await t.test("打开会话不读任何运行记录；行文字来自 usage_total", async () => {
    const opened: string[] = [];
    const openRun = async (runId: string) => {
      opened.push(runId);
      return detail(runId, { prompt: 1200, completion: 80, total: 1280 });
    };
    const runs = [
      run({ run_id: "run-a", usage_total: { prompt: 1200, completion: 80, total: 1280 } }),
      run({ run_id: "run-b", usage_total: { prompt: 10, completion: 2, total: 12 } }),
    ];
    const view = render(<UsageHarness runs={runs} openRun={openRun} />);
    await settle();
    assert.equal(view.getByTestId("run-a").textContent, "本轮用量 输入 1,200 · 输出 80");
    assert.deepEqual(opened, [], "打开会话时不得读取运行记录");
  });

  await t.test("悬停读一次该运行并缓存；展开显示逐次明细", async () => {
    const opened: string[] = [];
    const openRun = async (runId: string) => {
      opened.push(runId);
      return detail(runId, { prompt: 1200, completion: 80, total: 1280 });
    };
    const runs = [run({ run_id: "run-a", usage_total: { prompt: 1200, completion: 80, total: 1280 } })];
    const view = render(<UsageHarness runs={runs} openRun={openRun} />);
    const line = view.getByRole("button", { name: /本轮用量/ });
    fireEvent.mouseEnter(line);
    await settle();
    fireEvent.mouseEnter(line);
    await settle();
    assert.deepEqual(opened, ["run-a"], "同一运行只读一次");
    fireEvent.click(line);
    await settle();
    assert.ok(view.getByText("第 1 次回复：输入 1,200 · 输出 80"));
    assert.deepEqual(opened, ["run-a"]);
  });

  await t.test("一次只发一个请求：第二个运行等第一个读完", async () => {
    const pending = new Map<string, (value: AgentRunDetail) => void>();
    const opened: string[] = [];
    const openRun = (runId: string) => {
      opened.push(runId);
      const d = deferred<AgentRunDetail>();
      pending.set(runId, d.resolve);
      return d.promise;
    };
    const runs = [
      run({ run_id: "run-a", usage_total: { prompt: 1, completion: 1, total: 2 } }),
      run({ run_id: "run-b", usage_total: { prompt: 2, completion: 2, total: 4 } }),
    ];
    const view = render(<UsageHarness runs={runs} openRun={openRun} />);
    const [lineA, lineB] = view.getAllByRole("button", { name: /本轮用量/ });
    fireEvent.mouseEnter(lineA);
    fireEvent.mouseEnter(lineB);
    await settle();
    assert.deepEqual(opened, ["run-a"], "第一个请求未完成前不得发第二个");
    await act(async () => {
      pending.get("run-a")?.(detail("run-a", { prompt: 1, completion: 1, total: 2 }));
      await new Promise((resolve) => setTimeout(resolve, 30));
    });
    assert.deepEqual(opened, ["run-a", "run-b"]);
  });

  await t.test("usage_total 为 null → 不显示、不读", async () => {
    const opened: string[] = [];
    const openRun = async (runId: string) => {
      opened.push(runId);
      return detail(runId, null);
    };
    const view = render(<UsageHarness runs={[run({ run_id: "run-n", usage_total: null })]} openRun={openRun} />);
    await settle();
    assert.equal(view.getByTestId("run-n").textContent, "");
    assert.deepEqual(opened, []);
  });

  await t.test("读取失败 → 展开写明细不可用，不重试", async () => {
    let calls = 0;
    const openRun = async (): Promise<AgentRunDetail> => {
      calls++;
      throw new Error("ipc failed");
    };
    const view = render(
      <UsageHarness
        runs={[run({ run_id: "run-f", usage_total: { prompt: 1, completion: 1, total: 2 } })]}
        openRun={openRun}
      />,
    );
    const line = view.getByRole("button", { name: /本轮用量/ });
    fireEvent.click(line);
    await settle();
    assert.ok(view.getByText("逐次明细暂不可用"));
    fireEvent.mouseEnter(line);
    await settle();
    assert.equal(calls, 1);
  });

  await t.test("用量行没有挂在普通 div 上的 aria-label", () => {
    const view = render(
      <UsageHarness
        runs={[run({ run_id: "run-x", usage_total: { prompt: 1, completion: 1, total: 2 } })]}
        openRun={async (id) => detail(id, null)}
      />,
    );
    assert.equal(view.container.querySelector("div[aria-label]") === null, true);
  });
});

test("W5-9 条目 2：下载抽屉目录行与「打开目录」只在目录非空时出现", async (t) => {
  await t.test("已完成任务带目录 → 有「打开目录」", () => {
    const view = render(<TaskCard task={task("D:\\ScenePort\\downloads", "completed")} onOpenFolder={() => {}} />);
    assert.equal(view.queryByLabelText("打开所在目录") !== null, true);
  });

  await t.test("目录为空白 → 没有「打开目录」，详情没有「保存目录」", () => {
    const card = render(<TaskCard task={task("   ", "completed")} onOpenFolder={() => {}} />);
    assert.equal(card.queryByLabelText("打开所在目录") === null, true, "空白目录不得提供「打开目录」");
    cleanup();
    const detailView = render(<TaskDetailView task={task("   ", "completed")} onOpenFolder={() => {}} />);
    assert.equal(detailView.queryByText("保存目录") === null, true, "空白目录不得显示「保存目录」行");
  });

  await t.test("详情显示保存目录", () => {
    const view = render(<TaskDetailView task={task("D:\\ScenePort\\downloads", "completed")} onOpenFolder={() => {}} />);
    assert.ok(view.getByText("保存目录"));
    assert.ok(view.getByText("D:\\ScenePort\\downloads"));
  });
});

test("W5-9 R3 P1-1：「打开目录」把任务与修订号交给上层，失败显示中文原因", async (t) => {
  await t.test("任务卡：回调收到 {task_id, revision}", async () => {
    const calls: unknown[] = [];
    const view = render(
      <TaskCard
        task={task("D:\\ScenePort\\downloads", "completed")}
        onOpenFolder={(ref: any) => {
          calls.push(ref);
        }}
      />,
    );
    fireEvent.click(view.getByLabelText("打开所在目录"));
    await settle();
    assert.deepEqual(calls, [{ task_id: TASK_ID, revision: 4 }]);
  });

  await t.test("任务卡：失败显示「未能打开保存目录」，不露英文", async () => {
    const view = render(
      <TaskCard
        task={task("D:\\ScenePort\\downloads", "completed")}
        onOpenFolder={async () => {
          throw { ok: false, error: "Tauri application service operation failed", code: null };
        }}
      />,
    );
    fireEvent.click(view.getByLabelText("打开所在目录"));
    await settle();
    assert.ok(view.getByText("未能打开保存目录"));
    assert.equal((view.container.textContent ?? "").includes("Tauri"), false);
  });

  await t.test("详情：回调收到 {task_id, revision}，失败同样显示", async () => {
    const calls: unknown[] = [];
    const view = render(
      <TaskDetailView
        task={task("D:\\ScenePort\\downloads", "completed")}
        onOpenFolder={async (ref: any) => {
          calls.push(ref);
          throw { ok: false, error: "x", code: null };
        }}
      />,
    );
    fireEvent.click(view.getByText("打开"));
    await settle();
    assert.deepEqual(calls, [{ task_id: TASK_ID, revision: 4 }]);
    assert.ok(view.getByText("未能打开保存目录"));
  });
});
