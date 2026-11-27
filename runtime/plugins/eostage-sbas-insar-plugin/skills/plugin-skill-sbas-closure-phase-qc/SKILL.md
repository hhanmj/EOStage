---
name: plugin-skill-sbas-closure-phase-qc
description: 检验干涉对时空基线网连通性、计算三角环闭合相位残差并剔除失相干像对
author: EOStage
license: GPL-3.0-or-later
version: 1.3.0
---

# SBAS-InSAR 三角闭合相位与基线网质控规程

1. **时空基线连通图诊断**：构建时间基线 ($\Delta T \le 48\text{ d}$) 与空间垂直基线 ($|B_\perp| \le 150\text{ m}$) 连通图，检查全图代数连通度 $\lambda_2 > 0$，确保不存在孤立子网；
2. **三角闭合相位检验**：对任意干涉三元组 $(i, j, k)$ 逐像元计算三角闭合相位：
   $$\Delta\phi_{ijk} = \text{wrap}(\phi_{ij} + \phi_{jk} - \phi_{ik})$$
   识别相位解缠 $2\pi$ 整周跳变与土壤湿度/植被非弹性散射引起的系统性闭合残差；
3. **重加权最小二乘反演**：依据时间相干性 $\gamma_{\text{temp}} \ge 0.7$ 与闭合相位 RMS 建立方差-协方差矩阵，剔除强对流大气污染像对并输出毫米级年均形变速率场与累积位移序列。
