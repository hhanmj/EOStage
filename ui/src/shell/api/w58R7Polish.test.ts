import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { shortSourceLabel, sourceSummary } from "./manualConditionsLogic.ts";
import { routeMeta } from "./shellRoutes.ts";
import * as rowProjection from "./rowProjection.ts";
import type { SourceDescriptor } from "./types.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => fs.readFileSync(path.resolve(__dirname, rel), "utf8");

/** 取出紧跟在某个 label 文字之后的第一个 <input ... /> 的 className 字符串。 */
function inputClassAfterLabel(source: string, labelKey: string): string {
  const labelAt = source.indexOf(`<label htmlFor={\`${labelKey}-\${draft.id}\`}`);
  assert.ok(labelAt >= 0, `找不到「${labelKey}」标签`);
  const inputAt = source.indexOf("<input", labelAt);
  const inputEnd = source.indexOf("/>", inputAt);
  return source.slice(inputAt, inputEnd);
}

test("W5-8 R7 设置/地图/历史细节（纯逻辑与源码形状）", async (suite) => {
  await suite.test("1. 渠道名称用界面字体，服务基地址与模型标识保留对应排版", () => {
    const src = read("../components/settings/ModelChannelsSettingsTab.tsx");
    assertLocalizedSource(src, "settings.channels.name", "名称");
    assert.ok(
      !inputClassAfterLabel(src, "channel-name").includes("font-mono"),
      "渠道名称输入框不得用等宽字体（基线 §3：等宽只给路径/URL/产品编号/哈希）",
    );
    const models = read("../components/settings/ChannelModelsEditor.tsx");
    assertLocalizedSource(models, "settings.channels.modelIdLabel", "模型 {index} 标识");
    const modelIdAt = models.indexOf('aria-label={t("settings.channels.modelIdLabel"');
    assert.ok(modelIdAt >= 0, "模型标识必须保留在二级编辑对话框中");
    assert.ok(
      !models.slice(modelIdAt, models.indexOf("/>", modelIdAt)).includes("font-mono"),
      "模型标识输入框不得用等宽字体",
    );
    assertLocalizedSource(src, "settings.channels.baseUrl", "服务基地址");
    assert.ok(inputClassAfterLabel(src, "channel-url").includes("font-mono"), "服务基地址是 URL，保留等宽");
  });

  await suite.test("2. 地图浮层 Esc 处理在偏好设置打开时让位", () => {
    const src = read("../../components/desktop/ManualWorkbench.tsx");
    const start = src.indexOf('if (panelMode !== "overlay") return;');
    assert.ok(start >= 0);
    const handler = src.slice(start, src.indexOf("window.addEventListener", start));
    assert.ok(
      handler.includes("isSettingsModalOpen()"),
      "地图浮层的捕获阶段 Esc 处理必须在偏好设置打开时直接返回（否则 stopPropagation 吞掉设置的 Esc）",
    );
  });

  await suite.test("3. 演示数据的 504 文案保留状态码、去掉英文短语", () => {
    for (const rel of ["./fixture.ts", "../../components/desktop/DesktopHarness.tsx"]) {
      const src = read(rel);
      assert.ok(!src.includes("Gateway Timeout"), `${rel} 不得含英文「Gateway Timeout」`);
      if (rel.endsWith("DesktopHarness.tsx")) {
        assertLocalizedSource(src, "shell.harness.fixtureError", "网络连接中断，下载端点响应超时（HTTP 504）");
      } else {
        assert.ok(src.includes("下载端点响应超时（HTTP 504）"), `${rel} 应写作「下载端点响应超时（HTTP 504）」`);
      }
    }
  });

  await suite.test("4. 来源与卫星摘要显示完整来源名（不截成「NISAR L」）", () => {
    assert.equal(shortSourceLabel("NISAR L波段 (ASF)"), "NISAR L波段 (ASF)");
    assert.equal(shortSourceLabel("NISAR S波段 (Bhoonidhi)"), "NISAR S波段 (Bhoonidhi)");
    assert.equal(shortSourceLabel("Sentinel-1 雷达卫星"), "Sentinel-1");
    const nisar = {
      resource_id: "science.nisar.l.v1",
      group_id: "asf",
      display_name_zh: "NISAR L波段 (ASF)",
      family: "sar",
      search_mode: "spatial_temporal",
      filter_schema_id: null,
      filter_schema_revision: null,
      result_cap: 1000,
      accounts: [],
      network_default: "direct",
    } as SourceDescriptor;
    assert.equal(sourceSummary(nisar), "NISAR L波段 (ASF)");
  });

  await suite.test("5. 同轨同幅列表显示文件名，缺失时「未命名」，绝不显示 provider ID", () => {
    const sceneListLabel = (rowProjection as Record<string, unknown>).sceneListLabel as
      | ((scene: { file_name?: string | null }) => string)
      | undefined;
    assert.equal(typeof sceneListLabel, "function", "rowProjection 需要导出 sceneListLabel");
    assert.equal(sceneListLabel!({ file_name: "  S1A_IW_SLC__1SDV_20261020.zip " }), "S1A_IW_SLC__1SDV_20261020.zip");
    assert.equal(sceneListLabel!({ file_name: "   " }), "未命名");
    assert.equal(sceneListLabel!({ file_name: null }), "未命名");

    const map = read("../../components/WorkbenchMap.tsx");
    assertLocalizedSource(map, "panels.map.sameTrackList", "同幅影像清单");
    const listAt = map.indexOf('aria-label={t("panels.map.sameTrackList")}');
    assert.ok(listAt >= 0);
    const list = map.slice(listAt, listAt + 3000);
    assert.ok(!list.includes("item.scene_id, 24"), "同轨同幅列表不得显示 scene_id 原文");
    assert.ok(!list.includes("title={item.scene_id}"), "同轨同幅列表悬停提示不得是 scene_id");
    assert.ok(list.includes("sceneListLabel(item)"));
  });

  await suite.test("6. 空对话标题为「对话」", () => {
    assert.equal(routeMeta("agent").label, "对话");
  });
});
