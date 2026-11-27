---
name: sar-insar-textbook
description: "Synthetic Aperture Radar (SAR) & Interferometric SAR (InSAR) engineering textbook skill: Doppler/LOS imaging geometry, layover/shadow masks, D-InSAR baseline and ambiguity height, 2D phase unwrapping (SNAPHU/MCF), SBAS/PS time-series inversion (MintPy), and POEORB orbit + GACOS/ERA5 tropospheric mitigation."
---

# SAR 物理机理与 InSAR 时序形变教材攻略 (SAR & InSAR Textbook)

本技能包将雷达成像几何、双通 D-InSAR 相位分解、相位解缠、SBAS / PS 时序反演及外部误差改正拆解为 4 份工程规程；请先调用 `skill_read(skill_id="sar-insar-textbook", query=...)` 或按需读取 `references/*.md`：

| 子文档路径 | 核心理论与工程规程 | 对应 EOStage / 本机生态工具 |
| --- | --- | --- |
| `references/sar-geometry-radiometry.md` | 右侧斜距成像几何、叠掩（Layover）/透视收缩（Foreshortening）/阴影判别角、$\sigma^0 / \gamma^0$ 辐射地形校正（RTC）、Multi-look 与斑点滤波（Refined Lee） | `search_start(sar)`、`geodata_query(cop30)`、`nisar_subset_preflight` |
| `references/dinsar-phase-unwrapping.md` | 干涉相位五分量物理方程、高度模糊数 $h_a$ 与垂直基线选对规则（$B_\perp < 150\,\text{m}$）、相干性 $\gamma$ 估计、Goldstein 自适应滤波与 SNAPHU / MCF 最小费用流解缠 | `ISCE2`、`Sentinel-1 SLC / Burst`、`NISAR RSLC / GUNW` |
| `references/sbas-ps-timeseries.md` | 短时空基线集（SBAS）小奇异值分解与时间连通性检查、PS 振幅离差指数（$D_A < 0.25$）选点、MintPy 相位闭合检验（Phase Closure）、参考点稳定性与 LOS 速度转垂向 | `MintPy`、`NISAR GUNW`、`OPERA DISP-S1` |
| `references/atmospheric-orbit-error-mitigation.md` | 精密轨道 `AUX_POEORB` / `NISAR_OE` 下载与残余轨道坡面拟合、GACOS ZTD / ERA5 垂直分层延迟线性与非线性校正、IONEX 电离层 TEC 改正与 GNSS 外部校准 | `POEORB`、`GACOS`、`ERA5`、`NGL GNSS` |

## InSAR 数据准备强约束

1. **精密轨道必配**：Sentinel-1 采集后 20 天内使用 `AUX_RESORB`，满 20 天必须切换 `AUX_POEORB`；在 EOStage 中下载 Sentinel-1 SLC 时系统支持同步附带轨道与 GACOS 申请。
2. **垂直高程基准统一**：Copernicus DEM (EGM2008 正高) 与 SRTM (EGM96 正高) 在做干涉地理编码与地形相位扣除时**必须转换至 WGS84 椭球高**（请配合技能 `/dem-datum-conversion` 或 DEM 转换工作台）。
3. **NISAR HDF5 HTTP Range 裁剪**：在获取 NISAR `RSLC / GSLC / GCOV / GUNW` 产品时，优先使用 `nisar_subset_preflight` + `nisar_create_subset_task` 按 AOI 流式裁剪 `*_SUBSET.h5`，切勿误导用户整景下载。
