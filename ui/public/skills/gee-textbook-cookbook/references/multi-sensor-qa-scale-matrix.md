# 多传感器数据集 QA 掩膜、缩放定标与尺度投影速查矩阵

编写代码或审查结果前，先按本表核对官方数据集 ID、QA 掩膜字段、物理缩放偏移及尺度限制；**切勿仅凭记忆填写旧版本集合或跳过偏移项**。

## 1. 主流数据集质量控制与定标矩阵

| 类别与首选资产 ID | 分辨率/频次 | 定标/物理缩放与单位 | QA 掩膜基准 | 常见红线 |
| --- | --- | --- | --- | --- |
| Sentinel-2 SR (`COPERNICUS/S2_SR_HARMONIZED`) | 10–20 m / ~5 天 | 反射率 `DN / 10000` (无量纲 `[0,1]`)；已对齐 2022 基线 | 首选 `Cloud Score+` (`cs >= 0.60` 或 `cs_cdf`)；兜底 `SCL` 剔 `0,1,3,8,9,10,11` | **严禁再加减 `1000`**；`QA60` 缺 2022–2024 记录不可跨年单用 |
| Sentinel-2 云概率 (`COPERNICUS/S2_CLOUD_PROBABILITY`) | 10 m / 同步 S2 | `probability` 范围 `0..100` (越低越晴空) | 按 `system:index` Join 到 S2 后设阈值 `< 40` 等并投影云影 | 需显式记录阈值并展示至少一个掩膜诊断层 |
| Landsat 8/9 C2 L2 (`LANDSAT/LC08/C02/T1_L2`, `LC09`) | 30 m / 16 天 | 反射率 `SR_B.* * 0.0000275 - 0.2`；地表温度 `ST_B10 * 0.00341802 + 149.0` (K) | `QA_PIXEL` Bits 0–5 全零（去填充/膨胀云/卷云/云/云影/雪）联合 `QA_RADSAT == 0` | **不可漏减 `-0.2`**；8/9 用 `SR_B5/B4`，5/7 用 `SR_B4/B3` |
| NASA HLS v2 (`NASA/HLS/HLSL30/v002`, `HLSS30/v002`) | 30 m NBAR / 2–4 天 | 已有官方 SR 语义（NBAR） | `Fmask` 剔除 Bits 1–4 (云/邻近/云影/雪)；保留 `hls_sensor` | **禁止套用 C2 的 `0.0000275/-0.2`**；近红外对齐 `L30.B5` ↔ `S30.B8A` |
| MODIS 植被 (`MODIS/061/MOD13Q1`) | 250 m / 16 天 | `NDVI`、`EVI` 均乘以 `0.0001` | 依据 `DetailedQA` 或 `SummaryQA` 剔低质量观测 | 250 m 不可直接与 10/30 m 混合统计且不说明尺度差 |
| MODIS 温度 (`MODIS/061/MOD11A2`) | 1000 m / 8 天 | `LST_Day_1km` / `LST_Night_1km` × `0.02` K（`- 273.15` 得 °C） | 按 `QC_Day` / `QC_Night` 过滤质量不良像元 | 8 天 1 km 统计不可直接对比 Landsat 30 m 瞬时 LST |
| Dynamic World (`GOOGLE/DYNAMICWORLD/V1`) | 10 m / 近实时 | 9 类概率波段 `water..snow_and_ice` 每像元和为 `1`；`label` 为众类编码 | 与 S2 L1C 时间/区域对齐；分类可用概率阈值判断置信度 | **`label` 是离散分类码，严禁 `.mean()`**；`label` 非真值 |
| ESA WorldCover (`ESA/WorldCover/v200`) | 10 m / 2021 年静态 | `Map` 波段离散分类码（`80` 永久水体等） | 静态年度分类产物，无常规多期云量过滤 | **严禁 `.mean()`**；聚合作图用 `mode()`，面积乘 `pixelArea()` |
| Sentinel-1 GRD (`COPERNICUS/S1_GRD`) | 10 m / 6–12 天 | 后向散射 `VV`/`VH` 已转 dB；`angle` 为椭球入射角 | 锁定 `IW` + `10m` + 单一升降轨 + 同相对轨道 | **严禁 dB 直接相除或重复转 dB**；比值须走线性域 `10^(dB/10)` |
| VIIRS 月度夜光 (`NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG`) | ~463 m / 月 | `avg_rad`（均值辐亮度 `nanoWatts/cm²/sr`） | 结合 `cf_cvg` 有效无云覆盖次数过滤极低采样像元 | 属于人类活动**代理指标**；须警惕火点、油气燃焰与高纬夏缺 |
| 人口与建筑暴露 (`WorldPop/GP/100m/pop`, `JRC/GHSL/P2023A/GHS_POP`, `open-buildings/v3/polygons`) | 100 m / 年至静态矢量 | 人口为模型估算网格数；建筑矢量含 `confidence` | 灾害层与暴露层独立，匹配最近期次；矢量先按 AOI 与置信度过滤 | 人口网格是估算而非实测人口；超大建筑集禁转单一几何 |
| 地形产品 (`COPERNICUS/DEM/GLO30_2024_1`, `USGS/SRTMGL1_003`) | 30 m / 静态基准 | 高程单位米（Copernicus 为 EGM2008 正高；SRTM 为 EGM96） | 静态基准无需日期与云量过滤 | 坡度/坡向对投影极其敏感，严禁在未指定投影/默认度投影下盲算 |

## 2. 尺度（Scale）与投影对齐核查清单

- **显式声明 `scale` 或 `crs`**：所有 `reduceRegion` / `reduceRegions` / `Export` 必须指定明确的 `scale`（米），切忌把地图交互缩放视作科学统计分辨率。
- **默认合成投影陷阱**：多景 `median()` / `mosaic()` 产物的默认投影往往退化为 **WGS84 1 度尺度**；若下游要做坡度、梯度的像素对齐分析，须保留源影像投影或传递明确 `crs`。
- **慎用强制 `.reproject()`**：全图大范围强制 `.reproject()` 会破坏金字塔多尺度加速并极易触发内存超限（OOM）；正确做法是在最终归约或导出环节传入 `crs` + `crsTransform`（需要严格逐像元原点对齐时，`crsTransform` 优于单独 `scale`）。
- **已退役资产警惕**：遇到 Landsat C1、`MODIS/006/*` 或老旧 JRC 版本时，先核查目录推荐的替代版本（如 `MODIS/061/*`、`COPERNICUS/DEM/GLO30_2024_1`）再生成代码。
