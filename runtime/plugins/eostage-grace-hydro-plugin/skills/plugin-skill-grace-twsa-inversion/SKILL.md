---
name: plugin-skill-grace-twsa-inversion
description: 基于 CSR/JPL Mascon 与 GLDAS 陆面模型分离区域地下水储量变化 (GWSA)
author: EOStage
license: GPL-3.0-or-later
version: 1.1.0
---

# GRACE/GRACE-FO 地下水储量反演规程

1. **Mascon 等效水高读取**：读取 CSR/JPL Mascon 等效水高网格（扣除 2004–2009 基线均值）并应用尺度因子恢复信号泄漏；
2. **陆面水文分量剥离**：从 GLDAS/ERA5-Land 提取同基线期土壤水分 ($\Delta\text{SMS}$)、雪水当量 ($\Delta\text{SWE}$)、冠层截留水 ($\Delta\text{CWS}$) 与地表水储量变化 ($\Delta\text{SWS}$)；
3. **地下水异常反演**：按 $\Delta\text{GWSA} = \Delta\text{TWSA} - (\Delta\text{SMS} + \Delta\text{SWE} + \Delta\text{CWS} + \Delta\text{SWS})$ 计算区域月度地下水储量变化及误差传递包络。
