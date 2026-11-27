---
name: gee-textbook-cookbook
description: "Google Earth Engine (GEE) Python architecture, best practices & methodology handbook: Cloud Score+ vs SCL cloud masking & baseline harmonization, harmonic regression / Theil-Sen trend selection, Olofsson (2014) spatial CV & unbiased area estimation, hydrology/terrain exports, and server-side DAG performance/Join/debugging. Pairs with the 99 runnable recipes in eo-analysis-recipes."
---

# GEE 遥感与地学云端计算教材攻略 (GEE Textbook Cookbook)

本技能包基于渐进披露（Progressive Disclosure）结构组织，聚焦 **GEE 架构原理、方法学决策树、性能优化红线与常见报错排雷**；与按传感器和具体算法拆分的 **99 项独立可执行代码配方库 `eo-analysis-recipes`**（83 项 GEE Python + 16 项本地 GeoTIFF）明确互补、互不重复。请按需调用 `skill_read(skill_id="gee-textbook-cookbook", file_path=...)` 或 `skill_read(skill_id="gee-textbook-cookbook", query=...)` 精准检索：

| 专题子文档 | 核心原理、避坑规范与决定矩阵 | 适用场景 |
| --- | --- | --- |
| `references/optical-cloudmask-compositing.md` | Sentinel-2 `COPERNICUS/S2_SR_HARMONIZED` 与 Cloud Score+ (`linkCollection`) 首选去云、SCL 兜底与 `QA60` 弃用说明、Landsat C2 L2 `QA_PIXEL` + `QA_RADSAT` 与官方缩放系数 | 无云光学合成、跨 2022 基线长时序定标一致性 |
| `references/time-series-harmonic-trend.md` | 谐波回归（Harmonic OLS `sin/cos`）、Theil-Sen / Mann-Kendall (`select(["year", "NDVI"])`)、以及与 `LandTrendr`/`CCDC` 配方选型对比 | 植被物候、长期退化/恢复速率、扰动断点定代 |
| `references/classification-accuracy-assessment.md` | 空间分块独立交叉验证（防空间自相关泄露）、`smileRandomForest`、混淆矩阵与 Olofsson (2014) 分层无偏面积及 95% 置信区间公式 | 土地利用分类、高精度面积纠偏报告 |
| `references/hydrology-terrain-export.md` | Copernicus DEM 坡度/防误判淹没规则、JRC 水体频率基线交叉核验、EOStage `gee_chunked_download` 分块导出与 VRT 对齐 | 水文淹没制图、地形调制与大规模安全落盘 |
| `references/performance-joins-debugging.md` | 客户端/服务端隔离、`Filter Early`、禁用中间过程 `.clip`/`.reproject`、`tileScale`/`maxPixels` 调优、`ee.Join` 选型矩阵与高频超时/内存报错排错 | 消除 OOM/超时崩溃、多传感器时空关联 |

## 运行与校验铁律

1. **云端可用性与插件前提**：`gee_*` 工具仅在「插件」中安装并启用 `eostage-gee-plugin` 后解锁；中国大陆用户需在设置中配置网络代理与有效的 GEE OAuth / Cloud Project (`ee.Initialize(project=...)`) 凭据。
2. **禁止混用 TOA 与 SR**：同一次时间序列或变化检测严格限定单一辐射层级（优先 `COPERNICUS/S2_SR_HARMONIZED` 与 `LANDSAT/LC08/C02/T1_L2` / `LC09`）。
3. **跨 2022 Sentinel-2 基线防双扣**：`COPERNICUS/S2_SR_HARMONIZED` 已由 GEE 官方统一拉齐至 2022-01-25 之前的偏移量；**切勿**再手动加 `+1000` 或 `-1000`。
4. **先选配方再做调优**：需要具体传感器的现成 Python 函数时优先查阅 `eo-analysis-recipes`；遇到规模放大、内存溢出（`User memory limit exceeded`）、数组断点越界或耗时超时，立即按本技能 `performance-joins-debugging.md` 诊断重构。
