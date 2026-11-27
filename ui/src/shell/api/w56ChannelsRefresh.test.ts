import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import * as channelEvents from "./agentChannelsEvents.ts";
import {
  subscribeChannelsChanged,
  notifyChannelsChanged,
} from "./agentChannelsEvents.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-6 R1 渠道保存与设默认后即时刷新（发布订阅与源码接线）", async (t) => {
  await t.test("1. 模块仅导出 subscribeChannelsChanged 与 notifyChannelsChanged", () => {
    assert.deepEqual(
      Object.keys(channelEvents).sort(),
      ["notifyChannelsChanged", "subscribeChannelsChanged"],
      "agentChannelsEvents.ts 应仅导出 subscribeChannelsChanged 与 notifyChannelsChanged",
    );
  });

  await t.test("2. 订阅后 notifyChannelsChanged 使回调恰执行一次，退订后不再执行", () => {
    let count = 0;
    const unsubscribe = subscribeChannelsChanged(() => {
      count += 1;
    });
    notifyChannelsChanged();
    assert.equal(count, 1, "订阅后调用 notifyChannelsChanged 应使回调恰执行 1 次");

    unsubscribe();
    notifyChannelsChanged();
    assert.equal(count, 1, "退订后调用 notifyChannelsChanged 不应再触发回调");
  });

  await t.test("3. 两个订阅者都收到通知，单方退订不影响另一方", () => {
    const order: string[] = [];
    const unsubA = subscribeChannelsChanged(() => {
      order.push("A");
    });
    const unsubB = subscribeChannelsChanged(() => {
      order.push("B");
    });

    notifyChannelsChanged();
    assert.deepEqual(order, ["A", "B"], "两个订阅者都应同步收到一次通知");

    unsubA();
    notifyChannelsChanged();
    assert.deepEqual(order, ["A", "B", "B"], "A 退订后仅 B 继续收到通知");

    unsubB();
  });

  await t.test("4. ModelChannelsSettingsTab.tsx 在 handleSaveConfig 与 handleSetDefault 成功路径调用 notifyChannelsChanged", () => {
    const tabPath = path.resolve(
      here,
      "../components/settings/ModelChannelsSettingsTab.tsx",
    );
    const src = fs.readFileSync(tabPath, "utf-8");

    const saveMatch = src.match(
      /const handleSaveConfig = async \(\) => \{([\s\S]*?)\n  \};/,
    );
    assert.ok(saveMatch, "ModelChannelsSettingsTab.tsx 缺少 handleSaveConfig 定义");
    const saveTryBlock = saveMatch[1].split("catch")[0] ?? "";
    assert.match(
      saveTryBlock,
      /channelsSave[\s\S]*setChannels\(refreshed\)[\s\S]*notifyChannelsChanged\(\)/,
      "ModelChannelsSettingsTab.tsx 缺少接线点：handleSaveConfig 成功路径（channelsSave 与 setChannels 之后）未调用 notifyChannelsChanged()",
    );
    const saveCatchBlock = saveMatch[1].split("catch")[1] ?? "";
    assert.ok(
      !saveCatchBlock.includes("notifyChannelsChanged"),
      "ModelChannelsSettingsTab.tsx handleSaveConfig 失败路径不应调用 notifyChannelsChanged()",
    );

    const setDefaultMatch = src.match(
      /const handleSetDefault = async \(\) => \{([\s\S]*?)\n  \};/,
    );
    assert.ok(
      setDefaultMatch,
      "ModelChannelsSettingsTab.tsx 缺少 handleSetDefault 定义",
    );
    const defaultTryBlock = setDefaultMatch[1].split("catch")[0] ?? "";
    assert.match(
      defaultTryBlock,
      /channelsSetDefault[\s\S]*setChannels\(refreshed\)[\s\S]*notifyChannelsChanged\(\)/,
      "ModelChannelsSettingsTab.tsx 缺少接线点：handleSetDefault 成功路径（channelsSetDefault 与 setChannels 之后）未调用 notifyChannelsChanged()",
    );
    const defaultCatchBlock = setDefaultMatch[1].split("catch")[1] ?? "";
    assert.ok(
      !defaultCatchBlock.includes("notifyChannelsChanged"),
      "ModelChannelsSettingsTab.tsx handleSetDefault 失败路径不应调用 notifyChannelsChanged()",
    );
  });

  await t.test("5. DesktopAgentWorkbench.tsx 订阅 subscribeChannelsChanged 并关联 checkChannels", () => {
    const workbenchPath = path.resolve(
      here,
      "../../components/desktop/DesktopAgentWorkbench.tsx",
    );
    const src = fs.readFileSync(workbenchPath, "utf-8");

    assert.ok(
      src.includes("subscribeChannelsChanged"),
      "DesktopAgentWorkbench.tsx 缺少接线点：未引入或调用 subscribeChannelsChanged",
    );
    assert.match(
      src,
      /useEffect\(\(\) => \{\s*return subscribeChannelsChanged\(\(\) => \{\s*void checkChannels\(\);\s*\}\);\s*\}, \[checkChannels\]\)/,
      "DesktopAgentWorkbench.tsx 缺少接线点：subscribeChannelsChanged effect 未关联 void checkChannels() 或依赖 [checkChannels]",
    );
  });

  await t.test("6. DesktopWorkbench.tsx 订阅 subscribeChannelsChanged 刷新助手栏渠道就绪状态", () => {
    const desktopPath = path.resolve(
      here,
      "../../components/desktop/DesktopWorkbench.tsx",
    );
    const src = fs.readFileSync(desktopPath, "utf-8");

    assert.ok(
      src.includes("subscribeChannelsChanged"),
      "DesktopWorkbench.tsx 缺少接线点：未引入或调用 subscribeChannelsChanged",
    );
    assert.match(
      src,
      /subscribeChannelsChanged\(/,
      "DesktopWorkbench.tsx 缺少接线点：未在 effect 中调用 subscribeChannelsChanged 订阅渠道变更",
    );
  });
});
