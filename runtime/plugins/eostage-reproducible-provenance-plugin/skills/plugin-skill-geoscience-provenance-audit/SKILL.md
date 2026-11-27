---
name: plugin-skill-geoscience-provenance-audit
description: 核验传感器产品 ID、轨道精密星历版本、DEM 基准面与坐标转换参数链完整性
author: EOStage
license: GPL-3.0-or-later
version: 1.2.0
---

# 地学实验数据血缘审计规程

1. **原始观测与辅助数据核查**：逐项核查输入影像 Scene ID、采集起止时间 (UTC)、产品处理基线、精密轨道星历版本（POEORB/RESORB）与参考 DEM 垂直基准面（EGM96/EGM2008/WGS84 椭球高）；
2. **空间坐标转换链审计**：检查全部重投影与重采样步骤是否显式记录源/目标 EPSG 编码、大地水准面格网文件 SHA-256 及重采样核（双线性/三次卷积/最邻近）；
3. **可复现归档导出**：生成符合学术期刊 Data Availability 要求的方法学段落与 JSON-LD / RO-Crate 数据血缘清单。
