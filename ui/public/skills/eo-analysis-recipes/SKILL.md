---
name: eo-analysis-recipes
description: 当任务涉及 Earth Engine Python 光学或 SAR 遥感、物候与 RSEI、时序和空间统计、跨源融合、生态建模或本地 GeoTIFF 时，按数据集和分析目标加载对应配方。
---

## 核心数据源与波段速查

| 官方数据源 | 波段与物理量 |
|---|---|
| [COPERNICUS/S2_SR_HARMONIZED](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S2_SR_HARMONIZED) | B3 绿、B4 红、B8 近红外（10 m）；B8A、B11、SCL（20 m）。光谱 DN / 10000。 |
| [LANDSAT/LC08/C02/T1_L2](https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC08_C02_T1_L2) | OLI 红/近红外 SR_B4/B5；SR_B* × 0.0000275 − 0.2；ST_B10 × 0.00341802 + 149.0 K。 |
| [LANDSAT/LC09/C02/T1_L2](https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC09_C02_T1_L2) | 与 LC08 的 C2 L2 命名和缩放一致；光学、热波段分别应用质量掩膜。 |
| [COPERNICUS/DEM/GLO30_2024_1](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_DEM_GLO30_2024_1) | DEM 为 EGM2008 正高（m），包括植被和建筑。 |
| [COPERNICUS/S1_GRD](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S1_GRD) | VV/VH 为 dB，计算功率比先转线性。 |
| [ESA/WorldCover/v200](https://developers.google.com/earth-engine/datasets/catalog/ESA_WorldCover_v200) | Map 为 2021 土地覆盖类别，80 为永久水体。 |
| [MODIS/061/MOD13Q1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD13Q1) | NDVI × 0.0001，16 天。 |
| [MODIS/061/MOD11A1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD11A1) | LST_Day_1km × 0.02 K，日产品。 |
| [MODIS/061/MOD15A2H](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD15A2H) | Lai_500m × 0.1；Fpar_500m × 0.01。 |
| [MODIS/061/MOD17A2H](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD17A2H) | Gpp × 0.0001 kg C/m²/合成期。 |
| [NASA/GPM_L3/IMERG_V07](https://developers.google.com/earth-engine/datasets/catalog/NASA_GPM_L3_IMERG_V07) | precipitation 为 mm/h，每槽 30 分钟。 |
| [JRC/GHSL/P2023A/GHS_BUILT_S](https://developers.google.com/earth-engine/datasets/catalog/JRC_GHSL_P2023A_GHS_BUILT_S) | built_surface 为 m²/像元。 |
| [IDAHO_EPSCOR/TERRACLIMATE](https://developers.google.com/earth-engine/datasets/catalog/IDAHO_EPSCOR_TERRACLIMATE) | pr 为 mm/月，aet × 0.1 为 mm/月。 |
| [NASA/HLS/HLSL30/v002](https://developers.google.com/earth-engine/datasets/catalog/NASA_HLS_HLSL30_v002) | 与 HLSS30 v002 统一 30 m 地表反射率，Fmask 按位检查。 |
| [MODIS/061/MOD17A3HGF](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD17A3HGF) | Npp DN × 0.0001 kg C/m²/年（500 m）。 |
| [MODIS/061/MCD43A4](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MCD43A4) | NBAR × 0.0001（500 m），用配套质量位筛反演。 |
| [NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG](https://developers.google.com/earth-engine/datasets/catalog/NOAA_VIIRS_DNB_MONTHLY_V1_VCMSLCFG) | avg_rad 为 nW/cm²/sr，cf_cvg 为有效覆盖计数。 |
| [WorldPop/GP/100m/R2025A/v1/pop](https://developers.google.com/earth-engine/datasets/catalog/WorldPop_GP_100m_R2025A_v1_pop) | population 为像元人数，按年与国家瓦片筛选。 |
| [USGS/SRTMGL1_003](https://developers.google.com/earth-engine/datasets/catalog/USGS_SRTMGL1_003) | elevation 为 EGM96 正高（m），1 弧秒。 |
| [GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED](https://developers.google.com/earth-engine/datasets/catalog/GOOGLE_CLOUD_SCORE_PLUS_V1_S2_HARMONIZED) | cs / cs_cdf（[0, 1] 晴空质量评分），经 linkCollection 挂接至 S2_SR_HARMONIZED。 |
| [GOOGLE/DYNAMICWORLD/V1](https://developers.google.com/earth-engine/datasets/catalog/GOOGLE_DYNAMICWORLD/V1) | 10 m 九类近实时土地覆盖概率（water/trees/grass/crops/built 等）与 label 众数。 |
| [COPERNICUS/S5P/OFFL/L3_NO2](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S5P/OFFL/L3_NO2) | 对流层 NO2 柱浓度（mol/m²，常乘 1e6 转 µmol/m²），配 cloud_fraction 去云并保留轻度随机负噪。 |
| [ECMWF/ERA5_LAND/DAILY_AGGR](https://developers.google.com/earth-engine/datasets/catalog/ECMWF_ERA5_LAND_DAILY_AGGR) | 约 9 km 逐日再分析：temperature_2m（−273.15 转 °C），total_precipitation_sum（×1000 转 mm）。 |

> **分工提示**：本技能共收录 **99 项** 即用代码配方（83 项 GEE Python + 16 项本地 GeoTIFF）；若需服务端架构原则、性能调优（避坑 `.clip`/`.reproject`、`tileScale`、`ee.Join` 选型）与 Olofsson 面积无偏校正，请配合读取 `gee-textbook-cookbook`。

## 科学红线与常见翻车点

- GEE 配方由宿主完成授权和 ee.Initialize。时间区间左闭右开；调用方先确认时间窗有场景，再检查 AOI 内有效像元和各期覆盖率。
- 在服务端用 map、reduce 构建计算图。固定波段或固定模型因子的 Python 循环可用于构图；逐像元、逐景计算避免 getInfo。
- Landsat QA_PIXEL 位 0–5 全零可保守排除填充、膨胀云、卷云、云、云影和雪；结合 QA_RADSAT。早期 TM/ETM+ 须重新映射波段和热通道。
- 跨传感器比较先统一辐射基线、光谱响应、空间支持尺度和物候窗口。旧 TOA 回归系数需要按 SR 重新标定。
- [已停用需替换] 历史遗留的 LANDSAT/LE7_TOA_5YEAR 五年合成没有等价的 C2 L2 单景替代；重建同季合成时需记录观测数和 ETM+ 条带缺口。
- Sentinel-2 使用已调和的 SR，DN / 10000。按 SCL 或 Cloud Score+ 去云，作物与水体掩膜需结合地类和季节复核。
- 光学合成保留质量掩膜；分类用最近邻，连续量按目标物理量选择插值、面积加权均值或总量守恒聚合。
- 时序合成报告有效样本，瓦片重叠时按同日去重。跨年的 PCA、端元和回归应用固定参考，比较前验证口径。
- 统计明确 CRS 与格网。面积按 pixelArea 或合适的等面积投影计算；静态 DEM 和年度分类图按产品期次处理。
- 本地配方逐窗返回数组和位置，继承首幅或指定参考栅格的变换和 CRS。缺测用 NaN 表示，零值保留物理含义。

## references/ 配方路由

| 具体问题 | 独立配方 |
|---|---|
| 植被绿度中位数 | [s2-ndvi.md](references/s2-ndvi.md) |
| 开阔水体指数 | [s2-ndwi.md](references/s2-ndwi.md) |
| 河流水体浑浊度代理 | [s2-ndti.md](references/s2-ndti.md) |
| 冠层水分指数 | [s2-ndmi.md](references/s2-ndmi.md) |
| Landsat 地表反射率合成 | [landsat-surface-reflectance.md](references/landsat-surface-reflectance.md) |
| 晴空地表温度合成 | [landsat-surface-temperature.md](references/landsat-surface-temperature.md) |
| MODIS 时段植被指数 | [modis-ndvi.md](references/modis-ndvi.md) |
| 昼间地表温度均值 | [modis-day-lst.md](references/modis-day-lst.md) |
| 叶面积指数均值 | [modis-lai.md](references/modis-lai.md) |
| 冠层吸收光合有效辐射比例 | [modis-fpar.md](references/modis-fpar.md) |
| 年度总初级生产力 | [modis-annual-gpp.md](references/modis-annual-gpp.md) |
| 按行政区汇总地类面积 | [worldcover-class-area.md](references/worldcover-class-area.md) |
| 分层抽取地类像元 | [worldcover-stratified-sample.md](references/worldcover-stratified-sample.md) |
| 从分幅 DSM 计算坡度 | [copernicus-slope.md](references/copernicus-slope.md) |
| 栅格配对差值 | [raster-difference.md](references/raster-difference.md) |
| 土壤调节植被指数 | [raster-savi.md](references/raster-savi.md) |
| 有效观测时段均值 | [raster-mean.md](references/raster-mean.md) |
| 双侧时序插值填洞 | [raster-linear-fill.md](references/raster-linear-fill.md) |
| 逐像元线性趋势 | [raster-trend.md](references/raster-trend.md) |
| 配对时序误差 | [raster-rmse.md](references/raster-rmse.md) |
| 温度单位换算 | [raster-kelvin-to-celsius.md](references/raster-kelvin-to-celsius.md) |
| 逐像元配对相关 | [raster-pearson.md](references/raster-pearson.md) |
| 半小时降水率积分 | [imerg-rainfall.md](references/imerg-rainfall.md) |
| 月降水与实际蒸散水量差 | [terraclimate-monthly-water.md](references/terraclimate-monthly-water.md) |
| 双极化雷达植被指数 | [s1-radar-vegetation.md](references/s1-radar-vegetation.md) |
| 建成表面面积变化 | [ghsl-built-change.md](references/ghsl-built-change.md) |
| 二分像元植被覆盖度 | [fractional-vegetation.md](references/fractional-vegetation.md) |
| 同物候基准植被健康指数 | [vegetation-health.md](references/vegetation-health.md) |
| 多年平均坡面侵蚀估算 | [usle-soil-loss.md](references/usle-soil-loss.md) |
| 月尺度光能利用率模型 | [casa-monthly-npp.md](references/casa-monthly-npp.md) |
| 地形湿度指数 | [wetness-index.md](references/wetness-index.md) |
| 空间独立验证的土地覆盖分类 | [random-forest-land-cover.md](references/random-forest-land-cover.md) |
| 年序列稳健变化率 | [sen-slope.md](references/sen-slope.md) |
| 同轨 SAR 事件前后变化 | [s1-event-change.md](references/s1-event-change.md) |
| 对齐到参考栅格 | [raster-align.md](references/raster-align.md) |
| 参考范围归一化 | [raster-minmax.md](references/raster-minmax.md) |
| 保留缺测的线性去趋势 | [raster-detrend.md](references/raster-detrend.md) |
| 历史同期均值填洞 | [raster-historical-fill.md](references/raster-historical-fill.md) |
| 谐波拟合填补季节序列 | [raster-harmonic-gap-fill.md](references/raster-harmonic-gap-fill.md) |
| 局部多项式平滑时序 | [raster-savgol.md](references/raster-savgol.md) |
| 年际变异系数 | [raster-coefficient-variation.md](references/raster-coefficient-variation.md) |
| 分层地理探测器 q 统计量 | [geodetector-factor.md](references/geodetector-factor.md) |
| 汇水面积与坡度估计水流强度 | [stream-power-index.md](references/stream-power-index.md) |
| 按变量类型统一栅格 | [align-image-grid.md](references/align-image-grid.md) |
| 年度净初级生产力 | [annual-modis-npp.md](references/annual-modis-npp.md) |
| 年度植被指数与 FVC 汇总 | [annual-ndvi-fvc.md](references/annual-ndvi-fvc.md) |
| 逐像元数组线性回归 | [array-temporal-regression.md](references/array-temporal-regression.md) |
| 可重复分批提取大量点值 | [batched-point-sampling.md](references/batched-point-sampling.md) |
| 缨帽分量生物物理组成指数 | [bci-tasseled-cap.md](references/bci-tasseled-cap.md) |
| 随机森林地上生物量回归 | [biomass-rf-regression.md](references/biomass-rf-regression.md) |
| 按月汇总 MODIS 质量合格观测 | [calendar-modis-products.md](references/calendar-modis-products.md) |
| 样地标定的碳密度线性反演 | [carbon-linear-regression.md](references/carbon-linear-regression.md) |
| 时空筛选与场景元数据 | [collection-inventory.md](references/collection-inventory.md) |
| 按期读取专题资产 | [dated-asset-selector.md](references/dated-asset-selector.md) |
| 逐像元全连接网络推断 | [dense-pixel-inference.md](references/dense-pixel-inference.md) |
| 逐景多站点取值 | [geographic-point-series.md](references/geographic-point-series.md) |
| 桌面分块拉取网格 | [geotiff-tile-plan.md](references/geotiff-tile-plan.md) |
| 协调反射率多时相特征 | [hls-seasonal-features.md](references/hls-seasonal-features.md) |
| 多尺度重标极差 Hurst 指数 | [hurst-rescaled-range.md](references/hurst-rescaled-range.md) |
| Landsat 逐景晴空反射率序列 | [landsat-clear-series.md](references/landsat-clear-series.md) |
| 跨代 Landsat 植被覆盖度 | [landsat-historical-fvc.md](references/landsat-historical-fvc.md) |
| 核归一化植被指数 | [landsat-kndvi.md](references/landsat-kndvi.md) |
| 真假彩色显示波段 | [landsat-rgb-render.md](references/landsat-rgb-render.md) |
| Landsat 陆地生态指数 PCA | [landsat-rsei.md](references/landsat-rsei.md) |
| 带粗网格残差约束的地表温度细化 | [lst-thermal-sharpening.md](references/lst-thermal-sharpening.md) |
| 地表干度 NDBSI | [modis-ndbsi.md](references/modis-ndbsi.md) |
| MODIS 公共网格生态指数 | [modis-rsei.md](references/modis-rsei.md) |
| 固定同月基准植被条件 | [monthly-vegetation-condition.md](references/monthly-vegetation-condition.md) |
| 固定色标时间序列帧 | [ndvi-animation-frames.md](references/ndvi-animation-frames.md) |
| OLI 地表反射率缨帽分量 | [oli-tasseled-cap.md](references/oli-tasseled-cap.md) |
| 按时间键配对像元相关 | [pixel-time-correlation.md](references/pixel-time-correlation.md) |
| PM2.5 浓度区域时序 | [pm25-regional-series.md](references/pm25-regional-series.md) |
| 红边叶绿素代理指数 | [red-edge-chlorophyll.md](references/red-edge-chlorophyll.md) |
| 跨源校准填补影像空洞 | [reflectance-gap-transfer.md](references/reflectance-gap-transfer.md) |
| 单配对邻域反射率变化转移 | [reflectance-spatiotemporal-fusion.md](references/reflectance-spatiotemporal-fusion.md) |
| 多区连续栅格统计 | [regional-image-statistics.md](references/regional-image-statistics.md) |
| 同轨雷达洪水候选区 | [s1-inundation.md](references/s1-inundation.md) |
| 同轨坡地雷达异常筛查 | [s1-landslide-anomaly.md](references/s1-landslide-anomaly.md) |
| 样地标定叶绿素或冠层水分 | [s2-calibrated-canopy-trait.md](references/s2-calibrated-canopy-trait.md) |
| 晴空多波段中位数合成 | [s2-clear-composite.md](references/s2-clear-composite.md) |
| 单季 EVI 阈值物候 | [s2-evi-phenology.md](references/s2-evi-phenology.md) |
| 样本标定的水体面积分数 | [s2-water-fraction.md](references/s2-water-fraction.md) |
| 光谱阈值布尔掩膜 | [spectral-threshold-mask.md](references/spectral-threshold-mask.md) |
| SRTM 正高裁剪 | [srtm-elevation.md](references/srtm-elevation.md) |
| 月气候驱动区域序列 | [terraclimate-monthly-series.md](references/terraclimate-monthly-series.md) |
| 分块调用已部署的影像模型 | [vertex-image-inference.md](references/vertex-image-inference.md) |
| 月度夜光观测加权辐亮度 | [viirs-night-radiance.md](references/viirs-night-radiance.md) |
| 空间留出样本下的分类器比较 | [worldcover-classifiers.md](references/worldcover-classifiers.md) |
| 年度分区人口汇总 | [worldpop-population.md](references/worldpop-population.md) |
| Cloud Score+ 现代去云合成 | [s2-cloud-score-plus.md](references/s2-cloud-score-plus.md) |
| Dynamic World 10m 概率与地类变化 | [dynamic-world-lulc.md](references/dynamic-world-lulc.md) |
| 林火前后 dNBR / RdNBR 烈度分级 | [s2-wildfire-dnbr.md](references/s2-wildfire-dnbr.md) |
| Otsu 双峰直方图自动阈值掩膜 | [otsu-auto-threshold.md](references/otsu-auto-threshold.md) |
| Sentinel-5P 对流层 NO2 柱浓度 | [s5p-tropospheric-no2.md](references/s5p-tropospheric-no2.md) |
| ERA5-Land 逐月气温与降水距平 | [era5-land-climate-anomalies.md](references/era5-land-climate-anomalies.md) |
| LandTrendr 年度时间分割扰动年代 | [landtrendr-disturbance.md](references/landtrendr-disturbance.md) |
| CCDC 密集时序谐波断点监测 | [ccdc-harmonic-breakpoints.md](references/ccdc-harmonic-breakpoints.md) |
| Sentinel-1 线性功率域 Lee 相干斑滤波 | [s1-speckle-lee-filter.md](references/s1-speckle-lee-filter.md) |
| 服务端跨传感器集合时间空间配对 (`ee.Join`) | [ee-temporal-collection-join.md](references/ee-temporal-collection-join.md) |
