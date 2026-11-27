---
name: plugin-skill-mapgis-compat
author: EOStage
license: GPL-3.0-or-later
description: 原生探测与转换 MapGIS 6.x 点 (.wt)、线 (.wl)、区 (.wp) 及明码矢量文件为标准 Shapefile (.shp) 或 GeoJSON。
---

# MapGIS 6.x 存量地质矢量探测与转换规程

> 互操作说明：本转换能力基于公开格式样本独立实现，仅用于存量科研数据的格式互操作与开放格式迁移，与相关商业软件无官方关联。

## 1. 格式支持范围
- **矢量文件**：支持常见 MapGIS 6.x 点（`.wt`）、线（`.wl`）、区（`.wp`）及明码文本交换文件（`WPNT` / `WPLN` / `WPOL`）的几何与属性读取。
- **目标开放格式**：导出标准 ESRI Shapefile（`.shp` / `.shx` / `.dbf` / `.prj`）或 GeoJSON `FeatureCollection`。

## 2. 标准调用链
1. **探测文件结构（`application_mapgis_inspect`）**：
   - 输入：`{ "path": "D:/geodata/faults.wl" }`
   - 输出：几何类型（`Point` / `LineString` / `Polygon`）、要素数量、外接矩形 `bbox`、坐标系参数与属性表字段列表。
2. **执行转换（`application_mapgis_convert`）**：
   - 输入：`{ "input_path": "D:/geodata/faults.wl", "output_path": "D:/geodata/faults.shp", "scale": 1.0 }`
   - 输出：根据输出后缀生成完整 Shapefile 四件套或 GeoJSON 文件。
