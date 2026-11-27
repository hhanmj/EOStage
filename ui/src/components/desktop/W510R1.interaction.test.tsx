import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentConclusionCard } from "./AgentConclusionCard";
import { DesktopDownloadDrawer, TaskCard, TaskDetailView } from "./DesktopDownloadDrawer";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "../../lib/desktopShellBridge";
import { MAX_UI_TASK_PAGE_SIZE } from "../../lib/coreRestoreProjection";
import { classifyStreamEvents, type StreamConclusionItem } from "../../shell/api/agentStreamLogic";
import type { ApplicationTask, TaskRetryFallbackOutcome, UnifiedTaskItemAction } from "../../shell/api/types";
import { getDb } from "../../shell/api/fixture";
import { resetTaskFallbackRequestGatesForTests } from "../../shell/api/taskFallbackRequestGate";

test.afterEach(() => {
  cleanup();
  resetTaskFallbackRequestGatesForTests();
});

async function settle(ms = 30) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

type Claim = StreamConclusionItem["claims"][number];
type Ref = Claim["refs"][number];

const thisRunBatch: Ref = { kind: "batch", id: "batch-1", label: "批次 batch-1", trace_sequence: 22, response_count: 7 };
const taskRef: Ref = { kind: "task", id: "task-1", label: "任务", trace_sequence: null, response_count: null };

function conclusion(claims: Claim[], text = "结论正文"): StreamConclusionItem {
  return { kind: "conclusion", seq: 30, run_id: "run-c", text, claims, reasoning_off_refused: false };
}

/** 摘要徽章的可见悬停说明：aria-describedby 指向 role=tooltip 的元素，不靠 title */
/**
 * 徽章自己可聚焦时说明挂在徽章上；徽章在「依据」按钮里时说明挂在按钮上（R3：键盘聚焦按钮也要出气泡）。
 */
function tooltipOf(view: ReturnType<typeof render>, label: string): string | null {
  const badge = view.getAllByText(label)[0];
  assert.equal(badge.getAttribute("title"), null, "不得只靠 title");
  const button = badge.closest("button");
  if (button) {
    assert.equal(badge.getAttribute("aria-describedby"), null, "按钮里的徽章不重复挂说明");
  }
  const id = (button ?? badge).getAttribute("aria-describedby");
  if (!id) return null;
  const tip = view.container.ownerDocument.getElementById(id);
  assert.ok(tip, "aria-describedby 必须指向存在的提示元素");
  assert.equal(tip!.getAttribute("role"), "tooltip");
  return tip!.textContent;
}

test("复核二：失败或无效核验的单引用显示未知，展开态可访问", () => {
  for (const verified of [false, undefined, "false"]) {
    const item = { ...conclusion([]), claims: [{ statement: "x", verified, refs: [taskRef] }] } as unknown as StreamConclusionItem;
    const view = render(<AgentConclusionCard item={item} />);
    const button = view.getByText("依据").closest("button")!;
    assert.equal(button.getAttribute("aria-expanded"), "false");
    fireEvent.click(button);
    assert.equal(button.getAttribute("aria-expanded"), "true");
    assert.ok(view.getByText("逐条核验结果未知"));
    fireEvent.click(button);
    assert.equal(button.getAttribute("aria-expanded"), "false");
    cleanup();
  }
});

test("R4 复核：混合多引用不伪称逐条核验结果", () => {
  const view = render(<AgentConclusionCard item={conclusion([
    { statement: "通过", verified: true, refs: [thisRunBatch] },
    { statement: "未通过", verified: false, refs: [taskRef, { ...thisRunBatch, id: "bad-batch" }] },
  ])} />);
  assert.equal(tooltipOf(view, "部分核验"), "2 条有引用的陈述中 1 条已核验");
  fireEvent.click(view.getByText("依据"));
  assert.equal(view.getAllByText("逐条核验结果未知").length, 2);
  const badge = view.getAllByText("逐条核验结果未知")[0];
  const tip = view.container.ownerDocument.getElementById(badge.getAttribute("aria-describedby")!);
  assert.equal(tip?.textContent, "该陈述未通过核验，无法确定具体引用");
  assert.ok(tip?.className.includes("right-0"));
  assert.ok(tip?.className.includes("max-w-"));
  assert.ok(tip?.className.includes("whitespace-normal"));
});

test("R4 复核：结论正文保护代码区域，用户消息不替换", () => {
  const events = classifyStreamEvents([
    { seq: 1, kind: "message", payload: { run_id: "run-c", role: "user", text: "用户 [local] [redacted]" } },
    { seq: 2, kind: "conclusion", payload: { run_id: "run-c", text: "前 [local] `inline [redacted]`\n```json\n{\"x\":\"[redacted]\"}\n```\n后 [local]", claims: [], reasoning_off_refused: false } },
  ]);
  assert.equal(events[0].kind, "message");
  assert.equal((events[0] as any).text, "用户 [local] [redacted]");
  assert.equal((events[1] as any).text, "前 （本机内容） `inline [redacted]`\n```json\n{\"x\":\"[redacted]\"}\n```\n后 （本机内容）");
  const view = render(<AgentConclusionCard item={events[1] as StreamConclusionItem} />);
  assert.ok(view.getByText(/`inline \[redacted\]`/));
  fireEvent.click(view.getByRole("button", { name: "展开详情" }));
  assert.equal(view.container.querySelector("pre")?.textContent, '```json\n{"x":"[redacted]"}\n```');
});

test("R4 复核：详情任务从最新快照消失时撤回打开目录", async () => {
  const initial = task("completed");
  const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} onOpenFolder={() => {}} />);
  fireEvent.click(view.getByLabelText("查看详情"));
  assert.ok(view.getByText("打开"));
  view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[]} onOpenFolder={() => {}} />);
  await settle();
  assert.equal(view.queryByText("打开"), null);
  assert.ok(view.getByText("任务已不在最新记录中"));
});

test("W5-10 条目 1：依据摘要与逐条引用一致，原因可见", async (t) => {
  await t.test("R4：L4 r10 C2 形状 → 「部分核验」，展开后列出无引用的陈述", () => {
    const view = render(
      <AgentConclusionCard
        item={conclusion([
          { statement: "7 景", verified: true, refs: [thisRunBatch, { ...thisRunBatch }] },
          { statement: "建议", verified: false, refs: [] },
        ])}
      />,
    );
    assert.equal(tooltipOf(view, "部分核验"), "1 条有引用的陈述中 1 条已核验；另有 1 条陈述没有引用");
    fireEvent.click(view.getByText("依据"));
    assert.ok(view.getByText("1 条陈述没有引用 · 未核验"));
  });

  await t.test("R4：所有陈述都已核验 → 「已核验」，徽章字重用基线允许的 600", () => {
    const view = render(
      <AgentConclusionCard
        item={conclusion([
          { statement: "7 景", verified: true, refs: [thisRunBatch] },
          { statement: "最新 10-18", verified: true, refs: [{ ...thisRunBatch }] },
        ])}
      />,
    );
    const badge = view.getByText("已核验");
    const holder = badge.closest("span.inline-flex") ?? badge;
    assert.equal(holder.className.includes("font-medium"), false, holder.className);
    assert.equal(holder.className.includes("font-semibold"), true, holder.className);
    assert.equal(view.queryByText("未核验") === null, true);
  });

  await t.test("部分核验 → 「部分核验」+ 可见提示写明条数", () => {
    const view = render(
      <AgentConclusionCard
        item={conclusion([
          { statement: "7 景", verified: true, refs: [thisRunBatch] },
          { statement: "任务已建", verified: false, refs: [taskRef] },
        ])}
      />,
    );
    assert.equal(tooltipOf(view, "部分核验"), "2 条有引用的陈述中 1 条已核验");
  });

  await t.test("引用未核验 → 显示陈述级结果", () => {
    const view = render(
      <AgentConclusionCard item={conclusion([{ statement: "任务已建", verified: false, refs: [taskRef] }])} />,
    );
    assert.equal(tooltipOf(view, "未核验"), "1 条有引用的陈述中 0 条已核验");
  });

  await t.test("R3：说明挂在「依据」按钮上，按钮键盘聚焦时气泡可见", () => {
    const view = render(
      <AgentConclusionCard item={conclusion([{ statement: "任务已建", verified: false, refs: [taskRef] }])} />,
    );
    const button = view.getAllByText("未核验")[0].closest("button");
    assert.ok(button, "摘要徽章在「依据」按钮里");
    const id = button!.getAttribute("aria-describedby");
    assert.ok(id, "按钮必须挂 aria-describedby");
    const tip = view.container.ownerDocument.getElementById(id!);
    assert.equal(tip?.textContent, "1 条有引用的陈述中 0 条已核验");
    assert.equal(button!.className.includes("group/evidence"), true, button!.className);
    assert.equal(
      (tip?.className ?? "").includes("group-focus-visible/evidence:opacity-100"),
      true,
      `按钮聚焦时必须显示气泡，实际：${tip?.className}`,
    );
  });

  await t.test("没有任何引用 → 「未核验」+ 可见提示", () => {
    const view = render(<AgentConclusionCard item={conclusion([{ statement: "建议", verified: false, refs: [] }])} />);
    assert.equal(tooltipOf(view, "未核验"), "1 条陈述没有引用");
  });

  await t.test("展开后逐条「未核验」也有可见提示", () => {
    const view = render(
      <AgentConclusionCard
        item={conclusion([
          { statement: "7 景", verified: true, refs: [thisRunBatch] },
          { statement: "任务已建", verified: false, refs: [taskRef] },
        ])}
      />,
    );
    fireEvent.click(view.getByText("依据"));
    assert.equal(tooltipOf(view, "逐条核验结果未知"), "该陈述未通过核验，无法确定具体引用");
    const badge = view.getAllByText("逐条核验结果未知")[0];
    const tip = view.container.ownerDocument.getElementById(badge.getAttribute("aria-describedby")!);
    assert.equal((tip?.className ?? "").split(/\s+/).includes("right-0"), true, `引用行在右侧，气泡须右对齐，实际：${tip?.className}`);
    assert.equal((tip?.className ?? "").split(/\s+/).includes("left-0"), false, tip?.className);
    assert.equal(badge.className.includes("font-medium"), false, badge.className);
  });
});

test("W5-10 条目 3：结论正文的本机标记经事件分类换成中文占位，空格不增删", () => {
  const [item] = classifyStreamEvents([
    {
      seq: 30,
      kind: "conclusion",
      payload: {
        run_id: "run-c",
        text: "凭据为 [local-reference]，日志在 [local-reference:message_dir]，其余[已省略]。",
        claims: [],
        reasoning_off_refused: false,
      },
    },
  ]);
  const view = render(<AgentConclusionCard item={item as StreamConclusionItem} />);
  const text = view.container.textContent ?? "";
  assert.equal(text.includes("[local"), false, text);
  assert.equal(text.includes("[已省略]"), false, text);
  assert.ok(text.includes("凭据为 （本机内容），日志在 （消息中指定的目录），其余（已省略）。"), text);
});

function task(status: ApplicationTask["status"], outputDir: string | undefined = "D:\\ScenePort\\downloads"): ApplicationTask {
  return {
    task_id: "task-q-1",
    revision: 7,
    category: "scene_download",
    name: "Sentinel-1",
    provider: "asf",
    dataset: "s1_slc",
    status,
    items_count: 1,
    done_bytes: 10,
    total_bytes: 100,
    output_dir: outputDir,
    created_at: "2026-10-27T04:05:31Z",
    updated_at: "2026-10-27T04:07:00Z",
  } as ApplicationTask;
}

test("W5-10 R4：深链打开后刷新任务列表不把有效完成响应误判为不确定", async () => {
  const db = getDb();
  const original = db.retryTaskFallback;
  const originalUrl = window.location.href;
  let resolvePending!: (outcome: TaskRetryFallbackOutcome) => void;
  let calls = 0;
  db.retryTaskFallback = (() => {
    calls++;
    return new Promise<TaskRetryFallbackOutcome>((resolve) => {
      resolvePending = resolve;
    });
  }) as typeof db.retryTaskFallback;

  try {
    window.history.replaceState(null, "", "#retry-fallback");
    const initial = task("failed");
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    assert.equal(calls, 1);

    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[{ ...initial }]} />);
    await settle();
    await act(async () => {
      resolvePending({
        task: { task_id: initial.task_id, revision: 8, status: "queued", created: false },
        refreshed_batch_id: "batch-deep-link",
        rebuilt_item_ids: [],
        missing_product_ids: [],
        added_product_ids: [],
        needs_user_confirmation: false,
      });
      await Promise.resolve();
    });

    assert.equal(view.queryByRole("alertdialog") === null, true);
    assert.equal(view.queryByText(/无法安全确认/) === null, true);
    assert.equal(calls, 1);
  } finally {
    db.retryTaskFallback = original;
    window.history.replaceState(null, "", originalUrl);
  }
});

test("W5-10 R4：深链等待任务加载，关闭后列表刷新不重新打开", async () => {
  const originalUrl = window.location.href;
  try {
    window.history.replaceState(null, "", "#retry-fallback");
    const initial = task("failed");
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[]} />);
    assert.equal(view.queryByRole("alertdialog") === null, true);

    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
    assert.ok(view.getByRole("alertdialog"));
    fireEvent.click(view.getByRole("button", { name: "取消" }));
    assert.equal(view.queryByRole("alertdialog") === null, true);

    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[{ ...initial }]} />);
    assert.equal(view.queryByRole("alertdialog") === null, true);
  } finally {
    window.history.replaceState(null, "", originalUrl);
  }
});

test("W5-10 R4：第一步关闭重开保留 pending gate，旧结果不重复发送", async () => {
  const db = getDb();
  const original = db.retryTaskFallback;
  const pending: Array<(outcome: TaskRetryFallbackOutcome) => void> = [];
  let calls = 0;
  db.retryTaskFallback = ((id: string, batch: string | null = null) => {
    calls++;
    assert.equal(batch, null);
    return new Promise<TaskRetryFallbackOutcome>((resolve) => pending.push(resolve));
  }) as typeof db.retryTaskFallback;
  try {
    const initial = task("failed");
    const view = render(<DesktopDownloadDrawer open variant="drawer" tasks={[initial]} onClose={() => {}} />);
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    assert.equal(calls, 1);
    view.rerender(<DesktopDownloadDrawer open={false} variant="drawer" tasks={[initial]} onClose={() => {}} />);
    view.rerender(<DesktopDownloadDrawer open variant="drawer" tasks={[initial]} onClose={() => {}} />);
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    assert.equal((view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement).disabled, true);
    assert.ok(view.getByText(/请求已发出/));
    await act(async () => {
      pending[0]({ task: { task_id: initial.task_id, revision: initial.revision, status: "failed", created: false }, refreshed_batch_id: "batch-reopen", rebuilt_item_ids: [], missing_product_ids: ["missing"], added_product_ids: [], needs_user_confirmation: true });
      await Promise.resolve();
    });
    assert.equal(calls, 1);
    assert.ok(view.getByText("确认检索产品差异"));
  } finally {
    db.retryTaskFallback = original;
  }
});

test("W5-10 R4：第二步关闭重开保留 pending gate，任务切换互相隔离", async () => {
  const db = getDb();
  const original = db.retryTaskFallback;
  const pending: Array<(outcome: TaskRetryFallbackOutcome) => void> = [];
  const calls: Array<[string, string | null]> = [];
  db.retryTaskFallback = ((id: string, batch: string | null = null) => {
    calls.push([id, batch]);
    if (batch === null) {
      return Promise.resolve({ task: { task_id: id, revision: 7, status: "failed", created: false }, refreshed_batch_id: "batch-second", rebuilt_item_ids: [], missing_product_ids: ["missing"], added_product_ids: [], needs_user_confirmation: true });
    }
    return new Promise<TaskRetryFallbackOutcome>((resolve) => pending.push(resolve));
  }) as typeof db.retryTaskFallback;
  try {
    const first = task("failed");
    const second = { ...task("failed"), task_id: "task-q-2" };
    const view = render(<DesktopDownloadDrawer open variant="drawer" tasks={[first, second]} onClose={() => {}} />);
    fireEvent.click(view.getAllByLabelText("重新检索后重试")[0]);
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    await settle();
    fireEvent.click(view.getByRole("button", { name: "确认重建" }));
    assert.deepEqual(calls, [[first.task_id, null], [first.task_id, "batch-second"]]);
    view.rerender(<DesktopDownloadDrawer open={false} variant="drawer" tasks={[first, second]} onClose={() => {}} />);
    view.rerender(<DesktopDownloadDrawer open variant="drawer" tasks={[first, second]} onClose={() => {}} />);
    fireEvent.click(view.getAllByLabelText("重新检索后重试")[1]);
    assert.equal((view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement).disabled, false);
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    assert.deepEqual(calls, [[first.task_id, null], [first.task_id, "batch-second"], [second.task_id, null]]);
    await act(async () => {
      pending[0]({ task: { task_id: first.task_id, revision: 8, status: "queued", created: false }, refreshed_batch_id: "batch-second", rebuilt_item_ids: [], missing_product_ids: [], added_product_ids: [], needs_user_confirmation: false });
      await Promise.resolve();
    });
    assert.equal(calls.length, 3);
    assert.ok(view.getByRole("alertdialog"));
  } finally {
    db.retryTaskFallback = original;
  }
});

test("复核三：兜底异步返回与当前对话框及快照绑定", async (t) => {
  const db = getDb();
  const original = db.retryTaskFallback;
  const offer = (id: string): TaskRetryFallbackOutcome => ({ task: { task_id: id, revision: 7, status: "failed", created: false }, refreshed_batch_id: "batch-test", rebuilt_item_ids: [], missing_product_ids: ["missing"], added_product_ids: [], needs_user_confirmation: true });
  const completed = (id: string): TaskRetryFallbackOutcome => ({ ...offer(id), task: { task_id: id, revision: 8, status: "queued", created: false }, needs_user_confirmation: false });
  let resolvePending!: (outcome: TaskRetryFallbackOutcome) => void;
  try {
    for (const step of [1, 2] as const) {
      for (const change of ["missing", "status", "revision", "wrong-id", "wrong-batch", "reopen"] as const) {
        await t.test(`步骤 ${step} ${change}`, async () => {
          resetTaskFallbackRequestGatesForTests();
          let calls = 0;
          db.retryTaskFallback = ((id: string) => {
            calls++;
            if (step === 2 && calls === 1) return offer(id);
            return new Promise<TaskRetryFallbackOutcome>((resolve) => { resolvePending = resolve; });
          }) as typeof db.retryTaskFallback;
          const initial = task("failed");
          const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
          fireEvent.click(view.getByLabelText("重新检索后重试"));
          fireEvent.click(view.getByRole("button", { name: "重新检索" }));
          if (step === 2) {
            await settle();
            fireEvent.click(view.getByRole("button", { name: "确认重建" }));
          }
          const changed = change === "missing" ? [] : [{ ...initial, ...(change === "status" ? { status: "running" as const } : change === "revision" ? { revision: 8 } : {}) }];
          if (["missing", "status", "revision"].includes(change)) view.rerender(<DesktopDownloadDrawer open variant="page" tasks={changed} />);
          if (change === "reopen") {
            fireEvent.click(view.getByRole("alertdialog"));
            fireEvent.click(view.getByLabelText("重新检索后重试"));
          }
          const outcome = completed(change === "wrong-id" ? "other-task" : initial.task_id);
          if (change === "wrong-batch") outcome.refreshed_batch_id = step === 2 ? "other-batch" : "";
          await act(async () => { resolvePending(outcome); await Promise.resolve(); });
          assert.ok(view.getByRole("alertdialog"), "过期返回不得关闭当前确认框");
          const diffDialog = view.queryByRole("alertdialog", { name: "确认检索产品差异" });
          // 本用例的第一步 reopen 返回的是 queued 完成结果，不是需确认的差异 offer。
          const shouldShowDiff = step === 2 && ["wrong-id", "wrong-batch"].includes(change);
          assert.equal(diffDialog !== null, shouldShowDiff, shouldShowDiff
            ? "同快照的第二步异常必须保留差异对话框"
            : "快照变化、过期结果或不确定重开不得继续展示可确认差异");
          assert.ok(view.getByText(/请求已发出.*无法安全确认/), "不能声称已取消服务端操作");
          const actionButton = view.getByRole("button", {
            name: step === 2 && diffDialog ? "确认重建" : "重新检索",
          }) as HTMLButtonElement;
          assert.equal(actionButton.disabled, true, "未核实结果前不得重复发送");
          assert.equal(calls, step, "过期返回不能触发额外请求");
          cleanup();
        });
      }
    }
  } finally {
    db.retryTaskFallback = original;
  }
});

test("复核二：两步兜底在任务消失、状态或修订变化后不得发请求", async (t) => {
  const db = getDb();
  const original = db.retryTaskFallback;
  const calls: Array<[string, string | null]> = [];
  db.retryTaskFallback = (id: string, batch: string | null = null): TaskRetryFallbackOutcome => {
    calls.push([id, batch]);
    return { task: { task_id: id, revision: 7, status: "failed", created: false }, refreshed_batch_id: "batch-test", rebuilt_item_ids: [], missing_product_ids: ["missing"], added_product_ids: [], needs_user_confirmation: true };
  };
  try {
    for (const step of [1, 2]) {
      for (const change of ["missing", "status", "revision"] as const) {
        await t.test(`第 ${step} 步 ${change}`, async () => {
          resetTaskFallbackRequestGatesForTests();
          calls.length = 0;
          const initial = task("failed");
          const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
          fireEvent.click(view.getByLabelText("重新检索后重试"));
          if (step === 2) {
            fireEvent.click(view.getByRole("button", { name: "重新检索" }));
            await settle();
            assert.equal(calls.length, 1);
            assert.ok(view.getByRole("alertdialog", { name: "确认检索产品差异" }));
          }
          const changed = change === "missing" ? [] : [{ ...initial, ...(change === "status" ? { status: "running" as const } : { revision: 8 }) }];
          view.rerender(<DesktopDownloadDrawer open variant="page" tasks={changed} />);
          // 快照不再匹配时旧差异不得继续可确认；保留对无请求的原有断言。
          const confirmButton = view.queryByRole("button", { name: "确认重建" }) as HTMLButtonElement | null;
          assert.equal(confirmButton === null || confirmButton.disabled, true);
          const retryButton = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
          assert.equal(retryButton.disabled, true);
          fireEvent.click(retryButton);
          await settle();
          assert.equal(calls.length, step === 1 ? 0 : 1);
          cleanup();
        });
      }
    }
  } finally {
    db.retryTaskFallback = original;
  }
});

test("W5-10 R4：跨修订重开如实显示不确定且不得重发", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  let calls = 0;
  db.retryTaskFallback = (() => {
    calls++;
    return new Promise<TaskRetryFallbackOutcome>(() => {});
  }) as typeof db.retryTaskFallback;

  try {
    const initial = task("failed");
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    assert.equal(calls, 1);

    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[{ ...initial, revision: 8 }]} />);
    await settle();

    assert.ok(view.getByRole("alertdialog"));
    assert.ok(view.getByText(/任务状态发生变化/));
    const button = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
    assert.equal(button.disabled, true);

    fireEvent.click(button);
    await settle();
    assert.equal(calls, 1);
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 R4：不确定状态跨重开与跨修订不可重发", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  let calls = 0;
  db.retryTaskFallback = (() => {
    calls++;
    return Promise.resolve({
      task: { task_id: "other-task", revision: 8, status: "queued", created: false },
      refreshed_batch_id: "batch-test",
      rebuilt_item_ids: [],
      missing_product_ids: ["missing"],
      added_product_ids: [],
      needs_user_confirmation: false,
    });
  }) as typeof db.retryTaskFallback;

  try {
    const initial = task("failed");
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    await settle();
    assert.equal(calls, 1);
    assert.ok(view.getByText(/无法安全确认/));

    // 关闭抽屉后重开，gate 在关闭/重开后依然保留
    view.rerender(<DesktopDownloadDrawer open={false} variant="page" tasks={[initial]} />);
    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    const buttonAfterDrawerReopen = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
    assert.equal(buttonAfterDrawerReopen.disabled, true);
    fireEvent.click(buttonAfterDrawerReopen);
    await settle();
    assert.equal(calls, 1);

    // 跨修订后重开，不确定状态依然如实呈现且不可重发
    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[{ ...initial, revision: 8 }]} />);
    await settle();
    fireEvent.click(view.getByRole("button", { name: "取消" }));
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    const buttonAfterRevChange = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
    assert.equal(buttonAfterRevChange.disabled, true);
    assert.ok(view.getByText(/无法安全确认/));
    fireEvent.click(buttonAfterRevChange);
    await settle();
    assert.equal(calls, 1);
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 R4：任务 A 的迟到结果不污染任务 B 的对话框", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  const pendingResolvers: Array<(outcome: TaskRetryFallbackOutcome) => void> = [];
  let calls = 0;
  db.retryTaskFallback = (() => {
    calls++;
    return new Promise<TaskRetryFallbackOutcome>((resolve) => {
      pendingResolvers.push(resolve);
    });
  }) as typeof db.retryTaskFallback;

  try {
    for (const result of ["uncertain", "settled", "completed"] as const) {
      resetTaskFallbackRequestGatesForTests();
      pendingResolvers.length = 0;
      calls = 0;
      const taskA = task("failed");
      const taskB = { ...task("failed"), task_id: "task-q-2" };
      const view = render(<DesktopDownloadDrawer open variant="drawer" tasks={[taskA, taskB]} onClose={() => {}} />);

      // 打开任务 A 对话框并点击重新检索
      fireEvent.click(view.getAllByLabelText("重新检索后重试")[0]);
      fireEvent.click(view.getByRole("button", { name: "重新检索" }));
      assert.equal(calls, 1);
      assert.ok(view.getByText("请求已发出，关闭对话框不会取消服务端操作；请稍后核对任务状态。"));

      // 关闭 pending 展示不取消请求，换到另一任务。
      fireEvent.click(view.getByRole("alertdialog"));
      fireEvent.click(view.getAllByLabelText("重新检索后重试")[1]);
      fireEvent.click(view.getByRole("button", { name: "重新检索" }));
      assert.equal(calls, 2);
      assert.ok(view.getByText("请求已发出，关闭对话框不会取消服务端操作；请稍后核对任务状态。"));

      // A 的不匹配结果、差异结果及完成结果均不得污染或关闭 B。
      await act(async () => {
        pendingResolvers[0]({
          task: {
            task_id: result === "uncertain" ? "other-task" : taskA.task_id,
            revision: result === "completed" ? 8 : 7,
            status: result === "completed" ? "queued" : "failed",
            created: false,
          },
          refreshed_batch_id: "batch-test",
          rebuilt_item_ids: [],
          missing_product_ids: ["missing"],
          added_product_ids: [],
          needs_user_confirmation: result !== "completed",
        });
        await Promise.resolve();
      });

      assert.ok(view.getByRole("alertdialog"));
      assert.ok(view.getByText("请求已发出，关闭对话框不会取消服务端操作；请稍后核对任务状态。"));
      const buttonB = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
      assert.equal(buttonB.disabled, true);
      assert.equal(view.queryByText(/无法安全确认/) === null, true);
      assert.equal(view.queryByText("确认检索产品差异") === null, true);
      assert.equal(calls, 2);

      if (result === "uncertain") {
        // 关闭 B 的 pending 展示，重开 A，如实显示 A 的不确定状态。
        fireEvent.click(view.getByRole("alertdialog"));
        fireEvent.click(view.getAllByLabelText("重新检索后重试")[0]);
        assert.ok(view.getByRole("alertdialog"));
        assert.ok(view.getByText(/无法安全确认/));
        const buttonA = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
        assert.equal(buttonA.disabled, true);
        assert.equal(calls, 2);
      }
      cleanup();
    }
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 R4：任务从快照消失时 pending 立即转不确定，晚到结果不得结清", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  let resolvePending!: (outcome: TaskRetryFallbackOutcome) => void;
  let calls = 0;
  db.retryTaskFallback = (() => {
    calls++;
    return new Promise<TaskRetryFallbackOutcome>((resolve) => {
      resolvePending = resolve;
    });
  }) as typeof db.retryTaskFallback;

  try {
    const initial = task("failed");
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[initial]} />);
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    assert.equal(calls, 1);

    // 在响应返回前任务从快照消失
    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[]} />);
    await settle();

    // 快照对齐副作用立即将 pending 转为不确定
    assert.ok(view.getByRole("alertdialog"));
    assert.ok(view.getByText(/任务状态发生变化/));
    const button = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
    assert.equal(button.disabled, true);
    assert.equal(calls, 1);

    // 随后返回看似合法的同任务结果，也不得结清或展示差异框
    await act(async () => {
      resolvePending({
        task: { task_id: initial.task_id, revision: 7, status: "failed", created: false },
        refreshed_batch_id: "batch-gone",
        rebuilt_item_ids: [],
        missing_product_ids: ["missing"],
        added_product_ids: [],
        needs_user_confirmation: true,
      });
      await Promise.resolve();
    });

    assert.ok(view.getByRole("alertdialog"));
    assert.equal(view.queryByText("确认检索产品差异"), null);
    assert.ok(view.getByText(/任务状态发生变化/));
    assert.equal((view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement).disabled, true);
    assert.equal(calls, 1);
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 R4：差异确认框打开时任务跨修订，旧差异不可见且不得向后端提交旧 batch", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  const calls: Array<[string, string | null]> = [];
  db.retryTaskFallback = ((id: string, batch: string | null = null) => {
    calls.push([id, batch]);
    if (batch === null) {
      return Promise.resolve({
        task: { task_id: id, revision: 7, status: "failed", created: false },
        refreshed_batch_id: "batch-old-7",
        rebuilt_item_ids: [],
        missing_product_ids: ["missing-p-old"],
        added_product_ids: ["added-p-old"],
        needs_user_confirmation: true,
      });
    }
    return Promise.resolve({
      task: { task_id: id, revision: 8, status: "queued", created: false },
      refreshed_batch_id: batch,
      rebuilt_item_ids: [],
      missing_product_ids: [],
      added_product_ids: [],
      needs_user_confirmation: false,
    });
  }) as typeof db.retryTaskFallback;

  try {
    const taskA = { ...task("failed"), task_id: "task-a", revision: 7 };
    const taskB = { ...task("failed"), task_id: "task-b", revision: 7 };
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[taskA, taskB]} />);

    // task-a 打开第一步对话框并请求
    fireEvent.click(view.getAllByLabelText("重新检索后重试")[0]);
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    await settle();

    assert.deepEqual(calls, [["task-a", null]]);
    assert.equal(Boolean(view.queryByRole("alertdialog", { name: "确认检索产品差异" })), true);
    assert.equal(Boolean(view.queryByText(/missing-p-old/)), true);

    // 差异对话框打开时，任务列表刷新为 rev8（仍为 failed）
    const taskA_rev8 = { ...taskA, revision: 8 };
    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[taskA_rev8, taskB]} />);
    await settle();

    // 跨修订后旧差异不得继续作为有效差异呈现，确认按钮若存在必须不可用
    const confirmButton = view.queryByRole("button", { name: "确认重建" }) as HTMLButtonElement | null;
    if (confirmButton && !confirmButton.disabled) {
      fireEvent.click(confirmButton);
      await settle();
    }

    // 核心断言：后端绝不能收到带 batch-old-7 的第二步调用，且旧 diff 不可见
    assert.equal(
      calls.some(([id, b]) => id === "task-a" && b === "batch-old-7"),
      false,
      "任务修订号已变更，不得向后端提交旧的差异批次",
    );
    assert.equal(view.queryByText(/missing-p-old/) === null, true, "跨修订后旧差异产品不应继续显示");
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 R4：跨修订关闭重开不得展示旧差异，允许为新修订重新检索且不影响其它任务", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  const calls: Array<[string, string | null]> = [];
  db.retryTaskFallback = ((id: string, batch: string | null = null) => {
    calls.push([id, batch]);
    if (id === "task-a") {
      const isFirstCall = calls.filter(([callId]) => callId === "task-a").length === 1;
      if (batch === null) {
        return Promise.resolve({
          task: { task_id: id, revision: isFirstCall ? 7 : 8, status: "failed", created: false },
          refreshed_batch_id: isFirstCall ? "batch-old-7" : "batch-new-8",
          rebuilt_item_ids: [],
          missing_product_ids: [isFirstCall ? "missing-p-old" : "missing-p-new"],
          added_product_ids: [isFirstCall ? "added-p-old" : "added-p-new"],
          needs_user_confirmation: true,
        });
      }
      return Promise.resolve({
        task: { task_id: id, revision: 9, status: "queued", created: false },
        refreshed_batch_id: batch,
        rebuilt_item_ids: [],
        missing_product_ids: [],
        added_product_ids: [],
        needs_user_confirmation: false,
      });
    }
    // task-b 正常返回
    return Promise.resolve({
      task: { task_id: id, revision: 7, status: "failed", created: false },
      refreshed_batch_id: "batch-b",
      rebuilt_item_ids: [],
      missing_product_ids: [],
      added_product_ids: [],
      needs_user_confirmation: false,
    });
  }) as typeof db.retryTaskFallback;

  try {
    const taskA_rev7 = { ...task("failed"), task_id: "task-a", revision: 7 };
    const taskB = { ...task("failed"), task_id: "task-b", revision: 7 };
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[taskA_rev7, taskB]} />);

    // task-a 在 rev7 上完成第一步并展示差异框
    fireEvent.click(view.getAllByLabelText("重新检索后重试")[0]);
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    await settle();
    assert.equal(Boolean(view.queryByText(/missing-p-old/)), true);
    assert.deepEqual(calls, [["task-a", null]]);

    // 用户关闭对话框
    fireEvent.click(view.getByRole("button", { name: "取消" }));
    assert.equal(view.queryByRole("alertdialog") === null, true);

    // 任务刷新为 rev8（仍为 failed）
    const taskA_rev8 = { ...taskA_rev7, revision: 8 };
    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[taskA_rev8, taskB]} />);
    await settle();

    // 重新打开 task-a 的重试对话框
    fireEvent.click(view.getAllByLabelText("重新检索后重试")[0]);

    // 核心断言 1：不得直接弹出旧差异框或显示旧差异产品
    assert.equal(view.queryByText("确认检索产品差异") === null, true, "跨修订重开不得直接展示旧差异对话框");
    assert.equal(view.queryByText(/missing-p-old/) === null, true, "跨修订重开不得展示旧差异内容");

    // 应展示第一步对话框并允许重新调用第一步
    const retryButton = view.queryByRole("button", { name: "重新检索" });
    assert.equal(retryButton !== null, true, "应展示第一步对话框中的重新检索按钮");
    if (retryButton) {
      fireEvent.click(retryButton);
      await settle();
    }

    // 核心断言 2：应以 null 重新向后端请求第一步
    assert.deepEqual(calls, [["task-a", null], ["task-a", null]]);
    // 能够展示来自 rev8 的新差异
    assert.equal(Boolean(view.queryByRole("alertdialog", { name: "确认检索产品差异" })), true);
    assert.equal(Boolean(view.queryByText(/missing-p-new/)), true);

    // 核心断言 3：验证其它任务不受影响
    const cancelBtn = view.queryByRole("button", { name: "取消" });
    if (cancelBtn) fireEvent.click(cancelBtn);
    fireEvent.click(view.getAllByLabelText("重新检索后重试")[1]);
    const retryBtnB = view.queryByRole("button", { name: "重新检索" });
    if (retryBtnB) fireEvent.click(retryBtnB);
    await settle();
    assert.deepEqual(calls, [["task-a", null], ["task-a", null], ["task-b", null]]);
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 R4：第二步 pending 期间关闭并重新打开对话框，保留差异展示且确认按钮禁用不重复发请求", async () => {
  resetTaskFallbackRequestGatesForTests();
  const db = getDb();
  const original = db.retryTaskFallback;
  let secondResolve!: (outcome: TaskRetryFallbackOutcome) => void;
  const calls: Array<[string, string | null]> = [];
  db.retryTaskFallback = ((id: string, batch: string | null = null) => {
    calls.push([id, batch]);
    if (batch === null) {
      return Promise.resolve({
        task: { task_id: id, revision: 7, status: "failed", created: false },
        refreshed_batch_id: "batch-7",
        rebuilt_item_ids: [],
        missing_product_ids: ["missing-p-1"],
        added_product_ids: ["added-p-1"],
        needs_user_confirmation: true,
      });
    }
    // 第二步 pending 不立即返回
    return new Promise<TaskRetryFallbackOutcome>((resolve) => {
      secondResolve = resolve;
    });
  }) as typeof db.retryTaskFallback;

  try {
    const taskA = { ...task("failed"), task_id: "task-a", revision: 7 };
    const view = render(<DesktopDownloadDrawer open variant="page" tasks={[taskA]} />);

    // 1. 完成第一步，弹出差异框
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    fireEvent.click(view.getByRole("button", { name: "重新检索" }));
    await settle();

    assert.deepEqual(calls, [["task-a", null]]);
    assert.equal(Boolean(view.queryByRole("alertdialog", { name: "确认检索产品差异" })), true);
    assert.equal(Boolean(view.queryByText(/missing-p-1/)), true);

    // 2. 点击「确认重建」，进入第二步 pending
    fireEvent.click(view.getByRole("button", { name: "确认重建" }));
    await settle();

    assert.deepEqual(calls, [["task-a", null], ["task-a", "batch-7"]]);
    const confirmBtnPending = view.getByRole("button", { name: "确认重建" }) as HTMLButtonElement;
    assert.equal(confirmBtnPending.disabled, true);

    // 3. 用户在第二步 pending 期间关闭对话框（backdrop 点击关闭）
    fireEvent.click(view.getByRole("alertdialog"));
    assert.equal(view.queryByRole("alertdialog") === null, true);

    // 4. 用户重新打开对话框
    fireEvent.click(view.getByLabelText("重新检索后重试"));

    // 核心断言：必须依然显示差异框，保留差异内容，确认按钮仍然存在且处于 disabled 状态
    assert.equal(Boolean(view.queryByRole("alertdialog", { name: "确认检索产品差异" })), true);
    assert.equal(Boolean(view.queryByText(/missing-p-1/)), true);
    assert.equal(Boolean(view.queryByText(/请求已发出，关闭对话框不会取消服务端操作/)), true);
    const reopenConfirmBtn = view.getByRole("button", { name: "确认重建" }) as HTMLButtonElement;
    assert.equal(reopenConfirmBtn.disabled, true);

    // 尝试在 disabled 上点击，绝不重复调用后端
    fireEvent.click(reopenConfirmBtn);
    await settle();
    assert.deepEqual(calls, [["task-a", null], ["task-a", "batch-7"]], "第二步 pending 关闭重开不得重复调用后端");

    // 5. 第二步返回结果，按既有 reopened -> uncertain 保护：不误关对话框，显示无法安全确认且禁用再次请求
    await act(async () => {
      secondResolve({
        task: { task_id: "task-a", revision: 8, status: "queued", created: false },
        refreshed_batch_id: "batch-7",
        rebuilt_item_ids: [],
        missing_product_ids: [],
        added_product_ids: [],
        needs_user_confirmation: false,
      });
      await Promise.resolve();
    });
    await settle();

    // 既有 reopened 保护转为 uncertain：对话框保留，显示无法安全确认，禁用再次请求
    assert.equal(Boolean(view.queryByRole("alertdialog")), true);
    assert.equal(Boolean(view.queryByText(/无法安全确认/)), true);
    const retryBtn = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
    assert.equal(retryBtn.disabled, true);
    fireEvent.click(retryBtn);
    assert.deepEqual(calls, [["task-a", null], ["task-a", "batch-7"]], "不确定状态不得再次发请求");

    // 即使此后任务快照变更，关闭重开也必须保留第二步的不确定门禁。
    const taskARev8 = { ...taskA, revision: 8 };
    view.rerender(<DesktopDownloadDrawer open variant="page" tasks={[taskARev8]} />);
    await settle();
    fireEvent.click(view.getByRole("alertdialog"));
    fireEvent.click(view.getByLabelText("重新检索后重试"));
    const afterRevision = view.getByRole("button", { name: "重新检索" }) as HTMLButtonElement;
    assert.equal(afterRevision.disabled, true);
    assert.ok(view.getByText(/无法安全确认/));
    fireEvent.click(afterRevision);
    assert.deepEqual(calls, [["task-a", null], ["task-a", "batch-7"]]);
  } finally {
    db.retryTaskFallback = original;
    cleanup();
  }
});

test("W5-10 条目 2：队列与历史任务卡都有「打开目录」", async (t) => {
  for (const status of ["running", "queued", "paused", "cancelled", "interrupted", "failed", "completed"] as const) {
    await t.test(`${status}：有按钮，回调收到 {task_id, revision}`, async () => {
      const calls: unknown[] = [];
      const view = render(
        <TaskCard
          task={task(status)}
          onOpenFolder={(ref: any) => {
            calls.push(ref);
          }}
        />,
      );
      fireEvent.click(view.getByLabelText("打开所在目录"));
      await settle();
      assert.deepEqual(calls, [{ task_id: "task-q-1", revision: 7 }]);
    });
  }

  await t.test("没有保存目录 → 不显示", () => {
    const view = render(<TaskCard task={task("running", "")} onOpenFolder={() => {}} />);
    assert.equal(view.queryByLabelText("打开所在目录") === null, true);
  });

  await t.test("进行中任务修订号不符 → 中文提示", async () => {
    const view = render(
      <TaskCard
        task={task("running")}
        onOpenFolder={async () => {
          throw { ok: false, error: "Tauri application service operation failed", code: "V3_STORE_REVISION_CONFLICT" };
        }}
      />,
    );
    fireEvent.click(view.getByLabelText("打开所在目录"));
    await settle();
    assert.ok(view.getByText("任务状态刚刚变化，请再点一次「打开目录」"));
    assert.equal((view.container.textContent ?? "").includes("V3_"), false);
  });

  await t.test("R4：修订号变化后提示不被立即清掉，用户可关闭", async () => {
    const fail = async () => {
      throw { ok: false, error: "x", code: "V3_STORE_REVISION_CONFLICT" };
    };
    const view = render(<TaskCard task={task("running")} onOpenFolder={fail} />);
    fireEvent.click(view.getByLabelText("打开所在目录"));
    await settle();
    view.rerender(<TaskCard task={{ ...task("running"), revision: 8, updated_at: "2026-10-27T04:08:00Z" }} onOpenFolder={fail} />);
    await settle();
    assert.ok(view.getByText("任务状态刚刚变化，请再点一次「打开目录」"), "修订号变化后提示仍在");
    fireEvent.click(view.getByRole("button", { name: "关闭提示" }));
    assert.equal(view.queryByText("任务状态刚刚变化，请再点一次「打开目录」") === null, true);
  });
});

test("W5-10 R4 P3：详情页提示用按钮字「打开」，修订号变化后仍在；任务标识可选中，复制状态 3 秒后复位", async (t) => {
  await t.test("详情页冲突提示", async () => {
    const fail = async () => {
      throw { ok: false, error: "x", code: "V3_STORE_REVISION_CONFLICT" };
    };
    const view = render(<TaskDetailView task={task("completed")} onOpenFolder={fail} />);
    fireEvent.click(view.getByText("打开"));
    await settle();
    view.rerender(<TaskDetailView task={{ ...task("completed"), revision: 8 }} onOpenFolder={fail} />);
    await settle();
    assert.ok(view.getByText("任务状态刚刚变化，请再点一次「打开」"));
    fireEvent.click(view.getByRole("button", { name: "关闭提示" }));
    assert.equal(view.queryByText("任务状态刚刚变化，请再点一次「打开」") === null, true);
  });

  await t.test("任务标识与复制", async () => {
    const nav = globalThis.navigator as Navigator & { clipboard?: unknown };
    const original = Object.getOwnPropertyDescriptor(nav, "clipboard");
    Object.defineProperty(nav, "clipboard", { configurable: true, value: { writeText: async () => {} } });
    try {
      const view = render(<TaskDetailView task={task("completed")} />);
      fireEvent.click(view.getByRole("button", { name: /技术信息/ }));
      const idEl = view.getByText("task-q-1");
      assert.equal(idEl.className.includes("select-text"), true, idEl.className);
      fireEvent.click(view.getByRole("button", { name: "复制任务标识" }));
      await settle();
      assert.ok(view.getByText("已复制"));
      await settle(3100);
      assert.equal(view.queryByText("已复制") === null, true, "复制状态 3 秒后复位");
      assert.ok(view.getByText("复制"));
    } finally {
      if (original) Object.defineProperty(nav, "clipboard", original);
      else delete (nav as { clipboard?: unknown }).clipboard;
    }
  });
});

test("下载与任务中心增强：图标、倒序置顶、popover vs full page 分层、单条删除、资格过滤与分景动作", async (suite) => {
  await suite.test("popover 不展示时间戳和分景展开且带跳转完整中心按钮；full page 展示创建时间/结束时间/目录/置顶与单条删除", async () => {
    const deletedIds: string[] = [];
    const pinUpdates: Array<{ taskId: string; pinned: boolean }> = [];
    registerTestDesktopShellBridge({
      application: {
        deleteDownloadHistory: async () => {
          throw new Error("should use unified taskDeleteHistory");
        },
        unified: {
          listPinnedTasks: async () => [],
          taskSetPinned: async (taskId: string, pinned: boolean) => {
            pinUpdates.push({ taskId, pinned });
            return pinned ? [taskId] : [];
          },
          taskDeleteHistory: async (taskId: string) => {
            deletedIds.push(taskId);
          },
          taskRetryFallbackEligibility: async () => ({}),
        },
      },
    } as unknown as DesktopShellBridge);

    try {
      const olderQueueTask: ApplicationTask = {
        ...task("queued"),
        task_id: "q-old",
        name: "Older Queue Task",
        created_at: "2026-11-10T08:00:00Z",
        updated_at: "2026-11-10T08:05:00Z",
        output_dir: "/tmp/downloads/q-old",
        scene_ids: ["SCENE_1", "SCENE_2"],
        items_count: 2,
      };
      const newerQueueTask: ApplicationTask = {
        ...task("running"),
        task_id: "q-new",
        name: "Newer Queue Task",
        created_at: "2026-11-15T10:00:00Z",
        updated_at: "2026-11-15T10:05:00Z",
        output_dir: "/tmp/downloads/q-new",
      };
      const completedHistoryTask: ApplicationTask = {
        ...task("completed"),
        task_id: "h-done",
        name: "Completed History Task",
        created_at: "2026-11-14T08:00:00Z",
        updated_at: "2026-11-14T09:30:00Z",
        output_dir: "/tmp/downloads/h-done",
      };

      let openFullCalls = 0;
      const drawerView = render(
        <DesktopDownloadDrawer
          open
          variant="drawer"
          tasks={[olderQueueTask, newerQueueTask, completedHistoryTask]}
          onOpenFullPage={() => {
            openFullCalls += 1;
          }}
        />,
      );
      await settle();

      // Header uses Download icon, not Activity icon; drawer hides timestamps & expandable scene chevrons
      assert.equal(drawerView.container.querySelector(".lucide-activity"), null);
      assert.ok(drawerView.container.querySelector(".lucide-download"));
      assert.equal(drawerView.queryByText(/创建时间/), null);
      assert.equal(drawerView.queryByText(/结束时间/), null);
      assert.equal(drawerView.queryByRole("button", { name: "展开分景列表" }), null);
      assert.ok(drawerView.queryByTestId("download-drawer-footer-full-page-btn") === null);
      const headerFullBtn = drawerView.getByTestId("download-drawer-open-full-page-btn");
      fireEvent.click(headerFullBtn);
      assert.equal(openFullCalls, 1);
      cleanup();

      const pageView = render(
        <DesktopDownloadDrawer
          open
          variant="page"
          tasks={[olderQueueTask, newerQueueTask, completedHistoryTask]}
        />,
      );
      await settle();

      // Reverse chronological: q-new before q-old
      const initialCards = Array.from(
        pageView.container.querySelectorAll("[data-task-id]"),
      ).map((el) => el.getAttribute("data-task-id"));
      assert.deepEqual(initialCards.slice(0, 2), ["q-new", "q-old"]);

      // Full page surfaces 创建时间, terminal 结束时间, and 目录
      assert.ok(pageView.getAllByText(/创建时间/).length >= 3);
      assert.ok(pageView.getAllByText(/结束时间/).length >= 1);
      assert.ok(pageView.getByText(/\/tmp\/downloads\/h-done/));

      // Pin older queue task -> moves above newer queue task and persists
      const pinBtns = pageView.getAllByRole("button", { name: "置顶任务" });
      fireEvent.click(pinBtns[1]); // q-old
      await settle();
      assert.deepEqual(pinUpdates, [{ taskId: "q-old", pinned: true }]);
      const reorderedCards = Array.from(
        pageView.container.querySelectorAll("[data-task-id]"),
      ).map((el) => el.getAttribute("data-task-id"));
      assert.deepEqual(reorderedCards.slice(0, 2), ["q-old", "q-new"]);

      // Single history delete invokes unified taskDeleteHistory without requiring clear-all
      fireEvent.click(pageView.getByTestId("delete-history-task-h-done"));
      await settle();
      assert.deepEqual(deletedIds, ["h-done"]);
    } finally {
      clearTestDesktopShellBridge();
      window.localStorage.removeItem("eostage.downloadPriorityTaskIds.v1");
    }
  });

  await suite.test("不可重检失败任务隐藏兜底按钮；确定性 TypedError 直接提示不包「未能确认」", async () => {
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        unified: {
          taskRetryFallbackEligibility: async () => ({
            "f-eligible": true,
            "f-ineligible": false,
          }),
          taskRetryFallback: async () => {
            throw Object.assign(new Error("Earthdata 凭证缺失"), {
              code: "CREDENTIAL_MISSING",
              message_zh: "重检需要有效的 Earthdata 登录，请登录后再试；原任务未改动",
              retryable: false,
            });
          },
        },
      },
    } as unknown as DesktopShellBridge);

    try {
      const eligible: ApplicationTask = { ...task("failed"), task_id: "f-eligible", name: "Eligible" };
      const ineligible: ApplicationTask = { ...task("failed"), task_id: "f-ineligible", name: "Ineligible" };
      const view = render(
        <DesktopDownloadDrawer open variant="page" tasks={[eligible, ineligible]} />,
      );
      await settle();

      const fallbackButtons = view.getAllByLabelText("重新检索后重试");
      assert.equal(fallbackButtons.length, 1, "Only eligible failed task shows retry_fallback");

      fireEvent.click(fallbackButtons[0]);
      fireEvent.click(view.getByRole("button", { name: "重新检索" }));
      await settle();

      assert.ok(view.getByText("重检需要有效的 Earthdata 登录，请登录后再试；原任务未改动"));
      assert.equal(
        view.queryByText(/请求已发出，结果未能确认/),
        null,
        "Deterministic TypedError must not wrap uncertain prefix",
      );
    } finally {
      clearTestDesktopShellBridge();
    }
  });

  await suite.test("完整下载中心支持对多景任务逐景暂停、继续、重试、取消并直传 item_id", async () => {
    const itemActions: Array<{ taskId: string; itemId: string; action: UnifiedTaskItemAction }> = [];
    const makePageItem = (
      itemId: string,
      displayName: string,
      status: string,
      bytesCompleted: number,
      failure: { code: string; reason_zh: string; retryable: boolean } | null = null,
    ) => ({
      ordinal: 0,
      item: {
        item_id: itemId,
        provider_id: "asf",
        display_name: displayName,
        acquisition_datetime: null,
        footprint_geojson: null,
        metadata: {},
        payload: {},
        asset_access: null,
        execution_policy: null,
      },
      checked: true,
      status,
      bytes_completed: bytesCompleted,
      total_bytes: 100,
      source_total_bytes: 100,
      bytes_per_second: 0,
      attempt: 0,
      destination: null,
      failure,
      finished_at: null,
      retryable: status === "failed",
      local_reuse: null,
    });

    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        taskPage: async () => ({
          ok: true,
          page: {
            task_id: "multi-scene-1",
            revision: 3,
            page_index: 0,
            page_size: MAX_UI_TASK_PAGE_SIZE,
            total_count: 3,
            source_total_bytes: 300,
            network_bytes_received: 70,
            items: [
              makePageItem("it-a", "SCENE_A", "active", 50),
              makePageItem("it-b", "SCENE_B", "paused", 20),
              makePageItem("it-c", "SCENE_C", "failed", 0, {
                code: "TIMEOUT",
                reason_zh: "单景超时",
                retryable: true,
              }),
            ],
          },
        }),
        unified: {
          taskItemAction: async (
            taskId: string,
            _revision: number | null,
            itemId: string,
            action: UnifiedTaskItemAction,
          ) => {
            itemActions.push({ taskId, itemId, action });
            return { task_id: taskId, revision: 4, status: "running", created: false };
          },
        },
      },
    } as unknown as DesktopShellBridge);

    try {
      const multiTask: ApplicationTask = {
        ...task("running"),
        task_id: "multi-scene-1",
        revision: 3,
        name: "Multi Scene Task",
        items_count: 3,
        scene_ids: ["SCENE_A", "SCENE_B", "SCENE_C"],
        scene_items: [
          { item_id: "it-a", scene_id: "SCENE_A", status: "running", done_bytes: 50, total_bytes: 100 },
          { item_id: "it-b", scene_id: "SCENE_B", status: "paused", done_bytes: 20, total_bytes: 100 },
          { item_id: "it-c", scene_id: "SCENE_C", status: "failed", done_bytes: 0, total_bytes: 100 },
        ],
      };
      const view = render(<DesktopDownloadDrawer open variant="page" tasks={[multiTask]} />);
      fireEvent.click(view.getByRole("button", { name: "展开分景进度" }));
      await settle();

      assert.ok(view.getByText("单景超时"));
      fireEvent.click(view.getByRole("button", { name: "暂停 SCENE_A" }));
      await settle();
      fireEvent.click(view.getByRole("button", { name: "继续 SCENE_B" }));
      await settle();
      fireEvent.click(view.getByRole("button", { name: "重试 SCENE_C" }));
      await settle();
      fireEvent.click(view.getByRole("button", { name: "取消 SCENE_A" }));
      await settle();

      assert.deepEqual(itemActions, [
        { taskId: "multi-scene-1", itemId: "it-a", action: "pause" },
        { taskId: "multi-scene-1", itemId: "it-b", action: "resume" },
        { taskId: "multi-scene-1", itemId: "it-c", action: "retry" },
        { taskId: "multi-scene-1", itemId: "it-a", action: "cancel" },
      ]);
    } finally {
      clearTestDesktopShellBridge();
    }
  });

  await suite.test("任务卡片规范化命名、行首时间不挤占、目录行置于摘要行之上", async () => {
    const longDemTask: ApplicationTask = {
      ...task("running"),
      task_id: "dem-cop30-long-1",
      name: "COP30",
      provider: "compat.dem",
      dataset: "COP30",
      created_at: "2026-11-15T21:35:21Z",
      updated_at: "2026-11-15T21:36:10Z",
      output_dir: "/tmp/downloads/cop30_egm2008",
      items_count: 1,
      scene_ids: ["COP30_EGM2008_20261116_20c266c5.tif"],
    };
    const superLongTitleTask: ApplicationTask = {
      ...task("completed"),
      task_id: "long-title-s1-2",
      name: "Sentinel_1A_IW_SLC_Very_Long_Track_Name_That_Must_Truncate_Without_Squeezing_Created_Timestamps_Ever_In_Row_One",
      provider: "compat.sar",
      dataset: "Sentinel-1",
      created_at: "2026-11-15T10:00:00Z",
      updated_at: "2026-11-15T10:12:00Z",
      output_dir: "/tmp/downloads/long_s1_slc",
    };

    const view = render(
      <DesktopDownloadDrawer
        open
        variant="page"
        tasks={[longDemTask, superLongTitleTask]}
      />,
    );
    await settle();

    const demCard = view.container.querySelector('[data-task-id="dem-cop30-long-1"]') as HTMLElement;
    assert.ok(demCard);
    assert.ok(demCard.textContent?.includes("Copernicus DEM 30m (GLO-30)"));
    assert.ok(demCard.textContent?.includes("DEM 高程"));
    assert.ok(demCard.textContent?.includes("Copernicus / OpenTopography"));
    assert.equal(demCard.textContent?.includes("compat.dem"), false);

    // 行 1 含时间戳容器，且具有 shrink-0 whitespace-nowrap 类防止被超长标题挤占
    const demTs = demCard.querySelector('[data-testid="task-header-timestamps"]') as HTMLElement;
    assert.ok(demTs);
    assert.ok(demTs.className.includes("shrink-0"));
    assert.ok(demTs.className.includes("whitespace-nowrap"));
    assert.ok(demTs.textContent?.includes("创建时间"));

    // 目录行在 DOM 顺序中严格位于 summary 摘要行之前
    const demDir = demCard.querySelector('[data-testid="task-page-metadata"]') as HTMLElement;
    const demSummary = demCard.querySelector('[data-testid="task-summary-line"]') as HTMLElement;
    assert.ok(demDir && demSummary);
    assert.ok(demDir.textContent?.includes("目录 /tmp/downloads/cop30_egm2008"));
    const relMask = demDir.compareDocumentPosition(demSummary);
    assert.equal(
      Boolean(relMask & Node.DOCUMENT_POSITION_FOLLOWING),
      true,
      "Directory row must appear above the summary line",
    );

    const longCard = view.container.querySelector('[data-task-id="long-title-s1-2"]') as HTMLElement;
    assert.ok(longCard);
    const longTs = longCard.querySelector('[data-testid="task-header-timestamps"]') as HTMLElement;
    assert.ok(longTs?.textContent?.includes("创建时间"));
    assert.ok(longTs?.textContent?.includes("结束时间"));
    const titleSpan = longCard.querySelector("span.truncate") as HTMLElement;
    assert.ok(titleSpan?.className.includes("min-w-0"));
    assert.ok(titleSpan?.className.includes("shrink"));
  });
});
