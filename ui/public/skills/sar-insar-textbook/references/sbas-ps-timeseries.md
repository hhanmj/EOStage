# SBAS / PS-InSAR 时序网络反演与 MintPy 质控 (Time-Series Inversion)

## 1. 干涉网络连通性与冗余基线设计

- **单一连通分量（Connected Network）**：构建短时空基线图（例如连续 3 个邻近时相配对 $t_i \leftrightarrow t_{i+1}, t_{i+2}, t_{i+3}$ + 年度季节闭合对），使用 `MintPy` / `plot_network.py` 确认全序列无孤立子网络（`n_components == 1`）。
- **三角闭合相位检验（Triplet Phase Closure）**：
  对于无明显物理不连续的三元干涉对 $(i,j),(j,k),(i,k)$，计算闭合残差：
  $$C_{ijk} = W\{\phi_{ij} + \phi_{jk} - \phi_{ik}\}$$
  若大量像元呈现非零系统性闭合相位偏置，说明短时相土壤水分/介电常数渐变（Soil Moisture / Fading Signal）显著，**不可盲目增加超短基线加权而导致累计漂移（Cumulative Bias）**；宜拉长部分季节连接对时间跨度（De Zan 等规则）。

## 2. PS 永久散射体识别与参考点选取

- **振幅离差指数（Amplitude Dispersion Index）**：$D_A = \sigma_A / \mu_A$，要求至少 20–25 景均匀采样影像，$D_A < 0.25$ 作为强稳定散射体候选。
- **参考点原则**：选择**高时空相干性（$\bar{\gamma} > 0.85$）且地质构造稳定**的基岩出露区、大型桥梁锚固点或同步 GNSS 连续观测站，严禁选在第四纪冲积平原中心、地下水超采漏斗内或季节水体周边。

## 3. MintPy `smallbaselineApp.py` 标准质控门限

- `temporalCoherence > 0.7` 作为时序形变最终有效像元掩膜阈值；
- 反演速度后检查残差 RMS（`residuePhaseRMS`）及参考点零基准验证；
- 最终形变速率需注明 LOS 方向符号约定（通常正值代表靠近卫星/隆升，负值代表远离卫星/沉降）。
