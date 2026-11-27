import test from "node:test";
import assert from "node:assert/strict";

// 纯 TS 测试（node --test --experimental-strip-types，导入必须带 .ts 扩展名）
import * as outputDir from "./confirmCardOutputDir.ts";
import * as disclosure from "./planDisclosure.ts";

test("W5-7 确认卡输出目录来源标签与计划卡披露（纯逻辑）", async (t) => {
  await t.test("1. projectOutputDirRow 来源标签映射与既有行为保护", () => {
    // 任务项 1：from_message -> 「消息中指定的目录」
    const rowFromMessage = outputDir.projectOutputDirRow(
      { output_dir_source: "from_message", output_dir: "D:\\data\\maoxian" },
      null,
    );
    assert.equal(rowFromMessage.editable, true);
    assert.equal(rowFromMessage.dir, "D:\\data\\maoxian");
    assert.equal(rowFromMessage.sourceLabel, "消息中指定的目录");
    assert.equal(rowFromMessage.approveBlocked, false);
    assert.equal(rowFromMessage.hint, null);

    // 既有映射保持字节一致
    const rowDefault = outputDir.projectOutputDirRow(
      { output_dir_source: "default", output_dir: "D:\\ScenePort\\Downloads" },
      null,
    );
    assert.equal(rowDefault.sourceLabel, "默认目录");

    const rowModelSuggested = outputDir.projectOutputDirRow(
      { output_dir_source: "model_suggested", output_dir: "E:\\SAR\\茂县叠溪" },
      null,
    );
    assert.equal(rowModelSuggested.sourceLabel, "模型建议的目录");

    const rowChosenSnapshot = outputDir.projectOutputDirRow(
      { output_dir_source: "chosen_on_card", output_dir: "D:\\Chosen" },
      null,
    );
    assert.equal(rowChosenSnapshot.sourceLabel, "卡上选择");

    // chosen 存在时一律覆盖为「卡上选择」
    const rowChosenOverrideFromMessage = outputDir.projectOutputDirRow(
      { output_dir_source: "from_message", output_dir: "D:\\data\\maoxian" },
      "F:\\override\\path",
    );
    assert.equal(rowChosenOverrideFromMessage.dir, "F:\\override\\path");
    assert.equal(rowChosenOverrideFromMessage.sourceLabel, "卡上选择");

    const rowChosenOverrideDefault = outputDir.projectOutputDirRow(
      { output_dir_source: "default", output_dir: "D:\\ScenePort" },
      "F:\\override\\path",
    );
    assert.equal(rowChosenOverrideDefault.sourceLabel, "卡上选择");

    // 未知 source 字符串不猜测，返回 null
    const rowUnknownSource = outputDir.projectOutputDirRow(
      { output_dir_source: "unknown_source_kind", output_dir: "D:\\data" },
      null,
    );
    assert.equal(rowUnknownSource.sourceLabel, null);

    // source 为 null 或非对象，返回 null
    const rowNullSource = outputDir.projectOutputDirRow(
      { output_dir_source: null, output_dir: "D:\\data" },
      null,
    );
    assert.equal(rowNullSource.sourceLabel, null);

    const rowNonObject = outputDir.projectOutputDirRow(null, null);
    assert.equal(rowNonObject.sourceLabel, null);
  });

  await t.test("2. disclosureLine 来源解析优先级与 display_name_zh 处理", () => {
    const sourceNames: Record<string, string> = {
      "science.s1.standard": "Sentinel-1 映射源",
      "science.alos.palsar": "ALOS PALSAR 映射源",
    };

    // (a) display_name_zh 去空白非空 → 原样使用（verbatim），即使 sourceNames 无此 id
    const resA = disclosure.disclosureLine(
      {
        resource_id: "science.unknown.dataset",
        display_name_zh: "Sentinel-1 雷达卫星",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(
      resA,
      "将向 Sentinel-1 雷达卫星 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    // display_name_zh 与 sourceNames 同时存在且不同时，后端 display_name_zh 胜出
    const resDifferent = disclosure.disclosureLine(
      {
        resource_id: "science.s1.standard",
        display_name_zh: "Sentinel-1 权威后端名",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(
      resDifferent,
      "将向 Sentinel-1 权威后端名 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    // (b) display_name_zh 为 null / 空串 / 纯空白 → 回退到 sourceNames[resource_id]
    const resNullFallback = disclosure.disclosureLine(
      {
        resource_id: "science.alos.palsar",
        display_name_zh: null,
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(
      resNullFallback,
      "将向 ALOS PALSAR 映射源 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    const resEmptyFallback = disclosure.disclosureLine(
      {
        resource_id: "science.alos.palsar",
        display_name_zh: "",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(
      resEmptyFallback,
      "将向 ALOS PALSAR 映射源 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    const resWhitespaceFallback = disclosure.disclosureLine(
      {
        resource_id: "science.alos.palsar",
        display_name_zh: "   \t\n  ",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(
      resWhitespaceFallback,
      "将向 ALOS PALSAR 映射源 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    const resUndefinedFallback = disclosure.disclosureLine(
      {
        resource_id: "science.alos.palsar",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(
      resUndefinedFallback,
      "将向 ALOS PALSAR 映射源 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    // (c) 两者皆无（display_name_zh 为 null/空/纯空白 且 sourceNames 中未找到或为空白）→ 「未知数据源」
    const resBothAbsent = disclosure.disclosureLine(
      {
        resource_id: "secret.unregistered.dataset.id",
        display_name_zh: null,
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      {},
    );
    assert.equal(
      resBothAbsent,
      "将向 未知数据源 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    const resBothBlank = disclosure.disclosureLine(
      {
        resource_id: "secret.unregistered.dataset.id",
        display_name_zh: "   ",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      { "secret.unregistered.dataset.id": "   " },
    );
    assert.equal(
      resBothBlank,
      "将向 未知数据源 检索 大同盆地试验区 2024-04-01 至 2024-04-30",
    );

    // (d) resource_id 绝不出现在输出中，只要后端给出了名字就绝不显示「未知数据源」
    const secretId = "secret.dataset.never.leak.id";
    const resNoLeak = disclosure.disclosureLine(
      {
        resource_id: secretId,
        display_name_zh: "Sentinel-1 雷达卫星",
        aoi_name: "大同盆地试验区",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      {},
    );
    assert.ok(!resNoLeak?.includes(secretId));
    assert.ok(!resNoLeak?.includes("未知数据源"));
    assert.ok(resNoLeak?.includes("Sentinel-1 雷达卫星"));
  });
});
