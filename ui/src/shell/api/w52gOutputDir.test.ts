import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// 纯 TS 测试（node --test --experimental-strip-types，导入必须带 .ts 扩展名）
import * as outputDir from "./confirmCardOutputDir.ts";
import * as disclosure from "./planDisclosure.ts";

test("W5-2g 确认卡下载目录与计划卡检索披露（纯逻辑）", async (t) => {
  await t.test("1. validateLocalOutputDir 正例与反例", () => {
    // 正例
    assert.equal(outputDir.validateLocalOutputDir("D:\\EOStage\\Downloads"), null);
    assert.equal(outputDir.validateLocalOutputDir("C:/data/sar"), null);
    assert.equal(outputDir.validateLocalOutputDir("e:\\"), null);

    // 反例：空串
    assert.equal(outputDir.validateLocalOutputDir(""), "请输入下载目录");

    // 反例：相对路径、UNC、设备路径、POSIX、缺少分隔符
    assert.equal(
      outputDir.validateLocalOutputDir("relative\\dir"),
      "只接受本机盘符路径，例如 D:\\EOStage\\Downloads",
    );
    assert.equal(
      outputDir.validateLocalOutputDir("\\\\server\\share"),
      "只接受本机盘符路径，例如 D:\\EOStage\\Downloads",
    );
    assert.equal(
      outputDir.validateLocalOutputDir("\\\\?\\C:\\x"),
      "只接受本机盘符路径，例如 D:\\EOStage\\Downloads",
    );
    assert.equal(
      outputDir.validateLocalOutputDir("\\\\.\\C:\\x"),
      "只接受本机盘符路径，例如 D:\\EOStage\\Downloads",
    );
    assert.equal(
      outputDir.validateLocalOutputDir("/home/x"),
      "只接受本机盘符路径，例如 D:\\EOStage\\Downloads",
    );
    assert.equal(
      outputDir.validateLocalOutputDir("D:x"),
      "只接受本机盘符路径，例如 D:\\EOStage\\Downloads",
    );

    // 反例：首尾空格
    assert.equal(outputDir.validateLocalOutputDir(" D:\\x"), "目录不能以空格开头或结尾");
    assert.equal(outputDir.validateLocalOutputDir("D:\\x "), "目录不能以空格开头或结尾");

    // 反例：. 与 .. 段
    assert.equal(
      outputDir.validateLocalOutputDir("D:\\a\\..\\b"),
      "目录不能包含 . 或 .. 这样的段",
    );
    assert.equal(
      outputDir.validateLocalOutputDir("D:\\.\\b"),
      "目录不能包含 . 或 .. 这样的段",
    );

    // 反例：控制字符
    assert.equal(
      outputDir.validateLocalOutputDir("D:\\\u0007"),
      "目录不能包含控制字符",
    );

    // 边界与超长（按 UTF-8 字节数计）
    const exact1024 = "D:\\" + "a".repeat(1021); // 3 + 1021 = 1024 字节
    assert.equal(new TextEncoder().encode(exact1024).length, 1024);
    assert.equal(outputDir.validateLocalOutputDir(exact1024), null);

    const over1024Ascii = "D:\\" + "a".repeat(1022); // 3 + 1022 = 1025 字节
    assert.equal(new TextEncoder().encode(over1024Ascii).length, 1025);
    assert.equal(outputDir.validateLocalOutputDir(over1024Ascii), "目录过长");

    // 中文 UTF-8 每字 3 字节
    const over1024Utf8 = "D:\\" + "测".repeat(341); // 3 + 341 * 3 = 1026 字节
    assert.equal(new TextEncoder().encode(over1024Utf8).length, 1026);
    assert.equal(outputDir.validateLocalOutputDir(over1024Utf8), "目录过长");
  });

  await t.test("2. isCreateTaskSnapshot 识别 create_task 快照与排除 task_action 快照", () => {
    // create_task 形：包含自有属性 output_dir_source（即便为 null）
    assert.equal(outputDir.isCreateTaskSnapshot({ output_dir_source: null }), true);
    assert.equal(outputDir.isCreateTaskSnapshot({ output_dir_source: "default" }), true);

    // create_task 形：只有 already_queued_count 是有限数字
    assert.equal(outputDir.isCreateTaskSnapshot({ already_queued_count: 0 }), true);
    assert.equal(outputDir.isCreateTaskSnapshot({ already_queued_count: 5 }), true);

    // task_action 形：无 output_dir_source，already_queued_count 为 null
    assert.equal(
      outputDir.isCreateTaskSnapshot({
        output_dir: "D:\\x",
        result_count: null,
        already_queued_count: null,
      }),
      false,
    );

    // null / 非对象 / 空对象
    assert.equal(outputDir.isCreateTaskSnapshot(null), false);
    assert.equal(outputDir.isCreateTaskSnapshot(undefined), false);
    assert.equal(outputDir.isCreateTaskSnapshot("not_an_object"), false);
    assert.equal(outputDir.isCreateTaskSnapshot(123), false);
    assert.equal(outputDir.isCreateTaskSnapshot({}), false);
  });

  await t.test("3. projectOutputDirRow 四种来源标签与状态投影", () => {
    // 来源标签
    const rowDefault = outputDir.projectOutputDirRow(
      { output_dir_source: "default", output_dir: "D:\\ScenePort" },
      null,
    );
    assert.equal(rowDefault.editable, true);
    assert.equal(rowDefault.dir, "D:\\ScenePort");
    assert.equal(rowDefault.sourceLabel, "默认目录");
    assert.equal(rowDefault.approveBlocked, false);
    assert.equal(rowDefault.hint, null);

    const rowSuggested = outputDir.projectOutputDirRow(
      { output_dir_source: "model_suggested", output_dir: "D:\\ModelDir" },
      null,
    );
    assert.equal(rowSuggested.sourceLabel, "模型建议的目录");

    const rowChosenSnapshot = outputDir.projectOutputDirRow(
      { output_dir_source: "chosen_on_card", output_dir: "D:\\Chosen" },
      null,
    );
    assert.equal(rowChosenSnapshot.sourceLabel, "卡上选择");

    const rowNullSource = outputDir.projectOutputDirRow(
      { output_dir_source: null, output_dir: "D:\\Other" },
      null,
    );
    assert.equal(rowNullSource.sourceLabel, null);

    const rowUnknownSource = outputDir.projectOutputDirRow(
      { output_dir_source: "some_unrecognized", output_dir: "D:\\Other" },
      null,
    );
    assert.equal(rowUnknownSource.sourceLabel, null);

    // chosen 覆盖为「卡上选择」
    const rowOverridden = outputDir.projectOutputDirRow(
      { output_dir_source: "default", output_dir: "D:\\Original" },
      "E:\\CustomDir",
    );
    assert.equal(rowOverridden.dir, "E:\\CustomDir");
    assert.equal(rowOverridden.sourceLabel, "卡上选择");

    // 快照目录缺失或为 null 时 approveBlocked 与提示
    const rowMissingDir = outputDir.projectOutputDirRow(
      { output_dir_source: "default", output_dir: null },
      null,
    );
    assert.equal(rowMissingDir.editable, true);
    assert.equal(rowMissingDir.dir, null);
    assert.equal(rowMissingDir.approveBlocked, true);
    assert.equal(rowMissingDir.hint, "请先选择下载目录");

    // 空白目录视为 null
    const rowBlankDir = outputDir.projectOutputDirRow(
      { output_dir_source: "default", output_dir: "   " },
      null,
    );
    assert.equal(rowBlankDir.dir, null);
    assert.equal(rowBlankDir.approveBlocked, true);
    assert.equal(rowBlankDir.hint, "请先选择下载目录");

    // task_action 卡：editable=false 且 approveBlocked=false（即使 output_dir 为 null）
    const rowTaskAction = outputDir.projectOutputDirRow(
      { output_dir: null, already_queued_count: null },
      null,
    );
    assert.equal(rowTaskAction.editable, false);
    assert.equal(rowTaskAction.dir, null);
    assert.equal(rowTaskAction.approveBlocked, false);
    assert.equal(rowTaskAction.hint, null);
  });

  await t.test("4. approveOutputDirArg 仅在改写且合法时返回第三参数", () => {
    const snap = { output_dir_source: "default", output_dir: "D:\\Original" };

    // 未改 → undefined
    assert.equal(outputDir.approveOutputDirArg(snap, null), undefined);

    // 改成不同合法目录 → 该目录
    assert.equal(outputDir.approveOutputDirArg(snap, "E:\\NewDir"), "E:\\NewDir");

    // 改成与快照相同 → undefined
    assert.equal(outputDir.approveOutputDirArg(snap, "D:\\Original"), undefined);

    // 非法 chosen → undefined
    assert.equal(outputDir.approveOutputDirArg(snap, "invalid/relative"), undefined);
    assert.equal(outputDir.approveOutputDirArg(snap, "\\\\server\\share"), undefined);

    // task_action 卡 → undefined（不改写已有任务）
    const snapTaskAction = { output_dir: "D:\\Original", already_queued_count: null };
    assert.equal(outputDir.approveOutputDirArg(snapTaskAction, "E:\\NewDir"), undefined);
  });

  await t.test("5. defaultDirOffer 仅在有效且不同于当前目录时提供", () => {
    // 与当前目录相同 → null
    assert.equal(outputDir.defaultDirOffer("D:\\Downloads", "D:\\Downloads"), null);

    // 非法默认目录 → null
    assert.equal(outputDir.defaultDirOffer("relative\\dir", "D:\\Current"), null);
    assert.equal(outputDir.defaultDirOffer("\\\\server\\share", "D:\\Current"), null);

    // 空白或 null/undefined → null
    assert.equal(outputDir.defaultDirOffer("   ", "D:\\Current"), null);
    assert.equal(outputDir.defaultDirOffer(null, "D:\\Current"), null);
    assert.equal(outputDir.defaultDirOffer(undefined, "D:\\Current"), null);

    // 合法且与当前目录不同 → 返回它
    assert.equal(outputDir.defaultDirOffer("D:\\Downloads", "E:\\Other"), "D:\\Downloads");
    assert.equal(outputDir.defaultDirOffer("D:\\Downloads", null), "D:\\Downloads");
  });

  await t.test("6. disclosureLine 计划卡检索披露文本格式化", () => {
    const sourceNames: Record<string, string> = {
      asf_rtc: "ASF RTC",
      copernicus: "Copernicus Hub",
    };

    // 完整一例：来源名、AOI、两端时间（ISO 带时刻只取日期前 10 字符）
    const full = disclosure.disclosureLine(
      {
        resource_id: "asf_rtc",
        aoi_name: "舟山",
        time_range: {
          start: "2024-04-01T00:00:00Z",
          end: "2024-04-30T23:59:59.999Z",
        },
      },
      sourceNames,
    );
    assert.equal(full, "将向 ASF RTC 检索 舟山 2024-04-01 至 2024-04-30");

    // 来源查不到 →「未知数据源」且结果中不含 resource_id 原文
    const unknownSrc = disclosure.disclosureLine(
      {
        resource_id: "secret_dataset_id",
        aoi_name: "舟山",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(unknownSrc, "将向 未知数据源 检索 舟山 2024-04-01 至 2024-04-30");
    assert.ok(!unknownSrc!.includes("secret_dataset_id"));

    // 空 AOI →「未命名范围」
    const emptyAoi = disclosure.disclosureLine(
      {
        resource_id: "asf_rtc",
        aoi_name: "   ",
        time_range: { start: "2024-04-01", end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(emptyAoi, "将向 ASF RTC 检索 未命名范围 2024-04-01 至 2024-04-30");

    // 只有 start →「2024-04-01 起」
    const startOnly = disclosure.disclosureLine(
      {
        resource_id: "asf_rtc",
        aoi_name: "舟山",
        time_range: { start: "2024-04-01", end: null },
      },
      sourceNames,
    );
    assert.equal(startOnly, "将向 ASF RTC 检索 舟山 2024-04-01 起");

    // 只有 end →「至 2024-04-30」
    const endOnly = disclosure.disclosureLine(
      {
        resource_id: "asf_rtc",
        aoi_name: "舟山",
        time_range: { start: null, end: "2024-04-30" },
      },
      sourceNames,
    );
    assert.equal(endOnly, "将向 ASF RTC 检索 舟山 至 2024-04-30");

    // time_range 为 null / 缺失 / 两端均无 →「不限时间」
    const nullRange = disclosure.disclosureLine(
      {
        resource_id: "asf_rtc",
        aoi_name: "舟山",
        time_range: null,
      },
      sourceNames,
    );
    assert.equal(nullRange, "将向 ASF RTC 检索 舟山 不限时间");

    const noRange = disclosure.disclosureLine(
      {
        resource_id: "asf_rtc",
        aoi_name: "舟山",
      },
      sourceNames,
    );
    assert.equal(noRange, "将向 ASF RTC 检索 舟山 不限时间");

    // disclosure 不是对象或缺少 resource_id → null
    assert.equal(disclosure.disclosureLine(null, sourceNames), null);
    assert.equal(disclosure.disclosureLine(undefined, sourceNames), null);
    assert.equal(disclosure.disclosureLine("string", sourceNames), null);
    assert.equal(disclosure.disclosureLine({}, sourceNames), null);
    assert.equal(disclosure.disclosureLine({ aoi_name: "舟山" }, sourceNames), null);
  });

  await t.test("7. DesktopAgentWorkbench 源码断言包含 planApprove 接线与 pickDirectory 门控", () => {
    const desktopWorkbenchPath = path.resolve(
      __dirname,
      "../../components/desktop/DesktopAgentWorkbench.tsx",
    );
    const content = fs.readFileSync(desktopWorkbenchPath, "utf8");
    assert.ok(
      content.includes("bridge.agent.planApprove(planId, rawSnapshot, outputDir)"),
      "DesktopAgentWorkbench.tsx 必须调用 bridge.agent.planApprove(planId, rawSnapshot, outputDir)",
    );
    assert.ok(
      content.includes("onPickDirectory={isFixtureMode ? undefined : "),
      "DesktopAgentWorkbench.tsx 必须在 fixture 模式下不提供 onPickDirectory",
    );
  });
});
