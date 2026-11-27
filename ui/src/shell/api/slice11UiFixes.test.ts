import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { explainErrorCode, formatErrorWithCode } from "./errorExplanations.ts";
import { formatChannelTestResult } from "./agentChannelsLogic.ts";
import { mapStoreTaskSummaryToApplicationTask } from "./production.ts";
import { taskCardSizeLine } from "./taskCardLogic.ts";
import { formatBytes } from "./rowProjection.ts";
import type { StoreTaskSummary, ApplicationTask } from "./types.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W4 Slice 11 - UI Fixes Test Suite (Error Codes, Channel Copy, Run ID, Task Totals, Storage Z-Index)", async (t) => {
  // --------------------------------------------------------------------------
  // 1. 错误码映射表测试：逐字匹配 12 个映射，未知码返回 null，formatErrorWithCode 返回原始码
  // --------------------------------------------------------------------------
  await t.test("1. error code explanation mappings and fallback", () => {
    const expectedMappings: Record<string, string> = {
      model_transient: "模型服务暂时不可达",
      model_timeout: "模型服务响应超时",
      model_authentication: "访问密钥无效或未授权",
      model_parameter: "模型请求参数不被接受",
      model_capability: "模型服务拒绝了该请求",
      model_unknown: "模型服务返回未知错误",
      model_cancelled: "已取消",
      AGENT_CHANNEL_KEY_MISSING: "未配置访问密钥",
      AGENT_MODEL_UNAVAILABLE: "模型服务不可达或响应超时",
      CREDENTIAL_STORE_UNAVAILABLE: "系统凭据存储不可用",
      AGENT_CHANNEL_INVALID: "渠道配置参数不合法",
      BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE: "该区域请使用中国行政区选择器",
    };

    for (const [code, expected] of Object.entries(expectedMappings)) {
      assert.equal(explainErrorCode(code), expected, `Code ${code} must map to '${expected}'`);
      assert.equal(formatErrorWithCode(code), expected, `formatErrorWithCode(${code}) must return '${expected}'`);
    }

    // 未知码断言：返回 null，formatErrorWithCode 原样返回
    assert.equal(explainErrorCode("SOME_FUTURE_UNKNOWN_CODE"), null);
    assert.equal(formatErrorWithCode("SOME_FUTURE_UNKNOWN_CODE"), "SOME_FUTURE_UNKNOWN_CODE");
    assert.equal(explainErrorCode(""), null);
  });

  // --------------------------------------------------------------------------
  // 2. formatChannelTestResult 对接与格式兼容性
  // --------------------------------------------------------------------------
  await t.test("2. formatChannelTestResult incorporates model_* codes and maintains HTTP status format", () => {
    // model_transient
    const resultTransient = formatChannelTestResult({
      ok: false,
      code: "model_transient",
      status: null,
      model_count: null,
    });
    assert.equal(resultTransient, "网络连接失败");
    assert.equal(formatChannelTestResult({ ok: false, code: "model_timeout", status: null, model_count: null }), "连接超时");
    assert.equal(formatChannelTestResult({ ok: false, code: "model_authentication", status: 401, model_count: null }), "认证失败 (HTTP 401)");
    assert.equal(formatChannelTestResult({ ok: false, code: "model_parameter", status: 404, model_count: null }), "接口地址无效 (HTTP 404)");

    // 带 status (HTTP 502)
    const resultTransientWithStatus = formatChannelTestResult({
      ok: false,
      code: "model_transient",
      status: 502,
      model_count: null,
    });
    assert.equal(resultTransientWithStatus, "模型服务暂时不可达 (HTTP 502)");

    // 未知码原样保留且带 HTTP status
    const resultUnknown = formatChannelTestResult({
      ok: false,
      code: "CUSTOM_BACKEND_ERR",
      status: 400,
      model_count: null,
    });
    assert.equal(resultUnknown, "CUSTOM_BACKEND_ERR (HTTP 400)");
  });

  // --------------------------------------------------------------------------
  // 3. mapStoreTaskSummaryToApplicationTask 透传 source_total_bytes
  // --------------------------------------------------------------------------
  await t.test("3. mapStoreTaskSummaryToApplicationTask passes source_total_bytes transparently", () => {
    const baseSummary: StoreTaskSummary = {
      task_id: "task-001",
      batch_id: "batch-001",
      provider_id: "asf",
      dataset_id: "sentinel-1",
      concurrency: 2,
      task_type: null,
      created_at: "2026-10-25T00:00:00Z",
      status: "cancelled",
      revision: 1,
      updated_at: "2026-10-25T00:01:00Z",
      selected_count: 5,
      delivery_requested_count: null,
      delivery_rejected_count: null,
      checked_count: 5,
      active_count: 0,
      paused_count: 0,
      queued_count: 0,
      completed_count: 2,
      failed_count: 0,
      skipped_count: 0,
      cancelled_count: 3,
      terminal_bytes: 500,
      active_bytes: 0,
      other_bytes: 0,
      transferred_bytes: 500,
      total_bytes: null,
      source_total_bytes: 104857600, // 100 MB 冻结总量
    };

    const taskWithSourceTotal = mapStoreTaskSummaryToApplicationTask(baseSummary);
    assert.equal(taskWithSourceTotal.source_total_bytes, 104857600, "source_total_bytes must be passed through");

    const summaryWithoutSourceTotal: StoreTaskSummary = {
      ...baseSummary,
      source_total_bytes: null,
    };
    const taskWithoutSourceTotal = mapStoreTaskSummaryToApplicationTask(summaryWithoutSourceTotal);
    assert.equal(taskWithoutSourceTotal.source_total_bytes, null, "source_total_bytes must be null when backend sends null");
  });

  // --------------------------------------------------------------------------
  // 4. TaskCard 尺寸行纯函数 taskCardSizeLine
  // --------------------------------------------------------------------------
  await t.test("4. taskCardSizeLine logic for cancelled/interrupted tasks with frozen total fallback", () => {
    // A: cancelled + total_bytes=null + source_total_bytes=1234567 → 含「冻结总量」与格式化值
    const cancelledTask: ApplicationTask = {
      task_id: "task-cancelled",
      revision: 1,
      category: "scene_download",
      name: "测试下载任务",
      status: "cancelled",
      done_bytes: 2048,
      total_bytes: null,
      source_total_bytes: 1234567,
    };
    const sizeLineCancelled = taskCardSizeLine(cancelledTask, formatBytes);
    assert.match(sizeLineCancelled, /冻结总量/);
    assert.match(sizeLineCancelled, /已传输/);
    assert.match(sizeLineCancelled, /1\.2 MB/);

    // B: total_bytes 已知 → 现状 done/total
    const knownTotalTask: ApplicationTask = {
      task_id: "task-known",
      revision: 1,
      category: "scene_download",
      name: "测试下载任务",
      status: "cancelled",
      done_bytes: 1048576,
      total_bytes: 5242880,
      source_total_bytes: 5242880,
    };
    const sizeLineKnown = taskCardSizeLine(knownTotalTask, formatBytes);
    assert.equal(sizeLineKnown, "1.0 MB / 5.0 MB");
    assert.equal(sizeLineKnown.includes("冻结总量"), false);

    // C: 都未知 → 大小未知
    const allUnknownTask: ApplicationTask = {
      task_id: "task-unknown",
      revision: 1,
      category: "scene_download",
      name: "测试下载任务",
      status: "cancelled",
      done_bytes: 0,
      total_bytes: null,
      source_total_bytes: null,
    };
    const sizeLineAllUnknown = taskCardSizeLine(allUnknownTask, formatBytes);
    assert.equal(sizeLineAllUnknown, "大小未知");

    // D: running + total_bytes=null → 不使用冻结总量
    const runningTask: ApplicationTask = {
      task_id: "task-running",
      revision: 1,
      category: "scene_download",
      name: "正在运行任务",
      status: "running",
      done_bytes: 0,
      total_bytes: null,
      source_total_bytes: 1234567,
    };
    const sizeLineRunning = taskCardSizeLine(runningTask, formatBytes);
    assert.equal(sizeLineRunning.includes("冻结总量"), false, "running task must NOT use source_total_bytes fallback");
    assert.equal(sizeLineRunning, "大小未知");
  });

  // --------------------------------------------------------------------------
  // 5. 源码级断言（z-[2500], 文案清理, title 悬停）
  // --------------------------------------------------------------------------
  await t.test("5. source-level assertions for DesktopWorkbench, ModelChannelsSettingsTab, and DesktopAgentWorkbench", () => {
    // 5.1 DesktopWorkbench.tsx 存储提示遮罩含 z-[2500]
    const workbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
    const workbenchContent = fs.readFileSync(workbenchPath, "utf-8");
    assert.match(
      workbenchContent,
      /storageRequiredPrompt[\s\S]*?z-\[2500\]/,
      "DesktopWorkbench.tsx storageRequiredPrompt modal overlay must use z-[2500]"
    );

    // 5.2 ModelChannelsSettingsTab.tsx 不含「已配密钥」与「待配密钥」字样
    const channelsTabPath = path.resolve(__dirname, "../components/settings/ModelChannelsSettingsTab.tsx");
    const channelsTabContent = fs.readFileSync(channelsTabPath, "utf-8");
    assert.equal(channelsTabContent.includes("已配密钥"), false, "ModelChannelsSettingsTab.tsx must not contain '已配密钥'");
    assert.equal(channelsTabContent.includes("待配密钥"), false, "ModelChannelsSettingsTab.tsx must not contain '待配密钥'");

    // 5.3 DesktopAgentWorkbench.tsx 不再包含左侧运行记录侧栏（移至主导航 #/history 历史页 W5-2d），且不含作弊占位元素
    const agentWorkbenchPath = path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx");
    const agentWorkbenchContent = fs.readFileSync(agentWorkbenchPath, "utf-8");
    assert.doesNotMatch(
      agentWorkbenchContent,
      /运行记录\s*<\/span>/,
      "DesktopAgentWorkbench.tsx must no longer render run-record sidebar"
    );
    assert.doesNotMatch(
      agentWorkbenchContent,
      /false && runs\.map/,
      "DesktopAgentWorkbench.tsx must not contain dead element placeholder"
    );
  });
});
