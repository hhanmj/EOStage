---
name: ocean-tides-bathymetry
description: "Ocean tides, coastal sea-level correction, and global bathymetry workflow using DGFI-TUM EOT20, pyTMD, GEBCO 2026, and NOAA ETOPO 2022. Use for coastal InSAR tide loading correction, intertidal topography, or marine bathymetry masking."
---

# 海洋潮汐与全球海底地形规程 (TUM EOT20 / pyTMD / GEBCO / ETOPO)

本技能包整合地球与环境科学领域的权威潮汐模型与全球无缝地形/水深数据集：
- **`science.tum.eot20.v1`**：慕尼黑工大（DGFI-TUM）经验海洋潮汐模型 EOT20（$0.125^\circ$，涵盖 M2/S2/K1/O1 等 17 个主分潮与浅水倍潮）；
- **`science.gebco.bathymetry-2026.v1`**：GEBCO 全球海洋与陆地无缝高程/水深网格（15 arc-second ≈ 450m）；
- **`science.noaa.etopo-2022.v1`**：NOAA NCEI ETOPO 2022 基岩与冰面全球无缝高程水深模型；
- **软件栈**：`pyTMD`（支持在设置中心 -> 运行环境与工具中一键确认安装到 Conda 环境）。

详尽建模公式、OTL 潮汐负荷改正与 `pyTMD` 代码示例见 [references/eot20-pytmd-gebco.md](references/eot20-pytmd-gebco.md)。
