import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { mergeRowStates } from "./rowStateSync.ts";
import {
  withCreatePending,
  withoutCreatePending,
  createTaskButtonLabel,
} from "./creatingFeedback.ts";
import { humanTaskLabel } from "./taskCardNaming.ts";
import { UnifiedClient } from "./unifiedClient.ts";
import type { RowTaskState } from "./types.ts";
import {
  deriveWrappedAoiEnvelope,
  continuousDisplayBbox,
} from "../../lib/antimeridian.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");

test("W4 Slice 10 Subtask 3: Shell Fixes Suite", async (t) => {
  // -------------------------------------------------------------
  // D5-1: mergeRowStates 纯函数（E10 场景：取消任务后行状态同步）
  // -------------------------------------------------------------
  await t.test(
    "D5-1: mergeRowStates updates modified rows and signals change accurately",
    () => {
      const initialMap = new Map<string, RowTaskState>();
      initialMap.set("item-1", {
        item_id: "item-1",
        task_id: "task-1",
        task_status: "running",
        item_status: "running",
        actions: ["pause", "cancel"],
      });
      initialMap.set("item-2", {
        item_id: "item-2",
        task_id: "task-1",
        task_status: "running",
        item_status: "running",
        actions: ["pause", "cancel"],
      });

      // 1. 无变更时的调用
      const unchangedRows: RowTaskState[] = [
        {
          item_id: "item-1",
          task_id: "task-1",
          task_status: "running",
          item_status: "running",
          actions: ["pause", "cancel"],
        },
        {
          item_id: "item-2",
          task_id: "task-1",
          task_status: "running",
          item_status: "running",
          actions: ["pause", "cancel"],
        },
      ];
      const resUnchanged = mergeRowStates(initialMap, unchangedRows);
      assert.strictEqual(
        resUnchanged.changed,
        false,
        "Should report changed=false when states are identical"
      );

      // 2. 取消任务后后端返回的新状态 (item-1 被 cancelled)
      const cancelledRows: RowTaskState[] = [
        {
          item_id: "item-1",
          task_id: "task-1",
          task_status: "cancelled",
          item_status: "cancelled",
          actions: ["open_task", "retry"],
        },
        {
          item_id: "item-2",
          task_id: "task-1",
          task_status: "running",
          item_status: "running",
          actions: ["pause", "cancel"],
        },
      ];
      const resCancelled = mergeRowStates(initialMap, cancelledRows);
      assert.strictEqual(
        resCancelled.changed,
        true,
        "Should report changed=true when an item state changes"
      );
      const updatedItem1 = resCancelled.next.get("item-1");
      assert.ok(updatedItem1);
      assert.strictEqual(updatedItem1.task_status, "cancelled");
      assert.strictEqual(updatedItem1.item_status, "cancelled");
      assert.deepStrictEqual(updatedItem1.actions, ["open_task", "retry"]);
      assert.strictEqual(resCancelled.next.get("item-2")?.task_status, "running");
    }
  );

  // -------------------------------------------------------------
  // D5-2: 源码断言检查 DesktopWorkbench 中 onPollTasks 路径
  // -------------------------------------------------------------
  await t.test(
    "D5-2: DesktopWorkbench onPollTasks path integrates mergeRowStates with in-flight guard",
    () => {
      const workbenchFile = path.resolve(
        uiSrcDir,
        "components/desktop/DesktopWorkbench.tsx"
      );
      const code = fs.readFileSync(workbenchFile, "utf-8");

      assert.match(
        code,
        /mergeRowStates/,
        "DesktopWorkbench must invoke mergeRowStates"
      );
      assert.match(
        code,
        /batchPageInFlight|inFlight/i,
        "DesktopWorkbench must have an in-flight guard for polling batch_page"
      );
    }
  );

  // -------------------------------------------------------------
  // D6-1: creatingFeedback 纯函数与 dock 按钮状态
  // -------------------------------------------------------------
  await t.test(
    "D6-1: creatingFeedback manages in-flight task creation labels and pending sets",
    () => {
      const s0 = new Set<string>();
      const s1 = withCreatePending(s0, "scene-101");
      assert.ok(s1.has("scene-101"), "withCreatePending adds itemId to pending set");
      assert.ok(!s0.has("scene-101"), "withCreatePending is immutable");

      const s2 = withoutCreatePending(s1, "scene-101");
      assert.ok(!s2.has("scene-101"), "withoutCreatePending removes itemId");

      assert.strictEqual(
        createTaskButtonLabel(true, false),
        "正在确认账号…",
        "Single row button displays '正在确认账号…' when pending"
      );
      assert.strictEqual(
        createTaskButtonLabel(false, false),
        "下载",
        "Single row button displays '下载' when idle"
      );
      assert.strictEqual(
        createTaskButtonLabel(true, true),
        "正在确认账号…",
        "Batch button displays '正在确认账号…' when pending"
      );
      assert.strictEqual(
        createTaskButtonLabel(false, true),
        "加入下载队列",
        "Batch button displays '加入下载队列' when idle"
      );

      const dockFile = path.resolve(
        uiSrcDir,
        "components/desktop/ManualWorkbench.tsx"
      );
      const dockCode = fs.readFileSync(dockFile, "utf-8");
      assert.match(
        dockCode,
        /creatingItemIds/,
        "ManualWorkbench must accept creatingItemIds prop"
      );
      assert.match(
        dockCode,
        /batchCreating/,
        "ManualWorkbench must accept batchCreating prop"
      );
      // The localized dock keeps the in-flight account-confirmation label.
      assertLocalizedSource(dockCode, "mapWorkbench.results.confirmingAccount", "正在确认账号…");
    }
  );

  // -------------------------------------------------------------
  // D-extra-1: humanTaskLabel 任务卡命名
  // -------------------------------------------------------------
  await t.test(
    "D-extra-1: humanTaskLabel maps raw provider/dataset IDs to human-readable titles without technical terms",
    () => {
      const s1 = humanTaskLabel({
        provider_id: "asf.catalog",
        dataset_id: "SENTINEL-1",
        selected_count: 1,
      });
      assert.strictEqual(
        s1.title,
        "Sentinel-1",
        "Title for single Sentinel-1 product should be human-readable"
      );
      assert.strictEqual(s1.subtitle, "ASF", "Subtitle should be provider human label");

      const s3 = humanTaskLabel({
        provider_id: "asf.catalog",
        dataset_id: "SENTINEL-1",
        selected_count: 3,
      });
      assert.match(
        s3.title,
        /Sentinel-1.*3.*个产品/,
        "Batch download title includes product count"
      );

      // 绝不输出 task-unified 或 asf.catalog 原始实现名
      assert.ok(!s1.title.includes("task-unified"));
      assert.ok(!s1.subtitle.includes("asf.catalog"));
    }
  );

  // -------------------------------------------------------------
  // D-extra-2: 下载抽屉全卡字节口径一致性检查
  // -------------------------------------------------------------
  await t.test(
    "D-extra-2: DesktopDownloadDrawer uses formatBytes consistently and avoids raw 1024 math",
    () => {
      const drawerFile = path.resolve(
        uiSrcDir,
        "components/desktop/DesktopDownloadDrawer.tsx"
      );
      const drawerCode = fs.readFileSync(drawerFile, "utf-8");

      assert.doesNotMatch(
        drawerCode,
        /1024\s*\/\s*1024/,
        "DesktopDownloadDrawer must not contain raw / 1024 / 1024 math"
      );
      assert.match(
        drawerCode,
        /formatBytes/,
        "DesktopDownloadDrawer must format bytes using formatBytes"
      );
    }
  );

  // -------------------------------------------------------------
  // D-extra-3: 账号设置 OpenTopography 真实状态与 GACOS 邮箱卡
  // -------------------------------------------------------------
  await t.test(
    "D-extra-3: AccountsSettingsTab displays real OpenTopography status and GACOS notification email card",
    () => {
      const settingsFile = path.resolve(
        uiSrcDir,
        "shell/components/settings/AccountsSettingsTab.tsx"
      );
      const settingsCode = fs.readFileSync(settingsFile, "utf-8");

      // 不得将 opentopography 硬编码为 not_required
      assert.doesNotMatch(
        settingsCode,
        /accountId === "opentopography" \? "not_required"/,
        "Must not hardcode opentopography to not_required"
      );

      // GACOS 条件渲染：基于 AccountId 类型系统守护
      assert.match(
        settingsCode,
        /type\s+GacosInContract\s*=\s*"gacos"\s+extends\s+AccountId\s*\?\s*true\s*:\s*false/,
        "Must condition GACOS rendering on AccountId contract type check"
      );

      // GACOS 文本与功能约束
      assertLocalizedSource(settingsCode, "settings.accounts.notificationEmail", "通知邮箱");
      // Compact account cards omit helper copy.
      assert.doesNotMatch(
        settingsCode,
        />[^<]*统一管理数据源会话/,
        "Account cards must not retain a long explanatory page subtitle"
      );
      assert.doesNotMatch(
        settingsCode,
        /GACOS\s*账户/,
        "GACOS card must not refer to 'GACOS 账户'"
      );
      assert.doesNotMatch(
        settingsCode,
        /检测.*GACOS|GACOS.*检测/,
        "GACOS card must not provide a test/check button"
      );

      // OpenTopography API Key 配置操作
      assert.match(
        settingsCode,
        /saveOpentopographyKey/,
        "OpenTopography panel must connect saveOpentopographyKey"
      );
      assert.match(
        settingsCode,
        /clearOpentopographyKey/,
        "OpenTopography panel must connect clearOpentopographyKey"
      );
    }
  );

  // -------------------------------------------------------------
  // D-extra-4: unifiedClient 启动与批次分页并发 Promise 去重
  // -------------------------------------------------------------
  await t.test(
    "D-extra-4: unifiedClient deduplicates concurrent in-flight calls to listSources, sourceViews, and batchPage",
    async () => {
      let batchPageInvokeCount = 0;
      let listSourcesInvokeCount = 0;
      let sourceViewsInvokeCount = 0;

      const mockInvoke = async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
        if (cmd === "application_batch_page") {
          batchPageInvokeCount++;
          // 异步等待，模拟在途并发
          await new Promise((resolve) => setTimeout(resolve, 20));
          return {
            batch_id: args?.batchId,
            revision: args?.revision,
            total_count: 10,
            page: args?.page,
            page_size: args?.pageSize,
            scenes: [],
            rows: [],
          };
        }
        if (cmd === "application_list_sources") {
          listSourcesInvokeCount++;
          await new Promise((resolve) => setTimeout(resolve, 20));
          return [];
        }
        if (cmd === "application_source_views") {
          sourceViewsInvokeCount++;
          await new Promise((resolve) => setTimeout(resolve, 20));
          return [];
        }
        return null;
      };

      const client = new UnifiedClient({ invoke: mockInvoke });

      // 1. 同键并发 batchPage 只打一次后端
      const [res1, res2] = await Promise.all([
        client.batchPage("batch-1", 1, 0, 50),
        client.batchPage("batch-1", 1, 0, 50),
      ]);
      assert.strictEqual(batchPageInvokeCount, 1, "Concurrent batchPage with identical keys must only invoke backend once");
      assert.strictEqual(res1, res2, "Concurrent callers receive identical batchPage result");

      // 2. 不同键各打一次后端
      await client.batchPage("batch-2", 1, 0, 50);
      assert.strictEqual(batchPageInvokeCount, 2, "batchPage with different batchId must invoke backend again");

      await client.batchPage("batch-1", 2, 0, 50);
      assert.strictEqual(batchPageInvokeCount, 3, "batchPage with different revision must invoke backend again");

      await client.batchPage("batch-1", 1, 1, 50);
      assert.strictEqual(batchPageInvokeCount, 4, "batchPage with different page must invoke backend again");

      await client.batchPage("batch-1", 1, 0, 100);
      assert.strictEqual(batchPageInvokeCount, 5, "batchPage with different pageSize must invoke backend again");

      // 3. listSources 与 sourceViews 同理并发去重
      await Promise.all([
        client.listSources(),
        client.listSources(),
      ]);
      assert.strictEqual(listSourcesInvokeCount, 1, "Concurrent listSources must only invoke backend once");

      await Promise.all([
        client.sourceViews(),
        client.sourceViews(),
      ]);
      assert.strictEqual(sourceViewsInvokeCount, 1, "Concurrent sourceViews must only invoke backend once");
    }
  );

  // -------------------------------------------------------------
  // D-extra-5: C6 1421 顶点都江堰市 AOI 包络与地图中心读数更新
  // -------------------------------------------------------------
  await t.test(
    "D-extra-5: 1421-vertex polygon calculates valid envelope and map fits to AOI center",
    () => {
      // 模拟都江堰市 1421 顶点多边形
      const w = 103.428598;
      const s = 30.7488665;
      const e = 103.7841653;
      const n = 31.3689038;
      const points: [number, number][] = [];
      const N = 1420;
      for (let i = 0; i < N; i++) {
        const angle = (i / N) * 2 * Math.PI;
        const lng = (w + e) / 2 + ((e - w) / 2) * Math.cos(angle);
        const lat = (s + n) / 2 + ((n - s) / 2) * Math.sin(angle);
        points.push([lng, lat]);
      }
      points.push(points[0]!); // 闭合
      const geom = { type: "Polygon", coordinates: [points] };

      const env = deriveWrappedAoiEnvelope(geom);
      assert.ok(env, "deriveWrappedAoiEnvelope must succeed for 1421-vertex polygon");
      const bbox = continuousDisplayBbox(env);
      assert.ok(bbox, "continuousDisplayBbox must return valid display bbox");

      // 验证中心计算在都江堰市范围内，绝非旧手绘矩形视野 (104.0497, 30.7737)
      const centerLng = (bbox.west + bbox.east) / 2;
      const centerLat = (bbox.south + bbox.north) / 2;
      assert.ok(
        Math.abs(centerLng - 103.6064) < 0.05,
        `Center longitude ${centerLng} must be near Dujiangyan center 103.6064`
      );
      assert.ok(
        Math.abs(centerLat - 31.0589) < 0.05,
        `Center latitude ${centerLat} must be near Dujiangyan center 31.0589`
      );

      // 源码检查：WorkbenchMap 中的 MapStatusOverlay 与 FitToData
      const mapFile = path.resolve(uiSrcDir, "components/WorkbenchMap.tsx");
      const mapCode = fs.readFileSync(mapFile, "utf-8");
      assert.match(
        mapCode,
        /viewreset/,
        "MapStatusOverlay must listen to viewreset event to catch fitBounds updates"
      );
    }
  );

  // -------------------------------------------------------------
  // D-extra-6: C7 发送上下文补真值
  // -------------------------------------------------------------
  await t.test(
    "D-extra-6: DesktopWorkbench passes genuine AOI label, time range and source ID to DesktopAgentWorkbench",
    () => {
      const wbFile = path.resolve(
        uiSrcDir,
        "components/desktop/DesktopWorkbench.tsx"
      );
      const wbCode = fs.readFileSync(wbFile, "utf-8");
      assert.match(
        wbCode,
        /aoiLabel=\{aoiLabel\}/,
        "DesktopWorkbench must pass aoiLabel to DesktopAgentWorkbench"
      );
      assert.match(
        wbCode,
        /timeRange=\{/,
        "DesktopWorkbench must pass timeRange to DesktopAgentWorkbench"
      );
      assert.match(
        wbCode,
        /sourceId=\{/,
        "DesktopWorkbench must pass sourceId to DesktopAgentWorkbench"
      );

      const agentWbFile = path.resolve(
        uiSrcDir,
        "components/desktop/DesktopAgentWorkbench.tsx"
      );
      const agentWbCode = fs.readFileSync(agentWbFile, "utf-8");
      assert.doesNotMatch(
        agentWbCode,
        /label:\s*"主工作区范围"/,
        "DesktopAgentWorkbench must not hardcode '主工作区范围'"
      );
    }
  );
});
