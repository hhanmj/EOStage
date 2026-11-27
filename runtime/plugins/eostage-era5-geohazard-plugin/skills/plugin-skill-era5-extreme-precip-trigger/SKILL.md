---
name: plugin-skill-era5-extreme-precip-trigger
description: 基于前期有效降雨量 (API) 与 I-D 强度-历时曲线评估区域群发性滑坡气象危险性
author: EOStage
license: GPL-3.0-or-later
version: 2.0.0
---

# 极端降水与滑坡雨量阈值分析规程

1. **前期有效降雨量计算**：提取 ERA5-Land / GPM 小时级格点降水，按指数衰减系数 $k = 0.85$ 计算前 15 日前期有效降雨量 $\text{API}_{15} = \sum_{d=1}^{15} k^d P_d$；
2. **强度-历时 (I-D) 幂律阈值拟合**：在双对数坐标下拟合区域群发性滑坡临界降雨强度-历时曲线 $I = \alpha D^{-\beta}$；
3. **重现期与危险性耦合**：结合 GEV 广义极值分布重现期雨量与 InSAR 坡体形变速率场，输出降雨诱发滑坡超越概率空间分布。
