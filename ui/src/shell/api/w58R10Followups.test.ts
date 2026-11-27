import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createLatestResultGate } from "./latestResultGate.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

function read(rel: string): string {
  return fs.readFileSync(path.resolve(here, rel), "utf-8");
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

test("W5-8 R10 F2：活动状态只采用最后发出的请求的结果", async (t) => {
  await t.test("先发的请求晚返回 → 丢弃，不覆盖新值", async () => {
    const gate = createLatestResultGate();
    const applied: number[] = [];
    const first = deferred<number>();
    const second = deferred<number>();
    const p1 = gate.run(() => first.promise, (v) => applied.push(v));
    const p2 = gate.run(() => second.promise, (v) => applied.push(v));
    second.resolve(1);
    await p2;
    first.resolve(0);
    await p1;
    assert.deepEqual(applied, [1], `实际写入顺序：${JSON.stringify(applied)}`);
  });

  await t.test("按顺序返回 → 只写最后一次", async () => {
    const gate = createLatestResultGate();
    const applied: number[] = [];
    const first = deferred<number>();
    const second = deferred<number>();
    const p1 = gate.run(() => first.promise, (v) => applied.push(v));
    const p2 = gate.run(() => second.promise, (v) => applied.push(v));
    first.resolve(0);
    await p1;
    second.resolve(1);
    await p2;
    assert.deepEqual(applied, [1]);
  });

  await t.test("单个请求照常写入；失败向调用方抛出", async () => {
    const gate = createLatestResultGate();
    const applied: number[] = [];
    await gate.run(async () => 3, (v) => applied.push(v));
    assert.deepEqual(applied, [3]);
    await assert.rejects(gate.run(async () => {
      throw new Error("ipc failed");
    }, () => {}));
  });

  await t.test("DesktopWorkbench 的 refreshAgentActivityStatus 经序号守卫写入", () => {
    const src = read("../../components/desktop/DesktopWorkbench.tsx");
    const start = src.indexOf("const refreshAgentActivityStatus = useCallback(");
    assert.ok(start >= 0);
    const body = src.slice(start, src.indexOf("}, [", start));
    assert.equal(
      /activityStatusGate\.run\(/.test(body),
      true,
      "refreshAgentActivityStatus 必须经 activityStatusGate 写入，晚到的旧响应不得覆盖新值",
    );
  });
});

test("W5-8 R10 F3：DesktopAgentWorkbench 的异步结果写入前核对会话", () => {
  const src = read("../../components/desktop/DesktopAgentWorkbench.tsx");
  const between = (startMarker: string, endMarker: string) => {
    const start = src.indexOf(startMarker);
    assert.ok(start >= 0, `找不到 ${startMarker}`);
    return src.slice(start, src.indexOf(endMarker, start + startMarker.length));
  };
  const refresh = between("const refreshSessions = useCallback(", "useEffect(() => {");
  assert.equal(refresh.includes("sessionGuard.noNewChatSince("), true, "refreshSessions 在新对话后不得再选中旧会话");
  const history = between("const loadSessionHistory = useCallback(", "useEffect(() => {");
  assert.equal(history.includes("sessionGuard.stillCurrent("), true, "loadSessionHistory 写入前必须核对会话");
  const poll = between("// 轮询逻辑：活动 Run 期间每 1 秒 streamPoll 一次", "return () => clearInterval(interval);");
  assert.equal(poll.includes("sessionGuard.stillCurrent("), true, "轮询结果写入前必须核对会话");
  const newChat = between("onNewChat: () => {", "},");
  assert.equal(newChat.includes("cursorRef.current = 0"), true, "新对话必须重置事件游标");
});
