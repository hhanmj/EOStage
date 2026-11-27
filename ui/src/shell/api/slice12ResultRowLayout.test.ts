import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  buildResultRowBadges,
  type ResultRowBadge,
} from "./rowProjection.ts";
import type { SceneRow } from "../../lib/bridge.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");
const desktopDir = path.resolve(uiSrcDir, "components/desktop");

test("W4 Slice 12-5b - Owner Feedback (d) Result Catalog Full Row Layout", async (t) => {
  // ==========================================================================
  // 1. buildResultRowBadges 纯函数逐字断言
  // ==========================================================================
  await t.test("1. 完整真实元数据：生成七类徽章并逐字匹配（W5-3b 无 label、直接展示值）", () => {
    const fullScene: SceneRow = {
      scene_id: "S1A_IW_SLC__1SDV_20231015T120000_20231015T120030_050000_060000_ABCD",
      platform: "Sentinel-1A",
      product_type: "RAW",
      processing_level: "SLC",
      beam_mode: "IW",
      polarization: "VV+VH",
      acquisition_datetime: "2023-10-15T12:00:00Z",
      orbit_direction: "ascending",
      relative_orbit: 11,
      frame: 65,
      absolute_orbit: 50000,
      path: null,
      file_size_remote: 2576980377, // ~2.4 GB
      has_url: true,
    };

    const badges = buildResultRowBadges(fullScene);
    assert.strictEqual(badges.length, 7, "Full scene must yield 7 badges");

    const dateBadge = badges.find((b) => b.key === "date");
    assert.ok(dateBadge, "Must include date badge");
    assert.strictEqual(dateBadge.value, "2023-10-15");

    const dirBadge = badges.find((b) => b.key === "direction");
    assert.ok(dirBadge, "Must include direction badge");
    assert.strictEqual(dirBadge.value, "Ascending");

    const orbitBadge = badges.find((b) => b.key === "orbit");
    assert.ok(orbitBadge, "Must include orbit badge");
    assert.strictEqual(orbitBadge.value, "Path 11");

    const frameBadge = badges.find((b) => b.key === "frame");
    assert.ok(frameBadge, "Must include frame badge");
    assert.strictEqual(frameBadge.value, "Frame 65");

    const polBadge = badges.find((b) => b.key === "polarization");
    assert.ok(polBadge, "Must include polarization badge");
    assert.strictEqual(polBadge.value, "VV+VH");

    const levelBadge = badges.find((b) => b.key === "level");
    assert.ok(levelBadge, "Must include level badge");
    assert.strictEqual(levelBadge.value, "SLC");

    const sizeBadge = badges.find((b) => b.key === "size");
    assert.ok(sizeBadge, "Must include size badge");
    assert.strictEqual(sizeBadge.value, "2.4 GB");
  });

  await t.test("2. 产品级别优先级：processing_level 优先，缺失退 product_type，两者皆缺整枚省略", () => {
    // 2.1 退回 product_type
    const fallbackScene: SceneRow = {
      scene_id: "S1_FALLBACK",
      platform: "Sentinel-1",
      product_type: "GRD",
      processing_level: null,
      beam_mode: "IW",
      polarization: "VV",
      acquisition_datetime: "2023-10-15T00:00:00Z",
      orbit_direction: "descending",
      relative_orbit: null,
      frame: null,
      absolute_orbit: null,
      path: null,
      has_url: false,
    };
    const badgesFallback = buildResultRowBadges(fallbackScene);
    const levelFallback = badgesFallback.find((b) => b.key === "level");
    assert.ok(levelFallback, "Level badge should exist via product_type");
    assert.strictEqual(levelFallback.value, "GRD");

    // 2.2 两者皆缺整枚省略（W5-3b: 次要字段缺省整枚省略，仅剩 4 个关键徽章）
    const absentScene: SceneRow = {
      scene_id: "S1_ABSENT",
      platform: "Sentinel-1",
      product_type: "",
      processing_level: null,
      beam_mode: "",
      polarization: "",
      acquisition_datetime: "",
      orbit_direction: "",
      relative_orbit: null,
      frame: null,
      absolute_orbit: null,
      path: null,
      has_url: false,
    };
    const badgesAbsent = buildResultRowBadges(absentScene);
    const levelAbsent = badgesAbsent.find((b) => b.key === "level");
    assert.strictEqual(levelAbsent, undefined, "Level badge must be completely omitted when both are missing");
    assert.strictEqual(badgesAbsent.length, 4, "Total badges should be 4 when secondary badges are omitted");
  });

  await t.test("3. 缺失值诚实回退：缺失退为未知（方向退为 Unknown），次要字段整枚省略，严禁编造", () => {
    const emptyScene: SceneRow = {
      scene_id: "EMPTY_SCENE",
      platform: "",
      product_type: "",
      processing_level: undefined,
      beam_mode: "",
      polarization: "",
      acquisition_datetime: "",
      orbit_direction: "",
      relative_orbit: null,
      frame: null,
      absolute_orbit: null,
      path: null,
      file_size_remote: null,
      has_url: false,
    };

    const badges = buildResultRowBadges(emptyScene);
    assert.strictEqual(badges.find((b) => b.key === "date")?.value, "未知");
    assert.strictEqual(badges.find((b) => b.key === "direction")?.value, "Unknown");
    assert.strictEqual(badges.find((b) => b.key === "orbit")?.value, "Path 未知");
    assert.strictEqual(badges.find((b) => b.key === "frame")?.value, "Frame 未知");
    assert.strictEqual(badges.find((b) => b.key === "polarization"), undefined);
    assert.strictEqual(badges.find((b) => b.key === "size"), undefined);
    assert.strictEqual(badges.find((b) => b.key === "level"), undefined);
    assert.strictEqual(badges.length, 4, "Empty scene has exactly 4 key badges");
  });

  await t.test("4. 轨道字段兼容：无 relative_orbit 但有 path 时显示 Path N", () => {
    const pathScene: SceneRow = {
      scene_id: "ALOS_PATH_SCENE",
      platform: "ALOS-2",
      product_type: "L1.1",
      beam_mode: "WBD",
      polarization: "HH",
      acquisition_datetime: "2024-01-01T00:00:00Z",
      orbit_direction: "desc",
      relative_orbit: null,
      path: 42,
      frame: 120,
      absolute_orbit: null,
      has_url: false,
    };

    const badges = buildResultRowBadges(pathScene);
    const orbitBadge = badges.find((b) => b.key === "orbit");
    assert.ok(orbitBadge);
    assert.strictEqual(orbitBadge.value, "Path 42");
  });

  // ==========================================================================
  // 2. 源码级断言：ManualWorkbench.tsx 整行布局规范
  // ==========================================================================
  await t.test("5. ManualWorkbench.tsx 源码断言：结果行容器不是小卡片网格", () => {
    const dockPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const content = fs.readFileSync(dockPath, "utf8");

    assert.strictEqual(
      content.includes("grid-cols-2"),
      false,
      "ManualWorkbench.tsx must not contain grid-cols-2"
    );
    assert.strictEqual(
      content.includes("grid-cols-3"),
      false,
      "ManualWorkbench.tsx must not contain grid-cols-3"
    );
    assert.strictEqual(
      content.includes("grid-cols-4"),
      false,
      "ManualWorkbench.tsx must not contain grid-cols-4"
    );
  });

  await t.test("6. ManualWorkbench.tsx 源码断言：场景 ID 整行不截断（不使用 truncateMiddle）", () => {
    const dockPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const content = fs.readFileSync(dockPath, "utf8");

    assert.strictEqual(
      content.includes("truncateMiddle("),
      false,
      "ManualWorkbench.tsx must not use truncateMiddle on scene id row"
    );
  });

  await t.test("7. ManualWorkbench.tsx 源码断言：列表行内清除小字号类（无 text-[9px]/text-[9.5px]/text-[10px]/text-[11px]）", () => {
    const dockPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const content = fs.readFileSync(dockPath, "utf8");

    // 截取列表行渲染部分 (scenes.map ...)
    const listMapStart = content.indexOf("scenes.map");
    assert.ok(listMapStart > 0, "ManualWorkbench.tsx must have scenes.map block");
    const listSlice = content.slice(listMapStart);

    assert.strictEqual(
      listSlice.includes("text-[9px]"),
      false,
      "Result row area must not contain text-[9px]"
    );
    assert.strictEqual(
      listSlice.includes("text-[9.5px]"),
      false,
      "Result row area must not contain text-[9.5px]"
    );
    assert.strictEqual(
      listSlice.includes("text-[10px]"),
      false,
      "Result row area must not contain text-[10px]"
    );
    assert.strictEqual(
      listSlice.includes("text-[11px]"),
      false,
      "Result row area must not contain text-[11px]"
    );
  });

  await t.test("8. CatalogResultRow.tsx 源码断言：徽章渲染接线遍历 buildResultRowBadges，且 ManualWorkbench 挂载 CatalogResultRow", () => {
    // W5-2c: 结果行结构已抽取至共用组件 CatalogResultRow.tsx，ManualWorkbench 挂载该组件
    const rowPath = path.join(desktopDir, "CatalogResultRow.tsx");
    const rowContent = fs.readFileSync(rowPath, "utf8");
    assert.ok(
      rowContent.includes("buildResultRowBadges("),
      "CatalogResultRow.tsx must call buildResultRowBadges to render badges"
    );

    const dockPath = path.join(desktopDir, "ManualWorkbench.tsx");
    const dockContent = fs.readFileSync(dockPath, "utf8");
    assert.ok(
      dockContent.includes("<CatalogResultRow"),
      "ManualWorkbench.tsx must mount <CatalogResultRow"
    );
  });

  // ==========================================================================
  // 3. 顺带清理断言：DesktopActivityRail.tsx 气泡卡无插件死类清理
  // ==========================================================================
  await t.test("9. DesktopActivityRail.tsx 源码断言：气泡卡清除 animate-in fade-in zoom-in-95 死类", () => {
    const railPath = path.join(desktopDir, "DesktopActivityRail.tsx");
    const content = fs.readFileSync(railPath, "utf8");

    assert.strictEqual(
      content.includes("animate-in fade-in zoom-in-95"),
      false,
      "DesktopActivityRail.tsx must not contain animate-in fade-in zoom-in-95"
    );
  });
});
