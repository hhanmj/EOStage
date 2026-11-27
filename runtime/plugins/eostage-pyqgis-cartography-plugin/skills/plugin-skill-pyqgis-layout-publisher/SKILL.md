---
name: plugin-skill-pyqgis-layout-publisher
description: 自动生成带发散色标、比例尺、指北针与 CRS 元数据标注的 QGIS 工程与 PDF 图件
author: EOStage
license: GPL-3.0-or-later
version: 1.4.0
---

# PyQGIS 专题地图自动排版规程

1. **投影坐标系核验**：检查输入栅格/矢量的投影坐标系（严禁在度为单位的地理坐标系 `EPSG:4326` 下直接绘制公制比例尺或计算面积）；
2. **色盲安全配色**：对形变速率场采用零值对称的发散色标（如 `RdBu` / `Vik`），对单调高程或降水场采用感知均匀色标（`Viridis` / `Cividis`）；
3. **出版级图面要素**：自动添加经纬网、比例尺、指北针、图例与数据血缘脚注，导出 300 DPI PDF/PNG。
