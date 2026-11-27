import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { reasoningSwitchUnsupportedZh } from "./agentChannelsLogic.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

function read(rel: string): string {
  return fs.readFileSync(path.resolve(here, rel), "utf-8");
}

test("W5-10 条目 4：渠道 reasoning_switch_unsupported 的中文标签", () => {
  assert.equal(reasoningSwitchUnsupportedZh({ reasoning_switch_unsupported: true }), "该渠道不支持关闭思考");
  assert.equal(reasoningSwitchUnsupportedZh({ reasoning_switch_unsupported: false }), null);
  assert.equal(reasoningSwitchUnsupportedZh({}), null);
});

test("W5-10 条目 4：模型设置页已移除渠道级思考控件，推理等级由模型级参数控制", () => {
  const tab = read("../components/settings/ModelChannelsSettingsTab.tsx");
  assert.equal(tab.includes("<ChannelReasoningField"), false, "渠道级思考控制应已移除");
  assert.equal(tab.includes("ChannelModelsEditor"), true, "保留模型列表与模型级参数编辑器");
});
