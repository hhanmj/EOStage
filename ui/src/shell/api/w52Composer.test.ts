import test from "node:test";
import assert from "node:assert/strict";

import {
  agentAttachItems,
  agentApprovalModeOptions,
  projectChannelOptions,
  composerChannelLabel,
  canSendComposer,
  AGENT_VECTOR_EXTENSIONS,
  isAcceptedVectorFileName,
  vectorAoiDisplayName,
} from "./agentComposerLogic.ts";
import type { AgentChannelView } from "./agentBridgeTypes.ts";

test("W5-2a Agent Composer Logic Specification", async (t) => {
  // ==========================================================================
  // 1. agentAttachItems (mockup-exact order & capability omission)
  // ==========================================================================
  await t.test("1. agentAttachItems: respects mockup order and capability omissions", () => {
    // 全能力就绪：返回完整 3 项，顺序与文案严格对齐样稿
    const allEnabled = agentAttachItems({
      rectDraw: true,
      adminBoundary: true,
      vectorUpload: true,
    });
    assert.deepStrictEqual(allEnabled, [
      { id: "rect-draw", label_zh: "在地图上画矩形", aside_zh: null },
      { id: "admin-region", label_zh: "选择行政区", aside_zh: null },
      { id: "vector-upload", label_zh: "上传矢量文件", aside_zh: ".shp .geojson .kml" },
    ]);

    // 缺少画矩形能力：完全省略对应项
    const noRect = agentAttachItems({
      rectDraw: false,
      adminBoundary: true,
      vectorUpload: true,
    });
    assert.deepStrictEqual(noRect, [
      { id: "admin-region", label_zh: "选择行政区", aside_zh: null },
      { id: "vector-upload", label_zh: "上传矢量文件", aside_zh: ".shp .geojson .kml" },
    ]);

    // 仅保留上传能力
    const onlyUpload = agentAttachItems({
      rectDraw: false,
      adminBoundary: false,
      vectorUpload: true,
    });
    assert.deepStrictEqual(onlyUpload, [
      { id: "vector-upload", label_zh: "上传矢量文件", aside_zh: ".shp .geojson .kml" },
    ]);

    // 全部不可用：返回空数组
    const allDisabled = agentAttachItems({
      rectDraw: false,
      adminBoundary: false,
      vectorUpload: false,
    });
    assert.deepStrictEqual(allDisabled, []);
  });

  // ==========================================================================
  // 2. agentApprovalModeOptions (single-mode disable, no invented modes)
  // ==========================================================================
  await t.test("2. agentApprovalModeOptions: single confirm_each disables switching, ignores unknowns", () => {
    // 恰好支持 confirm_each：只包含一个选项，且切换被禁用
    const single = agentApprovalModeOptions(["confirm_each"]);
    assert.deepStrictEqual(single, {
      options: [{ value: "confirm_each", label_zh: "每次确认" }],
      switchingDisabled: true,
    });

    // 含有未知或伪造模式：只保留已知的 confirm_each，绝不凭空发明模式
    const withUnknowns = agentApprovalModeOptions(["auto", "confirm_each", "unattended"]);
    assert.deepStrictEqual(withUnknowns.options, [
      { value: "confirm_each", label_zh: "每次确认" },
    ]);
    assert.strictEqual(withUnknowns.switchingDisabled, true);

    // 未提供任何有效支持模式
    const empty = agentApprovalModeOptions([]);
    assert.deepStrictEqual(empty, {
      options: [],
      switchingDisabled: true,
    });
  });

  // ==========================================================================
  // 3. projectChannelOptions (order projection, labels, key completeness)
  // ==========================================================================
  await t.test("3. projectChannelOptions: default channel moves first, labels formatted correctly", () => {
    const rawViews: AgentChannelView[] = [
      {
        id: "chan-backup",
        protocol: "anthropic_messages",
        base_url: "https://api.anthropic.com/v1",
        model: "claude-3-5-sonnet",
        key_ready: false,
        is_default: false,
      },
      {
        id: "chan-openai-default",
        protocol: "chat_completions",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o",
        key_ready: true,
        is_default: true,
      },
      {
        id: "chan-responses",
        protocol: "responses",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o-realtime",
        key_ready: true,
        is_default: false,
      },
    ];

    const result = projectChannelOptions(rawViews);
    assert.strictEqual(result.hasAny, true);
    assert.strictEqual(result.allKeysMissing, false);
    assert.strictEqual(result.options.length, 3);

    // 默认渠道排第一
    assert.strictEqual(result.options[0].id, "chan-openai-default");
    assert.strictEqual(result.options[0].isDefault, true);
    assert.strictEqual(result.options[0].keyReady, true);
    assert.strictEqual(result.options[0].label_zh, "Chat Completions (/chat/completions) · gpt-4o");

    // 其余渠道保持相对顺序
    assert.strictEqual(result.options[1].id, "chan-backup");
    assert.strictEqual(result.options[1].label_zh, "Anthropic Messages (/v1/messages) · claude-3-5-sonnet");
    assert.strictEqual(result.options[2].id, "chan-responses");
    assert.strictEqual(result.options[2].label_zh, "Responses (/responses) · gpt-4o-realtime");

    // 全无密钥测试
    const noKeysViews: AgentChannelView[] = [
      {
        id: "c1",
        protocol: "chat_completions",
        base_url: "https://example.com",
        model: "m1",
        key_ready: false,
        is_default: true,
      },
    ];
    const noKeysResult = projectChannelOptions(noKeysViews);
    assert.strictEqual(noKeysResult.allKeysMissing, true);
    assert.strictEqual(noKeysResult.hasAny, true);

    // 空渠道视图
    const emptyResult = projectChannelOptions([]);
    assert.strictEqual(emptyResult.hasAny, false);
    assert.strictEqual(emptyResult.allKeysMissing, false);
    assert.deepStrictEqual(emptyResult.options, []);
  });

  // ==========================================================================
  // 4. composerChannelLabel (ready & hasAny determines label)
  // ==========================================================================
  await t.test("4. composerChannelLabel: shows 默认模型渠道 when ready, else 待配密钥", () => {
    // 渠道就绪且存在渠道
    assert.strictEqual(
      composerChannelLabel({ hasAny: true, allKeysMissing: false, ready: true }),
      "默认模型渠道"
    );

    // 未就绪（例如未配置有效密钥）
    assert.strictEqual(
      composerChannelLabel({ hasAny: true, allKeysMissing: true, ready: false }),
      "待配密钥"
    );

    // 无任何渠道
    assert.strictEqual(
      composerChannelLabel({ hasAny: false, allKeysMissing: false, ready: false }),
      "待配密钥"
    );

    // 即便 ready 但 hasAny 为 false 仍为待配密钥
    assert.strictEqual(
      composerChannelLabel({ hasAny: false, allKeysMissing: false, ready: true }),
      "待配密钥"
    );
  });

  // ==========================================================================
  // 5. canSendComposer (gating logic)
  // ==========================================================================
  await t.test("5. canSendComposer: requires non-empty trimmed text, channel ready, not busy", () => {
    // 满足所有条件
    assert.strictEqual(
      canSendComposer({ text: "检索大同盆地数据", channelReady: true, busy: false }),
      true
    );

    // 默认 busy 为 false/undefined
    assert.strictEqual(
      canSendComposer({ text: "检索大同盆地数据", channelReady: true }),
      true
    );

    // 文本为空或仅包含空白字符
    assert.strictEqual(
      canSendComposer({ text: "", channelReady: true }),
      false
    );
    assert.strictEqual(
      canSendComposer({ text: "   \n\t  ", channelReady: true }),
      false
    );

    // 渠道未就绪
    assert.strictEqual(
      canSendComposer({ text: "检索大同盆地数据", channelReady: false }),
      false
    );

    // 忙态中
    assert.strictEqual(
      canSendComposer({ text: "检索大同盆地数据", channelReady: true, busy: true }),
      false
    );
  });

  // ==========================================================================
  // 6. Vector File Extensions & Acceptance
  // ==========================================================================
  await t.test("6. isAcceptedVectorFileName: accepts supported formats case-insensitively", () => {
    assert.deepStrictEqual(AGENT_VECTOR_EXTENSIONS, [
      ".shp",
      ".zip",
      ".geojson",
      ".kml",
      ".kmz",
      ".json",
    ]);

    // 合法后缀（大小写均支持）
    assert.strictEqual(isAcceptedVectorFileName("study_area.shp"), true);
    assert.strictEqual(isAcceptedVectorFileName("AREA.SHP"), true);
    assert.strictEqual(isAcceptedVectorFileName("boundary.geojson"), true);
    assert.strictEqual(isAcceptedVectorFileName("track.kml"), true);
    assert.strictEqual(isAcceptedVectorFileName("polygon.kmz"), true);
    assert.strictEqual(isAcceptedVectorFileName("features.json"), true);
    assert.strictEqual(isAcceptedVectorFileName("shapefile_bundle.zip"), true);
    assert.strictEqual(isAcceptedVectorFileName("C:\\gis\\data\\test.GEOJSON"), true);
    assert.strictEqual(isAcceptedVectorFileName("/data/shapes/aoi.KML"), true);

    // 非法后缀与无后缀
    assert.strictEqual(isAcceptedVectorFileName("image.tif"), false);
    assert.strictEqual(isAcceptedVectorFileName("table.csv"), false);
    assert.strictEqual(isAcceptedVectorFileName("boundary"), false);
    assert.strictEqual(isAcceptedVectorFileName(""), false);
    assert.strictEqual(isAcceptedVectorFileName(".shp"), false); // 纯扩展名无主文件名视为无效文件
  });

  // ==========================================================================
  // 7. vectorAoiDisplayName (extension stripping & base naming)
  // ==========================================================================
  await t.test("7. vectorAoiDisplayName: extracts clean display name without extension", () => {
    assert.strictEqual(vectorAoiDisplayName("datong_basin.geojson"), "datong_basin");
    assert.strictEqual(vectorAoiDisplayName("SHANGHAI_AREA.SHP"), "SHANGHAI_AREA");
    assert.strictEqual(vectorAoiDisplayName("C:\\data\\test_aoi.kml"), "test_aoi");
    assert.strictEqual(vectorAoiDisplayName("/var/gis/boundary.json"), "boundary");
  });
});
