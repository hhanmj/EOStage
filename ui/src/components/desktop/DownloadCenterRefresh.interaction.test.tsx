import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useEffect, useState } from "react";
import { act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { DownloadCenterPage } from "./DownloadCenterPage";
import {
  getTasks,
  taskAction,
  isFixtureBackend,
  __resetFixtureState,
  type ApplicationTask,
} from "../../shell/api";
import { getDb } from "../../shell/api/fixture";
import { createTaskOverviewSource } from "../../shell/api/taskOverviewSource";
import type { PollerTimerDependencies } from "../../shell/api/poller";

test.afterEach(() => {
  cleanup();
  __resetFixtureState();
});

function createManualTimers() {
  const intervals = new Map<number, () => void>();
  let nextId = 1;

  const timers: PollerTimerDependencies = {
    setInterval: (fn) => {
      const id = nextId++;
      intervals.set(id, fn);
      return id;
    },
    clearInterval: (id) => {
      intervals.delete(id as number);
    },
  };

  const tick = async () => {
    await act(async () => {
      for (const fn of Array.from(intervals.values())) {
        fn();
      }
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  return { timers, intervals, tick };
}

test("W5-8 R1 - 下载中心外部任务刷新与修订冲突单次重试交互测试套件", async (suite) => {
  await suite.test(
    "1. 初始 0 任务下外部新建任务在第 2 次轮询自动入列、外部暂停抬升 revision 更新状态、取消遇 V3_STORE_REVISION_CONFLICT 自动取新 revision 重试成功并移出队列",
    async () => {
      assert.strictEqual(isFixtureBackend(), true, "jsdom 交互环境必须走 fixture 后端");

      // (a) 启动时 0 个任务
      __resetFixtureState();
      const db = getDb();
      db.clearTasks__fixture();
      assert.strictEqual((await getTasks()).length, 0);

      const { timers, intervals, tick } = createManualTimers();
      const source = createTaskOverviewSource({
        fetchTasks: getTasks,
        pollIntervalMs: 1000,
        timers,
      });

      const actionCalls: Array<{
        taskId: string;
        revision: number;
        action: "pause" | "resume" | "retry" | "cancel";
      }> = [];

      const scriptedTaskAction = async (
        taskId: string,
        revision: number,
        action: "pause" | "resume" | "retry" | "cancel"
      ) => {
        actionCalls.push({ taskId, revision, action });
        if (revision === 2) {
          const conflictErr = Object.assign(new Error("revision conflict"), {
            code: "V3_STORE_REVISION_CONFLICT",
          });
          throw conflictErr;
        }
        await taskAction(taskId, revision, action);
      };

      function TestHarness() {
        const [tasks, setTasks] = useState<ApplicationTask[]>([]);
        useEffect(() => {
          return source.subscribe((fresh) => {
            setTasks(fresh);
          });
        }, []);

        return (
          <DownloadCenterPage
            tasks={tasks}
            onTaskAction={scriptedTaskAction}
          />
        );
      }

      const view = render(<TestHarness />);

      // 等待首次订阅触发的初始拉取（第 1 次轮询）
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });

      assert.ok(view.queryByText("暂无排队任务"), "第 1 次轮询后队列为空");
      assert.strictEqual(
        intervals.size,
        1,
        "即使初始队列为 0 项，订阅源定时器也必须持续运行，不得空闲停"
      );

      // (b) 两次轮询之间由外部（Agent 或工作台）在 fixture db 创建任务
      const createdRef = db.createTask(
        "batch-s1-init",
        1,
        ["S1A_IW_SLC__1SDV_20240410T100122_20240410T100149_053370_06771F_C726"],
        { output_root: "D:/ScenePort/Downloads" }
      );
      db.patchTask__fixture(createdRef.task_id, {
        name: "Agent 外部新建 Sentinel-1 任务",
        status: "running",
        revision: 1,
      });

      // 第 2 次轮询 tick：无需重载或路由切换，队列直接出现该外部新建任务
      await tick();

      assert.strictEqual(
        view.queryByText("暂无排队任务"),
        null,
        "第 2 次轮询后空态提示应消失"
      );
      assert.ok(
        view.queryByText("Agent 外部新建 Sentinel-1 任务"),
        "第 2 次轮询后无需重载即可在下载中心队列看到外部新建的任务"
      );
      assert.ok(view.queryByText("下载中"), "任务初始状态显示为下载中");

      // (c) 外部暂停该任务并抬升 revision 至 2
      db.patchTask__fixture(createdRef.task_id, {
        status: "paused",
        revision: 2,
      });

      // 第 3 次轮询 tick：卡片状态更新为「已暂停」
      await tick();

      assert.ok(
        view.getAllByText("已暂停").length >= 1,
        "外部暂停并抬升 revision 后，卡片状态更新为已暂停"
      );

      // (d) 在用户点击「取消」前，外部又将后端/db 的 revision 抬升至 3（UI 卡片仍持有旧 revision 2）
      db.patchTask__fixture(createdRef.task_id, {
        revision: 3,
      });

      const cancelBtn = view.getByRole("button", { name: "取消任务" });
      await act(async () => {
        fireEvent.click(cancelBtn);
        await new Promise((resolve) => setTimeout(resolve, 0));
      });

      assert.deepStrictEqual(
        actionCalls,
        [
          { taskId: createdRef.task_id, revision: 2, action: "cancel" },
          { taskId: createdRef.task_id, revision: 3, action: "cancel" },
        ],
        "抽屉取消动作遇到 V3_STORE_REVISION_CONFLICT 时必须刷新最新 revision (3) 并自动重试一次"
      );
      assert.strictEqual(
        view.queryByRole("alert"),
        null,
        "冲突重试成功后卡片上不得显示失败错误条"
      );

      // 下一次轮询 tick：已取消的任务从执行队列消失（移入历史记录）
      await tick();

      assert.ok(
        view.queryByText("暂无排队任务"),
        "取消成功后的下一次轮询 tick，任务必须从执行队列移出"
      );
      assert.ok(
        view.getAllByText("已取消").length >= 1,
        "已取消的任务在历史记录分区显示为已取消"
      );
    }
  );

  await suite.test(
    "2. pendingPreparations 徽章按数据源动态插值（S-Band 绝不硬编码 Landsat）",
    async () => {
      __resetFixtureState();
      const view = render(
        <DownloadCenterPage
          pendingPreparations={[
            {
              id: "prep-sband-1",
              title: "NISAR S 波段 SAR",
              count: 2,
              outputRoot: "/tmp/out",
              status: "preparing",
            },
          ]}
        />
      );
      assert.ok(view.getByText("NISAR S 波段 SAR"));
      assert.ok(view.getByText("正在创建下载任务…"));
      assert.equal(view.queryByText(/Landsat/), null);
    }
  );

  await suite.test(
    "3. Landsat 官方原始归档长标题准备卡完整保留标题且不在状态胶囊重复拼接",
    async () => {
      __resetFixtureState();
      const longTitle = "Landsat C2 L2 · 官方原始归档 (.tar)";
      const view = render(
        <DownloadCenterPage
          pendingPreparations={[
            {
              id: "prep-landsat-1",
              title: longTitle,
              count: 1,
              outputRoot: "C:\\Downloads",
              status: "preparing",
            },
          ]}
        />
      );
      const titleNode = view.getByTitle(longTitle);
      assert.equal(titleNode.textContent, longTitle);
      assert.equal(view.getAllByText("正在创建下载任务…").length, 1);
      assert.equal(view.queryByText(`正在创建 ${longTitle} 下载任务…`), null);
    }
  );
});
