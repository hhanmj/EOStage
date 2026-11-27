import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  createTaskOverviewSource,
  taskSnapshotChanged,
  type TaskOverviewVisibilityDependencies,
} from "./taskOverviewSource.ts";
import type { ApplicationTask } from "./types.ts";
import type { PollerTimerDependencies } from "./poller.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function createFakeTimers() {
  const intervals = new Map<number, { fn: () => void; ms: number }>();
  let nextId = 1;

  const timers: PollerTimerDependencies = {
    setInterval: (fn, ms) => {
      const id = nextId++;
      intervals.set(id, { fn, ms });
      return id;
    },
    clearInterval: (id) => {
      intervals.delete(id as number);
    },
  };

  const tick = async () => {
    for (const entry of Array.from(intervals.values())) {
      entry.fn();
    }
    await new Promise((resolve) => setImmediate(resolve));
  };

  return { timers, intervals, tick };
}

function makeSampleTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: "task-ext-001",
    revision: 1,
    category: "scene_download",
    name: "Sentinel-1 外部新建下载任务",
    provider: "ASF",
    dataset: "Sentinel-1",
    status: "running",
    done_bytes: 0,
    total_bytes: 104857600,
    ...overrides,
  };
}

test("W5-8 R1 - TaskOverviewSource 单一任务总览订阅源测试套件", async (suite) => {
  await suite.test(
    "1. 订阅者依次收到初始空列表 []、第 2 次轮询外部新建任务、第 3 次轮询外部 revision 抬升",
    async () => {
      const { timers, intervals, tick } = createFakeTimers();
      const timeline: ApplicationTask[][] = [
        [],
        [makeSampleTask({ revision: 1, status: "running" })],
        [makeSampleTask({ revision: 2, status: "paused", done_bytes: 52428800 })],
      ];
      let fetchCallCount = 0;

      const source = createTaskOverviewSource({
        fetchTasks: async () => {
          const idx = Math.min(fetchCallCount, timeline.length - 1);
          fetchCallCount += 1;
          return timeline[idx];
        },
        pollIntervalMs: 1000,
        timers,
      });

      const received: ApplicationTask[][] = [];
      const unsubscribe = source.subscribe((tasks) => {
        received.push(tasks);
      });

      // 等待首次订阅触发的立即拉取 (#1)
      await new Promise((resolve) => setImmediate(resolve));
      assert.strictEqual(fetchCallCount, 1);
      assert.strictEqual(received.length, 1);
      assert.deepStrictEqual(received[0], []);
      assert.strictEqual(
        intervals.size,
        1,
        "即使初始列表为空（0 个活动任务），有订阅者期间定时器必须保持活跃，严禁空闲停"
      );

      // 两次轮询之间外部创建任务 -> 第 2 次轮询 tick (#2)
      await tick();
      assert.strictEqual(fetchCallCount, 2);
      assert.strictEqual(received.length, 2);
      assert.strictEqual(received[1].length, 1);
      assert.strictEqual(received[1][0].task_id, "task-ext-001");
      assert.strictEqual(received[1][0].revision, 1);
      assert.strictEqual(received[1][0].status, "running");

      // 外部状态变为 paused 且 revision 抬升至 2 -> 第 3 次轮询 tick (#3)
      await tick();
      assert.strictEqual(fetchCallCount, 3);
      assert.strictEqual(received.length, 3);
      assert.strictEqual(received[2][0].revision, 2);
      assert.strictEqual(received[2][0].status, "paused");
      assert.deepStrictEqual(source.getLatest(), timeline[2]);

      unsubscribe();
      assert.strictEqual(intervals.size, 0, "最后一个订阅者退订后必须停止全部定时器");
    }
  );

  await suite.test("2. 退订停止轮询且容忍 React StrictMode 双挂载/卸载", async () => {
    const { timers, intervals, tick } = createFakeTimers();
    let fetchCount = 0;

    const source = createTaskOverviewSource({
      fetchTasks: async () => {
        fetchCount += 1;
        return [makeSampleTask({ revision: fetchCount })];
      },
      timers,
    });

    const received1: ApplicationTask[][] = [];
    const received2: ApplicationTask[][] = [];

    // StrictMode: mount 1 -> unmount 1 -> mount 2
    const unsub1 = source.subscribe((list) => received1.push(list));
    unsub1();
    assert.strictEqual(intervals.size, 0, "第一次卸载后定时器已清理");

    const unsub2 = source.subscribe((list) => received2.push(list));
    assert.strictEqual(intervals.size, 1, "第二次挂载后重新启动单个定时器");

    await new Promise((resolve) => setImmediate(resolve));
    assert.ok(received2.length >= 1, "第二次挂载的订阅者必须收到任务快照");

    await tick();
    const countBeforeUnsub = fetchCount;
    unsub2();
    // 幂等退订
    unsub2();
    assert.strictEqual(intervals.size, 0, "全部退订后无残留定时器");

    await tick();
    assert.strictEqual(fetchCount, countBeforeUnsub, "退订后 tick 不再触发 fetchTasks");
  });

  await suite.test("3. 拉取报错时保留上一份列表，不抛出异常中断轮询环", async () => {
    const { timers, tick } = createFakeTimers();
    const initialTask = makeSampleTask({ revision: 1, status: "running" });
    const recoveredTask = makeSampleTask({ revision: 2, status: "paused" });
    let callIdx = 0;

    const source = createTaskOverviewSource({
      fetchTasks: async () => {
        callIdx += 1;
        if (callIdx === 1) return [initialTask];
        if (callIdx === 2) throw new Error("IPC transient error");
        return [recoveredTask];
      },
      timers,
    });

    const received: ApplicationTask[][] = [];
    const unsub = source.subscribe((list) => received.push(list));
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepStrictEqual(source.getLatest(), [initialTask]);
    assert.strictEqual(received.length, 1);

    // 第 2 次拉取报错：保留上一份列表，不崩溃
    await tick();
    assert.deepStrictEqual(source.getLatest(), [initialTask], "拉取报错后 getLatest 必须保留上一份列表");
    assert.strictEqual(received.length, 1, "拉取失败时不向监听器推送脏数据");

    // 第 3 次拉取恢复正常：轮询环仍在继续
    await tick();
    assert.deepStrictEqual(source.getLatest(), [recoveredTask], "报错恢复后继续更新列表");
    assert.strictEqual(received.length, 2);

    unsub();
  });

  await suite.test(
    "4. refreshNow() 并发合并：飞行中多次调用只保持 1 个 in-flight 且最多调度 1 次尾随拉取",
    async () => {
      const { timers } = createFakeTimers();
      let inFlight = 0;
      let maxInFlight = 0;
      let totalFetches = 0;
      const resolvers: Array<() => void> = [];

      const source = createTaskOverviewSource({
        fetchTasks: () =>
          new Promise<ApplicationTask[]>((resolve) => {
            inFlight += 1;
            maxInFlight = Math.max(maxInFlight, inFlight);
            totalFetches += 1;
            const currentFetchNum = totalFetches;
            resolvers.push(() => {
              inFlight -= 1;
              resolve([makeSampleTask({ revision: currentFetchNum })]);
            });
          }),
        timers,
      });

      const received: ApplicationTask[][] = [];
      // 不通过 subscribe 触发，或者先处理完初始拉取，再专门测试并发 refreshNow
      const p1 = source.refreshNow();
      // 在 p1 飞行期间连续发起 3 次并发 refreshNow
      const p2 = source.refreshNow();
      const p3 = source.refreshNow();
      const p4 = source.refreshNow();

      assert.strictEqual(maxInFlight, 1, "同一时刻最多只能有 1 个 in-flight 拉取");
      assert.strictEqual(totalFetches, 1, "飞行期间未完成前不得立即发起第 2 个并发请求");

      const unsub = source.subscribe((list) => received.push(list));
      // 完成第 1 次拉取
      resolvers.shift()!();
      await new Promise((resolve) => setImmediate(resolve));

      assert.strictEqual(maxInFlight, 1, "尾随拉取也只能单飞");
      assert.strictEqual(totalFetches, 2, "多次重叠调用必须合并为最多 1 次尾随拉取");

      // 完成第 2 次（尾随）拉取
      resolvers.shift()!();
      await Promise.all([p1, p2, p3, p4]);

      assert.strictEqual(totalFetches, 2, "尾随拉取完成后不得进入循环拉取");
      assert.strictEqual(source.getLatest()?.[0]?.revision, 2);
      unsub();
    }
  );

  await suite.test(
    "5. DesktopWorkbench 源码接线断言：使用 createTaskOverviewSource 且移除空闲停门禁",
    () => {
      const wbPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
      const content = fs.readFileSync(wbPath, "utf-8");

      assert.match(
        content,
        /createTaskOverviewSource/,
        "DesktopWorkbench.tsx 必须引入并使用 createTaskOverviewSource"
      );
      assert.doesNotMatch(
        content,
        /const\s+pollActive\s*=\s*downloadDrawerOpenRef\.current\s*\|\|\s*hasActive/,
        "DesktopWorkbench.tsx 必须移除 pollActive = downloadDrawerOpenRef.current || hasActive 空闲停门禁"
      );
      assert.doesNotMatch(
        content,
        /pollerRef\.current\?\.setActiveTasks\(/,
        "DesktopWorkbench.tsx 不得再通过 pollerRef.current?.setActiveTasks 门控任务轮询"
      );
      assert.match(
        content,
        /taskSnapshotChanged/,
        "DesktopWorkbench.tsx 的 handleTaskSnapshot 必须使用 taskSnapshotChanged 避免无变化时每秒重渲染"
      );
    }
  );

  await suite.test(
    "6. 窗口可见性降频与恢复：hidden 时轮询间隔降为 10000ms（不空闲停）；触发 visibilitychange 回到 visible 时立即 refreshNow 并恢复 1000ms；退订后停止",
    async () => {
      const { timers, intervals } = createFakeTimers();
      let hidden = true;
      const visHandlers = new Set<() => void>();
      const visibility: TaskOverviewVisibilityDependencies = {
        isHidden: () => hidden,
        onVisibilityChange: (fn) => {
          visHandlers.add(fn);
          return () => {
            visHandlers.delete(fn);
          };
        },
      };

      let fetchCount = 0;
      const source = createTaskOverviewSource({
        fetchTasks: async () => {
          fetchCount += 1;
          return [makeSampleTask({ revision: fetchCount })];
        },
        pollIntervalMs: 1000,
        hiddenPollIntervalMs: 10000,
        timers,
        visibility,
      });

      const unsub = source.subscribe(() => {});
      await new Promise((resolve) => setImmediate(resolve));

      assert.strictEqual(fetchCount, 1, "首次订阅立即拉取一次");
      assert.strictEqual(intervals.size, 1, "hidden 状态下仍保持 1 个活跃定时器（绝不空闲停）");
      const hiddenTimer = Array.from(intervals.values())[0];
      assert.strictEqual(
        hiddenTimer?.ms,
        10000,
        "当 document.visibilityState === 'hidden' 时轮询间隔必须降为 10000ms (10s)"
      );

      // 模拟窗口切回前台 visible
      hidden = false;
      for (const fn of Array.from(visHandlers)) {
        fn();
      }
      await new Promise((resolve) => setImmediate(resolve));

      assert.strictEqual(fetchCount, 2, "回到 visible 时必须立即触发一次 refreshNow()");
      assert.strictEqual(intervals.size, 1);
      const visibleTimer = Array.from(intervals.values())[0];
      assert.strictEqual(
        visibleTimer?.ms,
        1000,
        "回到 visible 后轮询间隔必须恢复为 1000ms (1s)"
      );

      // 再次切到后台 hidden
      hidden = true;
      for (const fn of Array.from(visHandlers)) {
        fn();
      }
      assert.strictEqual(intervals.size, 1);
      assert.strictEqual(
        Array.from(intervals.values())[0]?.ms,
        10000,
        "再次切入 hidden 时轮询间隔重新降为 10000ms"
      );

      unsub();
      assert.strictEqual(intervals.size, 0, "退订后清除定时器");
      assert.strictEqual(visHandlers.size, 0, "退订后移除 visibilitychange 监听");
    }
  );

  await suite.test(
    "7. taskSnapshotChanged：相同任务快照签名返回 false（避免空闲每秒重渲染），状态/进度/修订变化返回 true",
    () => {
      const t1 = makeSampleTask({
        task_id: "t-1",
        status: "running",
        revision: 1,
        done_bytes: 1024,
        total_bytes: 4096,
        updated_at: "2026-10-27T01:00:00Z",
      });
      const t1Clone = makeSampleTask({
        task_id: "t-1",
        status: "running",
        revision: 1,
        done_bytes: 1024,
        total_bytes: 4096,
        updated_at: "2026-10-27T01:00:00Z",
      });

      assert.strictEqual(
        taskSnapshotChanged(null, [t1]),
        true,
        "初始无基线时应视为变化"
      );
      assert.strictEqual(
        taskSnapshotChanged([t1], [t1Clone]),
        false,
        "同签名快照不得判定为变化（避免无变化重渲染）"
      );
      assert.strictEqual(
        taskSnapshotChanged([], []),
        false,
        "连续两次空列表不得判定为变化"
      );
      assert.strictEqual(
        taskSnapshotChanged([t1], [makeSampleTask({ ...t1, done_bytes: 2048 })]),
        true,
        "done_bytes 变化时应判定为变化"
      );
      assert.strictEqual(
        taskSnapshotChanged([t1], [makeSampleTask({ ...t1, revision: 2 })]),
        true,
        "revision 变化时应判定为变化"
      );
      assert.strictEqual(
        taskSnapshotChanged([t1], [makeSampleTask({ ...t1, status: "paused" })]),
        true,
        "status 变化时应判定为变化"
      );
    }
  );
});
