import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-6 R2 渠道删除桥接契约与设置页源码接线", async (t) => {
  await t.test("1. agentBridgeTypes.ts 包含 channelsDelete(channelId: string): Promise<AgentChannelView[]>", () => {
    const typesPath = path.resolve(here, "agentBridgeTypes.ts");
    const src = fs.readFileSync(typesPath, "utf-8");
    assert.match(
      src,
      /channelsSetDefault\(channelId: string\): Promise<AgentChannelView\[\]>;\s*channelsDelete\(channelId: string\): Promise<AgentChannelView\[\]>;/,
      "agentBridgeTypes.ts 缺少接线点：channelsSetDefault 之后未声明 channelsDelete(channelId: string): Promise<AgentChannelView[]>",
    );
  });

  await t.test("2. desktopShellBridgeTauri.ts 包含 agent_channels_delete 且载荷为 { channelId }", () => {
    const tauriBridgePath = path.resolve(
      here,
      "../../lib/desktopShellBridgeTauri.ts",
    );
    const src = fs.readFileSync(tauriBridgePath, "utf-8");
    assert.match(
      src,
      /channelsDelete:\s*\(channelId:\s*string\)\s*=>\s*dependencies\.invoke\("agent_channels_delete",\s*\{\s*channelId\s*\}\)\s*as\s*Promise<AgentChannelView\[\]>/,
      "desktopShellBridgeTauri.ts 缺少接线点：未实现 channelsDelete 调用 dependencies.invoke(\"agent_channels_delete\", { channelId })",
    );
  });

  await t.test("3. ModelChannelsSettingsTab.tsx 引用 ChannelDeleteControl 且删除成功路径调用 notifyChannelsChanged()（失败路径不调用）", () => {
    const tabPath = path.resolve(
      here,
      "../components/settings/ModelChannelsSettingsTab.tsx",
    );
    const src = fs.readFileSync(tabPath, "utf-8");

    assert.ok(
      src.includes("ChannelDeleteControl"),
      "ModelChannelsSettingsTab.tsx 缺少接线点：未引入或渲染 ChannelDeleteControl",
    );

    const deleteMatch = src.match(
      /const handleConfirmDelete = async \(\) => \{([\s\S]*?)\r?\n  \};/,
    );
    assert.ok(
      deleteMatch,
      "ModelChannelsSettingsTab.tsx 缺少接线点：未定义 handleConfirmDelete",
    );
    const deleteTryBlock = deleteMatch[1].split("catch")[0] ?? "";
    assert.match(
      deleteTryBlock,
      /channelsDelete\(draft\.id\)[\s\S]*setChannels\([\s\S]*notifyChannelsChanged\(\)[\s\S]*showNotice\("success",\s*t\("settings\.channels\.deleteSucceeded"\)\)/,
      "ModelChannelsSettingsTab.tsx 缺少接线点：handleConfirmDelete 成功路径未按序调用 channelsDelete(draft.id)、setChannels、notifyChannelsChanged() 与本地化删除成功提示",
    );
    assertLocalizedSource(src, "settings.channels.deleteSucceeded", "渠道已删除");
    const deleteCatchBlock = deleteMatch[1].split("catch")[1] ?? "";
    assert.ok(
      !deleteCatchBlock.includes("notifyChannelsChanged"),
      "ModelChannelsSettingsTab.tsx handleConfirmDelete 失败路径不应调用 notifyChannelsChanged()",
    );
  });

  await t.test("4. desktopShellBridgeFixture.ts 与 desktopShellBridgePywebview.ts 未新增 agent 代理", () => {
    const fixturePath = path.resolve(
      here,
      "../../lib/desktopShellBridgeFixture.ts",
    );
    const pywebviewPath = path.resolve(
      here,
      "../../lib/desktopShellBridgePywebview.ts",
    );
    const fixtureSrc = fs.readFileSync(fixturePath, "utf-8");
    const pywebviewSrc = fs.readFileSync(pywebviewPath, "utf-8");

    assert.ok(
      !/\bagent\s*:/.test(fixtureSrc) && !fixtureSrc.includes("channelsDelete"),
      "desktopShellBridgeFixture.ts 不得新增 agent 代理或 channelsDelete",
    );
    assert.ok(
      !/\bagent\s*:/.test(pywebviewSrc) && !pywebviewSrc.includes("channelsDelete"),
      "desktopShellBridgePywebview.ts 不得新增 agent 代理或 channelsDelete",
    );
  });
});
