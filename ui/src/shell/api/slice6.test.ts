import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  getStatusStateConfig,
  type StateVariant,
} from "./statusStateLogic.ts";
import {
  formatStoragePanelInfo,
} from "./settingsStorageLogic.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W4 Slice 6: Motion, States & Storage Suite", async (t) => {
  await t.test("1. prefers-reduced-motion 禁用全部循环动画类与位移", () => {
    const cssPath = path.resolve(__dirname, "../../index.css");
    const cssContent = fs.readFileSync(cssPath, "utf-8");

    // 必须包含 prefers-reduced-motion 媒体查询
    assert.match(
      cssContent,
      /@media\s*\(\s*prefers-reduced-motion\s*:\s*reduce\s*\)/,
      "index.css 必须包含 @media (prefers-reduced-motion: reduce) 媒体查询"
    );

    // 提取媒体查询块内容
    const match = cssContent.match(
      /@media\s*\(\s*prefers-reduced-motion\s*:\s*reduce\s*\)\s*\{([^}]+(?:\}[^}]+)*)\}/
    );
    assert.ok(match, "必须能匹配到完整的 prefers-reduced-motion 媒体查询块");
    const mediaBlock = match[1];

    // 验证核心循环类在媒体查询内被显式禁用
    const requiredLoopClasses = [
      "dl-pulse-dot",
      "progress-indeterminate-bar",
      "animate-spin",
    ];

    for (const cls of requiredLoopClasses) {
      assert.ok(
        mediaBlock.includes(cls),
        `媒体查询内必须显式包含循环动画类 .${cls}`
      );
    }

    assert.ok(
      mediaBlock.includes("animation: none") || mediaBlock.includes("animation-duration"),
      "媒体查询内必须禁用动画"
    );
  });

  await t.test("2. 共享状态组件五种变体映射：图标、一句话、至多一个操作", () => {
    const variants: StateVariant[] = [
      "empty",
      "loading",
      "error",
      "not_connected",
      "login_required",
    ];

    for (const v of variants) {
      const config = getStatusStateConfig(v);
      assert.ok(config, `必须支持状态变体: ${v}`);
      assert.ok(config.iconName, `变体 ${v} 必须指定图标名称`);
      assert.ok(
        typeof config.title === "string" && config.title.length > 0,
        `变体 ${v} 必须有一句简明标题`
      );
      // 至多一个操作
      if (config.defaultAction) {
        assert.ok(
          typeof config.defaultAction.label === "string" && config.defaultAction.label.length > 0,
          `变体 ${v} 的操作必须有明确文案`
        );
      }
    }
  });

  await t.test("3. 存储与目录设置面板数据逻辑：路径展示与诚实数据契约", () => {
    // 未配置路径时
    const emptyInfo = formatStoragePanelInfo("");
    assert.equal(emptyInfo.displayPath, "未配置存储根路径");
    assert.equal(emptyInfo.dbSize, "--");
    assert.equal(emptyInfo.diskFree, "--");
    assert.equal(emptyInfo.pickActionLabel, "选择新存储位置");

    // 传入有效路径时
    const configuredInfo = formatStoragePanelInfo("D:\\ScenePortData");
    assert.equal(configuredInfo.displayPath, "D:\\ScenePortData");
    assert.equal(configuredInfo.dbSize, "--");
    assert.equal(configuredInfo.diskFree, "--");
  });
});
