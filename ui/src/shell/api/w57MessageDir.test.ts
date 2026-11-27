import test from "node:test";
import assert from "node:assert/strict";

import { localMarkersToZh } from "./modelTextMarkers.ts";
import {
  classifyStreamEvents,
  type StreamMessageItem,
  type StreamConclusionItem,
} from "./agentStreamLogic.ts";
import type { AgentStreamEvent } from "../../lib/generated/v3-contracts.ts";

test("W5-7 R3 条目 5 消息目录占位符替换与事件分类测试套件", async (suite) => {
  await suite.test("1. localMarkersToZh（W5-10 R3 起唯一的标记替换）：边界与空格规则", () => {
    // 1) 非字符串输入返回空字符串 ""
    assert.strictEqual(localMarkersToZh(null), "");
    assert.strictEqual(localMarkersToZh(undefined), "");
    assert.strictEqual(localMarkersToZh(12345), "");

    // 2) 无占位符原样返回
    const normalText = "普通文本，不含任何目录占位符。";
    assert.strictEqual(localMarkersToZh(normalText), normalText);

    // 3) 标记两侧原有空格保留，不额外增删
    assert.strictEqual(
      localMarkersToZh("数据将下载到 [local-reference:message_dir]，请确认。"),
      "数据将下载到 （消息中指定的目录），请确认。"
    );

    // 4) 其他 local-reference 种类按本机内容显示（W5-10 条目 3）
    assert.strictEqual(
      localMarkersToZh("未知标记 [local-reference:unknown] 及 [local-reference:message_dir]"),
      "未知标记 （本机内容） 及 （消息中指定的目录）"
    );
  });

  await suite.test("2. classifyStreamEvents: 助手消息含两处字面量时完成替换，且零原始字面量残留", () => {
    const events: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "message",
        payload: {
          role: "assistant",
          run_id: "run-fixture-01",
          text: "数据将下载到 [local-reference:message_dir]，检索完成后请确认。下载目录已锁定为 [local-reference:message_dir]。",
          context: null,
        },
      },
    ];

    const items = classifyStreamEvents(events);
    assert.strictEqual(items.length, 1);
    assert.strictEqual(items[0].kind, "message");
    const msg = items[0] as StreamMessageItem;
    assert.strictEqual(
      msg.text,
      "数据将下载到 （消息中指定的目录），检索完成后请确认。下载目录已锁定为 （消息中指定的目录）。"
    );
    assert.ok(
      !msg.text.includes("[local-reference:message_dir]"),
      "助手消息文本不得残留 [local-reference:message_dir] 原始字面量"
    );
  });

  await suite.test("3. classifyStreamEvents: 用户消息中的占位符原样保留不转换", () => {
    const userRawText = "请把文件下载到 [local-reference:message_dir] 目录下。";
    const events: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "message",
        payload: {
          role: "user",
          run_id: "run-fixture-01",
          text: userRawText,
          context: null,
        },
      },
    ];

    const items = classifyStreamEvents(events);
    assert.strictEqual(items.length, 1);
    assert.strictEqual(items[0].kind, "message");
    const msg = items[0] as StreamMessageItem;
    assert.strictEqual(
      msg.text,
      userRawText,
      "用户消息不得做任何占位符替换"
    );
  });

  await suite.test("4. classifyStreamEvents: 结论正文中的占位符同样完成替换", () => {
    const events: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "conclusion",
        payload: {
          run_id: "run-fixture-01",
          text: "检索与创建已完成，数据将保存至 [local-reference:message_dir]。",
          claims: [],
          reasoning_off_refused: false,
        },
      },
    ];

    const items = classifyStreamEvents(events);
    assert.strictEqual(items.length, 1);
    assert.strictEqual(items[0].kind, "conclusion");
    const conc = items[0] as StreamConclusionItem;
    assert.strictEqual(
      conc.text,
      "检索与创建已完成，数据将保存至 （消息中指定的目录）。"
    );
    assert.ok(
      !conc.text.includes("[local-reference:message_dir]"),
      "结论文本不得残留原始字面量"
    );
  });

  await suite.test("5. classifyStreamEvents: 本机标记统一替换，其他方括号标记不受影响", () => {
    const events: AgentStreamEvent[] = [
      {
        seq: 1,
        kind: "message",
        payload: {
          role: "assistant",
          run_id: "run-fixture-01",
          text: "存在未知引用 [local-reference:unknown]，同时目标为 [local-reference:message_dir]。",
        },
      },
      {
        seq: 2,
        kind: "conclusion",
        payload: {
          run_id: "run-fixture-01",
          text: "结论中包含 [other:token] 以及 [local-reference:message_dir] 结果。",
          claims: [],
        },
      },
    ];

    const items = classifyStreamEvents(events);
    assert.strictEqual(items.length, 2);
    const msg = items[0] as StreamMessageItem;
    const conc = items[1] as StreamConclusionItem;

    assert.strictEqual(
      msg.text,
      "存在未知引用 （本机内容），同时目标为 （消息中指定的目录）。"
    );
    assert.strictEqual(
      conc.text,
      "结论中包含 [other:token] 以及 （消息中指定的目录） 结果。"
    );
  });
});
