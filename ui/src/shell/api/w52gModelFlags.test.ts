import assert from "node:assert/strict";
import test from "node:test";
import { classifyStreamEvents } from "./agentStreamLogic.ts";
import type { AgentStreamEvent } from "../../lib/generated/v3-contracts.ts";
import { explainErrorCode } from "./errorExplanations.ts";
import { projectErrorCard } from "./approvalLogic.ts";
import { projectToolGroup } from "./agentConversationLogic.ts";
import type { StreamToolGroupItem } from "./agentStreamLogic.ts";

test("W5-2g R6 (2g-9): 模型标志、重发与截断错误码规范", async (suite) => {
  await suite.test(
    "1. classifyStreamEvents: 结论 reasoning_off_refused true / 缺省 / 非布尔三例",
    () => {
      // 1) true
      const evTrue: AgentStreamEvent[] = [
        {
          seq: 1,
          kind: "conclusion",
          payload: {
            run_id: "run-true",
            text: "完成",
            claims: [],
            reasoning_off_refused: true,
          },
        },
      ];
      const itemsTrue = classifyStreamEvents(evTrue);
      assert.strictEqual(itemsTrue.length, 1);
      assert.strictEqual(itemsTrue[0].kind, "conclusion");
      if (itemsTrue[0].kind === "conclusion") {
        assert.strictEqual(itemsTrue[0].reasoning_off_refused, true);
      }

      // 2) 缺省 (undefined)
      const evDef: AgentStreamEvent[] = [
        {
          seq: 2,
          kind: "conclusion",
          payload: {
            run_id: "run-def",
            text: "完成",
            claims: [],
          },
        },
      ];
      const itemsDef = classifyStreamEvents(evDef);
      assert.strictEqual(itemsDef.length, 1);
      assert.strictEqual(itemsDef[0].kind, "conclusion");
      if (itemsDef[0].kind === "conclusion") {
        assert.strictEqual(itemsDef[0].reasoning_off_refused, false);
      }

      // 3) 非布尔 ("true" 字符串 / 数字)
      const evNonBool: AgentStreamEvent[] = [
        {
          seq: 3,
          kind: "conclusion",
          payload: {
            run_id: "run-str",
            text: "完成",
            claims: [],
            reasoning_off_refused: "true" as any,
          },
        },
      ];
      const itemsNonBool = classifyStreamEvents(evNonBool);
      assert.strictEqual(itemsNonBool.length, 1);
      assert.strictEqual(itemsNonBool[0].kind, "conclusion");
      if (itemsNonBool[0].kind === "conclusion") {
        assert.strictEqual(itemsNonBool[0].reasoning_off_refused, false);
      }
    }
  );

  await suite.test("2. explainErrorCode: model_truncated 逐字中文解释", () => {
    const expected =
      "模型回复超出长度上限，本次运行未生成结论；请缩小范围或换渠道";
    assert.strictEqual(explainErrorCode("model_truncated"), expected);
  });

  await suite.test(
    "3. projectErrorCard: model_truncated 可见文字、detailCode 与 snake_case 防护",
    () => {
      const expected =
        "模型回复超出长度上限，本次运行未生成结论；请缩小范围或换渠道";
      const proj = projectErrorCard({
        code: "model_truncated",
        status: "failed",
      });
      assert.strictEqual(proj.wording, expected);
      assert.strictEqual(proj.detailCode, "model_truncated");
      assert.strictEqual(proj.modeLine, null);
      // 可见文字绝不包含 snake_case 原文
      const snakePattern = /[a-z]+_[a-z_]+/;
      assert.strictEqual(snakePattern.test(proj.wording), false);
    }
  );

  await suite.test(
    "4. projectToolGroup: 带/不带 reasoningResent 两例（末行文字、无时间耗时、计数与行数一致）",
    () => {
      const mockItem: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 1,
        run_id: "run-tg",
        status: "done",
        rounds: [
          {
            tool_id: "resolve_aoi",
            status: "done",
            started_at: "2026-10-26T10:00:00Z",
            duration_ms: 1200,
            result_count: 1,
            summary_zh: "已解析 AOI",
          },
        ],
      };

      // 4a. 不带 options (默认)
      const projDefault = projectToolGroup(mockItem);
      assert.strictEqual(projDefault.count, 1);
      assert.strictEqual(projDefault.rows.length, 1);
      assert.strictEqual(projDefault.rows[0].name_zh, "resolve_aoi");
      assert.notStrictEqual(
        projDefault.rows[0].name_zh,
        "已按渠道要求重发（关闭思考被拒绝）"
      );

      // 4b. 带 { reasoningResent: true }
      const projResent = projectToolGroup(mockItem, undefined, {
        reasoningResent: true,
      });
      assert.strictEqual(projResent.rows.length, 2);
      assert.strictEqual(projResent.count, projResent.rows.length);

      const resentRow = projResent.rows[projResent.rows.length - 1];
      assert.strictEqual(
        resentRow.name_zh,
        "已按渠道要求重发（关闭思考被拒绝）"
      );
      assert.strictEqual(resentRow.status, "done");
      assert.strictEqual(resentRow.timeZh, null);
      assert.strictEqual(resentRow.durationZh, null);
      assert.strictEqual(resentRow.resultCountZh, null);
      assert.strictEqual(resentRow.summaryZh, null);
    }
  );
});
