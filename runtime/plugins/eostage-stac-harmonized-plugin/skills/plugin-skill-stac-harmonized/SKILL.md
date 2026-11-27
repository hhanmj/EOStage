---
name: plugin-skill-stac-harmonized
description: Sentinel-2 L2A 与 Landsat Collection 2 Level-2 跨传感器辐射定标对齐与谐波时间序列构建规程
author: EOStage
license: GPL-3.0-or-later
version: 2.1.0
---

# STAC 跨传感器表面反射率谐波合成规程

1. **辐射定标与基线偏移统一**：
   - Sentinel-2 L2A：核查 `PROCESSING_BASELINE`，对 2022-01-25 起 Baseline ≥ 04.00 产品按 $\rho = (\text{DN} + \text{BOA\_ADD\_OFFSET}) / 10000$ 扣除 `-1000` 偏移（注意避免在已消除偏移的 STAC 切片上重复扣除）；
   - Landsat Collection 2 Level-2：按 $\rho = \text{DN} \times 0.0000275 - 0.2$ 转换表面反射率，并应用窄波段至 Sentinel-2 MSI 等效光谱带通线性调整系数；
2. **双重质量掩膜**：使用 Landsat `QA_PIXEL` 位掩码与 Sentinel-2 `SCL` 场景分类层联合剔除云、卷云、云影与冰雪像元；
3. **谐波拟合与合成**：在每个像元上拟合一阶/二阶傅里叶谐波模型，输出无云季节合成影像与空间块交叉验证报告。
