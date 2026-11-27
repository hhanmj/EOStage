import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { errorCardHintZh } from "./approvalLogic.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

function read(rel: string): string {
  return fs.readFileSync(path.resolve(here, rel), "utf-8");
}

const HINT = "该渠道无法关闭思考，输出可能被思考占满；可改用支持关闭思考的模型";

test("W5-9 条目 6：输出被截断且渠道拒绝关闭思考时，错误卡给出说明", async (t) => {
  await t.test("model_truncated + reasoning_off_refused → 说明", () => {
    assert.equal(errorCardHintZh("model_truncated", true), HINT);
  });
  await t.test("只有截断、或只有拒绝关闭思考 → 没有说明", () => {
    assert.equal(errorCardHintZh("model_truncated", false), null);
    assert.equal(errorCardHintZh("record_failed", true), null);
  });
  await t.test("说明是中文，不含英文", () => {
    assert.equal(/[A-Za-z]/.test(HINT), false);
  });
});

// 「未核验」原因的规则已由 W5-10 条目 1 取代（evidenceSummary，见 w510R1.test.ts）

test("W5-9 R2 接线：错误卡拿到该运行的 reasoning_off_refused", () => {
  const src = read("../../components/desktop/DesktopAgentWorkbench.tsx");
  const start = src.indexOf('case "error": {');
  assert.ok(start >= 0);
  const branch = src.slice(start, src.indexOf('case "stop_all"', start));
  assert.equal(/<AgentErrorCard[^>]*reasoningOffRefused=/.test(branch), true, "错误卡分支必须传入 reasoningOffRefused");
  const card = read("../../components/desktop/AgentErrorCard.tsx");
  assert.equal(card.includes("errorCardHintZh("), true, "错误卡必须用 errorCardHintZh 决定说明");
  const conclusionCard = read("../../components/desktop/AgentConclusionCard.tsx");
  assert.equal(conclusionCard.includes("evidenceSummary("), true, "结论卡依据摘要必须由 evidenceSummary 推导");
});
