import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  clearApprovedOutputDirs,
  recallApprovedOutputDir,
  rememberApprovedOutputDir,
} from "./approvedOutputDirMemory.ts";
import { planStepStatusLabelZh } from "./agentConversationLogic.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-8 R9 P2-1：批准时使用的下载目录按 plan_id 记在进程内存", async (t) => {
  t.afterEach(() => clearApprovedOutputDirs());

  await t.test("记下后可取回，别的计划不受影响", () => {
    rememberApprovedOutputDir("plan-a", "D:\\ScenePort\\downloads");
    assert.equal(recallApprovedOutputDir("plan-a"), "D:\\ScenePort\\downloads");
    assert.equal(recallApprovedOutputDir("plan-b"), null);
  });

  await t.test("空目录不记录", () => {
    rememberApprovedOutputDir("plan-a", "   ");
    rememberApprovedOutputDir("plan-b", null);
    assert.equal(recallApprovedOutputDir("plan-a"), null);
    assert.equal(recallApprovedOutputDir("plan-b"), null);
  });

  await t.test("DesktopAgentWorkbench 批准成功后记录目录，并把记录交给确认卡", () => {
    const src = fs.readFileSync(
      path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
      "utf-8",
    );
    const start = src.indexOf("const handleApprovePlan = async (");
    assert.ok(start >= 0);
    const body = src.slice(start, src.indexOf("// 确认卡：拒绝", start));
    const approveIdx = body.indexOf("await bridge.agent.planApprove(");
    const rememberIdx = body.indexOf("rememberApprovedOutputDir(planId, outputDir)");
    assert.equal(rememberIdx > approveIdx && approveIdx >= 0, true, "批准请求成功后才记录目录");
    assert.equal(
      src.includes("approvedOutputDir={recallApprovedOutputDir(item.plan_id)}"),
      true,
      "确认卡分支必须传入 approvedOutputDir",
    );
  });
});

test("W5-8 R9 P2-2：计划步骤 done 的中文是「已完成」", () => {
  assert.equal(planStepStatusLabelZh("done"), "已完成");
});
