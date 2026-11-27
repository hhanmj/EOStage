import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import * as channelsLogic from "./agentChannelsLogic.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-6 R3 渠道 base URL /v1 提示与设置弹窗高度/标题/渠道名", async (t) => {
  await t.test("1. baseUrlNeedsV1Hint 纯逻辑判定", () => {
    const fn = (channelsLogic as Record<string, unknown>).baseUrlNeedsV1Hint as
      | ((url: string | null | undefined) => boolean)
      | undefined;
    assert.equal(typeof fn, "function", "agentChannelsLogic.ts 必须导出 baseUrlNeedsV1Hint");
    assert.equal(fn!("https://api.x.com"), true);
    assert.equal(fn!("https://api.x.com/"), true);
    assert.equal(fn!("https://api.x.com/v1"), false);
    assert.equal(fn!("https://api.x.com/V1/"), false);
    assert.equal(fn!("  https://api.x.com/v1  "), false);
    assert.equal(fn!(""), false);
    assert.equal(fn!("   "), false);
    assert.equal(fn!(null), false);
    assert.equal(fn!(undefined), false);
  });

  await t.test("2. appendV1ToBaseUrl 纯逻辑补全与幂等性", () => {
    const fn = (channelsLogic as Record<string, unknown>).appendV1ToBaseUrl as
      | ((url: string) => string)
      | undefined;
    assert.equal(typeof fn, "function", "agentChannelsLogic.ts 必须导出 appendV1ToBaseUrl");
    assert.equal(fn!("https://api.x.com"), "https://api.x.com/v1");
    assert.equal(fn!("https://api.x.com/"), "https://api.x.com/v1");
    assert.equal(fn!("https://api.x.com///"), "https://api.x.com/v1");
    assert.equal(fn!("https://api.x.com/v1"), "https://api.x.com/v1");
    assert.equal(fn!("https://api.x.com/V1/"), "https://api.x.com/V1/");
  });

  await t.test("3. formatChannelTestResult 锁定 HTTP 状态码显示与缺省行为", () => {
    const msgWithStatus = channelsLogic.formatChannelTestResult({
      ok: false,
      code: "model_authentication",
      status: 401,
      model_count: null,
    });
    assert.match(msgWithStatus, /HTTP 401/, "status 非 null 时应显示 HTTP 401");

    const msgWithoutStatus = channelsLogic.formatChannelTestResult({
      ok: false,
      code: "model_transient",
      status: null,
      model_count: null,
    });
    assert.ok(!msgWithoutStatus.includes("HTTP"), "status 为 null 时不应显示 HTTP");
  });

  await t.test("4. /v1 使用信息提示与紧凑追加操作，渠道行保持单行", () => {
    const tabPath = path.resolve(
      here,
      "../components/settings/ModelChannelsSettingsTab.tsx",
    );
    const src = fs.readFileSync(tabPath, "utf-8");

    assertLocalizedSource(src, "settings.channels.baseUrlHint", "OpenAI 兼容端点通常以 /v1 结尾");
    assertLocalizedSource(src, "settings.channels.appendV1", "追加 /v1");
    assert.ok(
      src.includes("baseUrlNeedsV1Hint") && src.includes("appendV1ToBaseUrl"),
      "ModelChannelsSettingsTab.tsx 未接入 baseUrlNeedsV1Hint 与 appendV1ToBaseUrl",
    );
    assertLocalizedSource(src, "settings.categories.models", "模型设置");
    assert.match(
      src,
      /sticky top-0 z-10[^>]*bg-sp-surface/,
      "ModelChannelsSettingsTab.tsx 的「模型设置」标题块未保留 sticky 标题",
    );
    const row = fs.readFileSync(path.resolve(here, "../components/settings/ChannelListItem.tsx"), "utf-8");
    assert.ok(row.includes("title={channel.id}"), "渠道名称保留完整值悬停查看入口");
  });

  // Owner 2026-10-29: 对齐对话区排版语言，设置页标题由 font-bold 调整为 text-lg font-semibold
  await t.test("5. SettingsModal.tsx 弹窗改为全窗口页面且内联标签页标题块为 text-lg font-semibold", () => {
    // Owner 2026-10-29: SettingsModal 改为全窗口页面，移除弹窗定高与内嵌 sticky 栏，对齐 font-semibold 排版规范
    const modalPath = path.resolve(here, "../../components/SettingsModal.tsx");
    const src = fs.readFileSync(modalPath, "utf-8");

    assert.ok(
      !src.includes("h-[620px]"),
      "SettingsModal.tsx 仍包含固定矮高度 h-[620px]",
    );
    assert.ok(
      // 留出 40px 标题栏（窗口按钮与拖动区不能被设置页盖住）
      src.includes("fixed inset-x-0 bottom-0 top-10") && src.includes("bg-sp-canvas"),
      "SettingsModal.tsx 缺少全窗口页面结构 fixed inset-0 与 bg-sp-canvas",
    );
    for (const [key, heading] of [
      ["settings.dialog.storageTitle", "存储与落盘路径"],
      ["settings.dialog.transferTitle", "传输与并发控制"],
    ] as const) {
      assertLocalizedSource(src, key, heading);
    }
  });
});
