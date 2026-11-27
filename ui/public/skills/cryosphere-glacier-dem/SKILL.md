---
name: cryosphere-glacier-dem
description: "Glacier surface elevation change, 3D DEM coregistration, and geodetic mass balance analysis using xDEM, Copernicus DEM / NASADEM / GEM-15, and vertical datum alignment. Use for cryosphere thinning, landslide volume estimation, or multi-epoch DEM differencing."
---

# 冰川变化与多期 DEM 三维配准规程 (Cryosphere & xDEM Mass Balance)

面向冰川表面高程变化（$dh/dt$）、滑坡土石方量测算及历史地形图与现代 SAR/激光雷达 DEM（Copernicus GLO-30、AW3D30、SRTM、ICESat-2）对比：**严禁未做水平配准与垂直高程基准统一直接相减**。

核心规程：
1. **统一垂直高程基准**：先确认两期 DEM 同为 WGS84 椭球高或 EGM2008 正高（EOStage 转换工作台支持 `dem-datum-conversion`）；
2. **稳定基岩掩膜**：利用冰川轮廓清单（RGI 7.0）剔除冰雪、水体与高变化区，仅在稳定无植被基岩区拟合空间配准矩阵；
3. **三维配准（Nuth & Kääb 2011）**：求解 $(dx, dy, dz)$ 偏移、along-track 抖动消除及地形坡度/曲率系统误差改正；
4. **密度转换与误差传播**：计算大地测量物质平衡（通常冰体转换因子 $850 \pm 60\,\text{kg/m}^3$，Huss 2013），利用空间自相关变差函数（Variogram）严谨估算区域均值标准误。

详尽 `xDEM` 可运行代码流程见 [references/xdem-glacier-mass-balance.md](references/xdem-glacier-mass-balance.md)。
