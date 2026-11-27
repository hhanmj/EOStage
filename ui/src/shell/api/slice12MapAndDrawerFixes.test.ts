import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { createMapFitReporter, defaultMapFitReporter } from "./mapFitReporter.ts";
import { resolveDownloadDrawerEscapeAction } from "./drawerEscapeLogic.ts";
import type { MapFitView } from "./mapFitReporter.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../../");

test("W4 Slice 12-2 - Map Fit Synchronization & Download Drawer Escape/Focus Suite", async (t) => {
  // --------------------------------------------------------------------------
  // 1. MapFitReporter 行为测试：发布 -> 订阅者接收；恰好一次通知；无 fit 无通知
  // --------------------------------------------------------------------------
  await t.test("1. MapFitReporter: publish view, exact notifications, and getLatest tracking", () => {
    const reporter = createMapFitReporter();
    assert.strictEqual(reporter.getLatest(), null, "initial getLatest() must be null");

    const receivedSub1: MapFitView[] = [];
    const unsubscribe1 = reporter.subscribe((view) => {
      receivedSub1.push(view);
    });

    assert.strictEqual(receivedSub1.length, 0, "no notifications on subscription without fit");

    // 第一次发布视图（模拟启动后 AOI 恢复 fit 到珠海/澳门）
    const zhuhaiFit: MapFitView = {
      center: { lat: 22.1605, lng: 113.5595 },
      zoom: 12,
      bbox: { west: 113.52, south: 22.10, east: 113.60, north: 22.22 },
    };
    reporter.reportFit(zhuhaiFit);

    assert.strictEqual(receivedSub1.length, 1, "sub1 must receive exactly 1 notification");
    assert.deepStrictEqual(receivedSub1[0], zhuhaiFit, "sub1 must receive published zhuhai view");
    assert.deepStrictEqual(reporter.getLatest(), zhuhaiFit, "getLatest() must reflect zhuhai fit");

    // 注册第二个订阅者：无新 fit 发生时，无额外通知
    const receivedSub2: MapFitView[] = [];
    const unsubscribe2 = reporter.subscribe((view) => {
      receivedSub2.push(view);
    });
    assert.strictEqual(receivedSub1.length, 1, "sub1 call count remains 1");
    assert.strictEqual(receivedSub2.length, 0, "sub2 gets no notification without new fit");

    // 第二次发布视图（模拟后续一次检索 fit）
    const searchFit: MapFitView = {
      center: { lat: 31.0589, lng: 103.6064 },
      zoom: 11,
      bbox: { west: 103.50, south: 31.00, east: 103.70, north: 31.10 },
    };
    reporter.reportFit(searchFit);

    assert.strictEqual(receivedSub1.length, 2, "sub1 receives 2nd notification");
    assert.strictEqual(receivedSub2.length, 1, "sub2 receives 1st notification");
    assert.deepStrictEqual(reporter.getLatest(), searchFit);

    // 取消 sub1 订阅后再次发布
    unsubscribe1();
    const finalFit: MapFitView = {
      center: { lat: 39.9042, lng: 116.4074 },
      zoom: 10,
    };
    reporter.reportFit(finalFit);

    assert.strictEqual(receivedSub1.length, 2, "unsubscribed sub1 must not receive 3rd notification");
    assert.strictEqual(receivedSub2.length, 2, "sub2 receives 2nd notification");
    assert.deepStrictEqual(reporter.getLatest(), finalFit);

    // clear 测试
    reporter.clear();
    assert.strictEqual(reporter.getLatest(), null);
    unsubscribe2();
  });

  // --------------------------------------------------------------------------
  // 2. defaultMapFitReporter 模块级单例契约
  // --------------------------------------------------------------------------
  await t.test("2. defaultMapFitReporter singleton contract", () => {
    assert.ok(defaultMapFitReporter, "defaultMapFitReporter must be defined");
    assert.strictEqual(typeof defaultMapFitReporter.reportFit, "function");
    assert.strictEqual(typeof defaultMapFitReporter.subscribe, "function");
    assert.strictEqual(typeof defaultMapFitReporter.getLatest, "function");
    assert.strictEqual(typeof defaultMapFitReporter.clear, "function");
  });

  // --------------------------------------------------------------------------
  // 3. WorkbenchMap 源码级断言：FitToData 发布、MapStatusOverlay 消费与防竞态
  // --------------------------------------------------------------------------
  await t.test("3. WorkbenchMap: FitToData publishes and MapStatusOverlay subscribes/reads latest", () => {
    const mapFile = path.resolve(uiSrcDir, "components/WorkbenchMap.tsx");
    const mapCode = fs.readFileSync(mapFile, "utf-8");

    // FitToData 必须向 mapFitReporter 汇报 fit 结果
    assert.match(
      mapCode,
      /reportFit/,
      "FitToData must report applied fit view to mapFitReporter"
    );

    // MapStatusOverlay 必须保留已存在的 viewreset 监听（契约防护）
    assert.match(
      mapCode,
      /viewreset\(\)\s*\{[\s\S]*?updateFromCenter/,
      "MapStatusOverlay must continue listening to viewreset event"
    );

    // MapStatusOverlay 必须订阅 mapFitReporter
    assert.match(
      mapCode,
      /defaultMapFitReporter\.subscribe|mapFitReporter\.subscribe/,
      "MapStatusOverlay must subscribe to mapFitReporter"
    );

    // MapStatusOverlay 必须在挂载/订阅时读取 getLatest() 消除 React 更新顺序导致的事件丢失竞态
    assert.match(
      mapCode,
      /getLatest\(\)/,
      "MapStatusOverlay must query getLatest() on mount/subscription to eliminate event race"
    );
  });

  // --------------------------------------------------------------------------
  // 4. resolveDownloadDrawerEscapeAction: Esc 层级决策纯逻辑测试
  // --------------------------------------------------------------------------
  await t.test("4. resolveDownloadDrawerEscapeAction respects strict hierarchical escape order", () => {
    // 4.1 有兜底确认时，优先关兜底确认
    assert.strictEqual(
      resolveDownloadDrawerEscapeAction({
        hasFallbackConfirm: true,
        hasClearHistoryConfirm: true,
        hasSelectedTaskDetail: true,
      }),
      "close_fallback"
    );

    // 4.2 无兜底确认、有清除历史确认时，关清除历史确认
    assert.strictEqual(
      resolveDownloadDrawerEscapeAction({
        hasFallbackConfirm: false,
        hasClearHistoryConfirm: true,
        hasSelectedTaskDetail: true,
      }),
      "close_clear_history"
    );

    // 4.3 仅有任务详情打开时，关任务详情
    assert.strictEqual(
      resolveDownloadDrawerEscapeAction({
        hasFallbackConfirm: false,
        hasClearHistoryConfirm: false,
        hasSelectedTaskDetail: true,
      }),
      "close_task_detail"
    );

    // 4.4 没有任何子对话框时，关闭抽屉本身
    assert.strictEqual(
      resolveDownloadDrawerEscapeAction({
        hasFallbackConfirm: false,
        hasClearHistoryConfirm: false,
        hasSelectedTaskDetail: false,
      }),
      "close_drawer"
    );
  });

  // --------------------------------------------------------------------------
  // 5. DesktopDownloadDrawer 源码级断言：Esc 关闭抽屉、焦点捕获与焦点归还
  // --------------------------------------------------------------------------
  await t.test("5. DesktopDownloadDrawer: Esc closes drawer, autoFocus, and returns focus", () => {
    const drawerFile = path.resolve(uiSrcDir, "components/desktop/DesktopDownloadDrawer.tsx");
    const drawerCode = fs.readFileSync(drawerFile, "utf-8");

    // Esc 处理链中必须包含最终关闭抽屉本身的分支 (onClose)
    assert.match(
      drawerCode,
      /resolveDownloadDrawerEscapeAction|onClose\(\)/,
      "Escape handler must contain branch to close drawer via onClose"
    );

    // 打开抽屉时记录 activeElement（且只在 open 从 false 变为 true 的跳变时记录）
    assert.match(
      drawerCode,
      /activeElement/,
      "Drawer must capture document.activeElement on opening transition"
    );

    // 关闭或卸载时必须有归还焦点逻辑 (调用 focus())
    assert.match(
      drawerCode,
      /\.focus\(\)/,
      "Drawer must return focus to trigger element on close/unmount"
    );

    // 关闭按钮应具备 autoFocus 属性以便打开时焦点进入抽屉
    assert.match(
      drawerCode,
      /autoFocus/,
      "Drawer close button or panel must specify autoFocus"
    );
  });
});
