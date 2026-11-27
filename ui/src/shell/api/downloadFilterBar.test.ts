import test from "node:test";
import assert from "node:assert/strict";
import {
  isInternalDatasetId,
  normalizeTaskDisplayName,
  shouldHideRedundantDatasetToken,
  taskDataTypeKey,
  taskDataTypeLabel,
  taskProductBadgeMeta,
} from "../../components/desktop/downloadFilterLogic.ts";

test("taskDataTypeKey 纯逻辑单元测试", async (t) => {
  await t.test("DEM 转换类别恒映射为 'DEM 转换'", () => {
    assert.equal(
      taskDataTypeLabel(taskDataTypeKey({ category: "dem_convert", dataset: "COP30 DEM" })),
      "DEM 转换"
    );
    assert.equal(
      taskDataTypeLabel(taskDataTypeKey({ category: "dem_convert", dataset: "NASADEM", name: "高程转换" })),
      "DEM 转换"
    );
  });

  await t.test("Sentinel-1 家族细分 SLC 与 GRD", () => {
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "Sentinel-1", name: "Sentinel-1 SLC 5 景批次" }),
      "Sentinel-1 SLC"
    );
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "Sentinel-1 SLC", name: "下载任务" }),
      "Sentinel-1 SLC"
    );
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "Sentinel-1", name: "Sentinel-1 GRD 3 景批次" }),
      "Sentinel-1 GRD"
    );
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "Sentinel-1 GRD", name: "GRD 下载" }),
      "Sentinel-1 GRD"
    );
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "Sentinel-1", name: "通用观测" }),
      "Sentinel-1"
    );
  });

  await t.test("NISAR 家族细分 L 与 S 波段", () => {
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "NISAR L", name: "NISAR L波段 2 景批次" }),
      "NISAR L"
    );
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "NISAR", name: "NISAR S波段 1 景" }),
      "NISAR S"
    );
  });

  await t.test("ALOS-2 映射为 'ALOS-2'", () => {
    assert.equal(
      taskDataTypeKey({ category: "scene_download", dataset: "ALOS-2", name: "ALOS-2 PALSAR-2" }),
      "ALOS-2"
    );
  });

  await t.test("无明确数据集时回退至 provider 或 '其他'", () => {
    assert.equal(
      taskDataTypeKey({ category: "scene_download", provider: "ASF", dataset: "" }),
      "ASF"
    );
    assert.equal(
      taskDataTypeLabel(taskDataTypeKey({ category: "scene_download", provider: "", dataset: "" })),
      "其他"
    );
    assert.equal(taskDataTypeLabel(taskDataTypeKey(null)), "其他");
    assert.equal(taskDataTypeLabel(taskDataTypeKey(undefined)), "其他");
  });
});

test("光学数据集内部 ID 不直接作为筛选标签或摘要展示", () => {
  assert.equal(taskDataTypeLabel(taskDataTypeKey({ dataset: "optical.landsat.c2.l2", provider: "MPC" })), "Landsat");
  assert.equal(taskDataTypeLabel(taskDataTypeKey({ dataset: "optical.sentinel-2.l2a" })), "Sentinel-2");
  assert.equal(isInternalDatasetId("optical.landsat.c2.l2"), true);
  assert.equal(isInternalDatasetId("sar.nisar.s.ssar.rslc.v1"), true);
  assert.equal(isInternalDatasetId("Sentinel-1 SLC"), false);
  assert.equal(isInternalDatasetId(""), false);
});

test("taskProductBadgeMeta 细分 Sentinel-1 SLC/GRD 与 NISAR S/L 裁剪/完整标识", () => {
  assert.deepEqual(
    taskProductBadgeMeta({
      category: "scene_download",
      dataset: "SENTINEL-1",
      task_type: "sentinel1_slc",
    }),
    { label: "Sentinel-1 SLC", isSubset: false, modeLabel: null }
  );
  assert.deepEqual(
    taskProductBadgeMeta({
      category: "scene_download",
      dataset: "SENTINEL-1",
      task_type: "sentinel1_grd",
    }),
    { label: "Sentinel-1 GRD", isSubset: false, modeLabel: null }
  );
  assert.deepEqual(
    taskProductBadgeMeta({
      category: "scene_download",
      dataset: "sar.nisar.s.ssar.rslc.v1",
      task_type: "nisar_subset",
    }),
    { label: "NISAR S · 裁剪", isSubset: true, modeLabel: "裁剪" }
  );
  assert.deepEqual(
    taskProductBadgeMeta({
      category: "scene_download",
      dataset: "sar.nisar.l.rslc.provisional.v1",
      task_type: "nisar_complete",
    }),
    { label: "NISAR L · 完整", isSubset: false, modeLabel: "完整" }
  );
});

test("shouldHideRedundantDatasetToken hides placeholder dashes and blank values", () => {
  assert.equal(shouldHideRedundantDatasetToken("—"), true);
  assert.equal(shouldHideRedundantDatasetToken("-"), true);
  assert.equal(shouldHideRedundantDatasetToken("--"), true);
  assert.equal(shouldHideRedundantDatasetToken("  "), true);
  assert.equal(shouldHideRedundantDatasetToken(null), true);
  assert.equal(shouldHideRedundantDatasetToken("Sentinel-1"), false);
});

test("ALOS PALSAR 25m Mosaic distinguishes from ALOS-2 and normalizes internal IDs", () => {
  assert.equal(
    taskDataTypeKey({ category: "scene_download", dataset: "alos_palsar_mosaic", name: "ALOS PALSAR 25m Mosaic" }),
    "ALOS PALSAR"
  );
  assert.equal(
    taskDataTypeKey({ category: "scene_download", dataset: "ALOS-2", name: "ALOS-2 PALSAR-2" }),
    "ALOS-2"
  );
  assert.equal(
    taskDataTypeKey({ category: "scene_download", dataset: "optical.sentinel-3.standard" }),
    "Sentinel-3"
  );
  assert.equal(normalizeTaskDisplayName({ name: "optical.sentinel-3.standard" }), "Sentinel-3 标准光学产品");
  assert.equal(normalizeTaskDisplayName({ name: "nasa_ecostress" }), "NASA ECOSTRESS 地表温度");
  assert.equal(normalizeTaskDisplayName({ name: "alos_palsar_mosaic" }), "ALOS PALSAR 25m 镶嵌数据集");
  assert.equal(normalizeTaskDisplayName({ name: "dem.aw3d30-ellipsoidal.v1" }), "ALOS World 3D 椭球高 (AW3D30)");
  assert.equal(normalizeTaskDisplayName({ name: "gem15_dtm" }), "GEM-15 DTM 地形高程 (15m)");
  assert.equal(isInternalDatasetId("nasa_ecostress"), true);
  assert.equal(isInternalDatasetId("alos_palsar_mosaic"), true);
  assert.equal(shouldHideRedundantDatasetToken("nasa_ecostress"), true);
});
