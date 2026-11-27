---
name: landcover-change-clcd
description: "Long-term 30m land cover transition matrix and hazard exposure analysis using Wuhan University CLCD (China Land Cover Dataset v1.0.4, 1985-present) combined with ESA WorldCover 10m and InSAR deformation fields."
---

# 中国年度 30m 土地覆盖变化与灾害暴露度规程 (WHU CLCD + InSAR Exposure)

基于地球与环境科学领域重点收录的权威数据 **`science.whu.clcd.v1`**（武汉大学 Yang & Huang 团队中国逐年 30m 土地覆盖数据集 CLCD v1.0.4，1985–2023+）：

## CLCD 官方 9 级分类编码

| 编码 | 英文标签 | 中文地类 | 遥感与形变耦合重点 |
| --- | --- | --- | --- |
| `1` | Cropland | 耕地 | 季节土壤湿度影响 InSAR 相位闭合；华北漏斗区农业抽采 |
| `2` | Forest | 森林 | C 波段体散射失相干，宜换 L/S 波段 (ALOS-2 / NISAR) |
| `3` | Shrub | 灌木 | 山地边缘生态恢复或干旱退化指示 |
| `4` | Grassland | 草地 | 冻土退化与热喀斯特沉降监测 |
| `5` | Water | 水体 | 相干性近零，解缠阶段必须严格掩膜 |
| `6` | Snow/Ice | 冰雪 | 剔除表面融雪位移或交由 `cryosphere-glacier-dem` |
| `7` | Barren | 裸地 | 高相干天然散射体，适合作 SBAS/PS 稳定参考候选 |
| `8` | Impervious | 不透水面（城镇建筑/交通） | **城镇地面沉降暴露核心目标**：统计形变 $> 20\,\text{mm/yr}$ 覆盖建成区面积 |
| `9` | Wetland | 湿地 | 沿海/内陆湿地水位与沉降耦合 |

转移矩阵计算代码与 InSAR 建成区沉降暴露统计见 [references/whu-clcd-transition-matrix.md](references/whu-clcd-transition-matrix.md)。
