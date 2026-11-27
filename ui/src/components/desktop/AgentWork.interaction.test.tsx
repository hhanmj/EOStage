import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentWorkPanel } from "./AgentWorkPanel";
import type { AgentWorkItem } from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "../../shell/api/types.ts";

test.afterEach(() => {
  cleanup();
});

function work(id: string, status: AgentWorkItem["run"]["status"], started: string, request: string, tasks: string[]): AgentWorkItem {
  return {
    run: {
      run_id: id,
      session_id: "sess-1",
      status,
      started_at: started,
      ended_at: status === "running" ? null : started.replace(":00Z", ":30Z"),
      model_route: "channel/model-a",
      usage_total: id === "run-old" ? { prompt: 1200, completion: 300, total: 1500 } : null,
    },
    request,
    plan_ids: [],
    created_task_ids: tasks,
  };
}

function task(id: string, name: string, status: ApplicationTask["status"], done: number, total: number): ApplicationTask {
  return { task_id: id, revision: 1, category: "scene_download", name, status, done_bytes: done, total_bytes: total };
}

// 后端给出的顺序故意打乱，验证面板自己按开始时间排序
const fakeWork: AgentWorkItem[] = [
  work("run-old", "completed", "2026-10-30T09:00:00Z", "下载上海 Sentinel-1", ["task-a", "task-gone"]),
  work("run-new", "running", "2026-10-30T11:00:00Z", "检索北京 ALOS-2", ["task-b"]),
];
const fakeTasks: ApplicationTask[] = [
  task("task-a", "上海 S1 批次", "running", 50, 100),
  task("task-b", "北京 ALOS-2 批次", "queued", 0, 0),
];

test("AgentWorkPanel 任务与成果交互", async (suite) => {
  await suite.test("无会话时显示空态", () => {
    const view = render(<AgentWorkPanel work={null} tasks={fakeTasks} />);
    assert.ok(view.getByText("暂无运行"));
  });

  await suite.test("运行按开始时间新的在前", () => {
    const view = render(<AgentWorkPanel work={fakeWork} tasks={fakeTasks} />);
    const list = view.getByRole("list", { name: "运行列表" });
    const buttons = Array.from(list.querySelectorAll("button"));
    assert.equal(buttons.length, 2);
    assert.ok(buttons[0].textContent?.includes("检索北京 ALOS-2"));
    assert.ok(buttons[1].textContent?.includes("下载上海 Sentinel-1"));
    assert.ok(buttons[0].textContent?.includes("执行中"));
    assert.ok(buttons[1].textContent?.includes("已完成"));
    // 默认选中最新的运行
    assert.equal(buttons[0].getAttribute("aria-pressed"), "true");
  });

  await suite.test("选中运行显示其创建的任务与状态，缺失任务显示已清理，并显示用量", () => {
    const view = render(<AgentWorkPanel work={fakeWork} tasks={fakeTasks} onOpenTask={() => {}} />);
    fireEvent.click(view.getByText("下载上海 Sentinel-1"));
    const tasks = view.getByRole("list", { name: "创建的任务" });
    const text = tasks.textContent ?? "";
    assert.ok(text.includes("上海 S1 批次"));
    assert.ok(text.includes("下载中"));
    assert.ok(text.includes("50%"));
    assert.ok(text.includes("task-gone"));
    assert.ok(text.includes("已清理"));
    assert.ok(view.getByText("用量 输入 1,200 · 输出 300"));
    // 已清理的任务不可点击
    assert.equal(tasks.querySelectorAll("button").length, 1);
  });

  await suite.test("点击任务以其 id 调用打开下载中心回调", () => {
    const opened: string[] = [];
    const view = render(<AgentWorkPanel work={fakeWork} tasks={fakeTasks} onOpenTask={(id) => opened.push(id)} />);
    fireEvent.click(view.getByRole("button", { name: "北京 ALOS-2 批次 等待中" }));
    assert.deepEqual(opened, ["task-b"]);
  });

  await suite.test("筛选：已结束只显示已结束的运行", () => {
    const view = render(<AgentWorkPanel work={fakeWork} tasks={fakeTasks} />);
    fireEvent.click(view.getByRole("radio", { name: "已结束 1" }));
    const list = view.getByRole("list", { name: "运行列表" });
    assert.equal(list.querySelectorAll("button").length, 1);
    assert.ok(list.textContent?.includes("下载上海 Sentinel-1"));
    assert.ok(view.getByRole("radio", { name: "全部 2" }));
    assert.ok(view.getByRole("radio", { name: "进行中 1" }));
  });
});
