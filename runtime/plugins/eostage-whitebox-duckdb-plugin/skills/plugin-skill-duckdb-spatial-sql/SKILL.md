---
name: plugin-skill-duckdb-spatial-sql
description: 使用本地 DuckDB + Spatial 扩展 (runtime/duckdb-cli/duckdb.exe) 执行矢量空间连接、缓冲区统计与 GeoParquet 在线查询。
author: EOStage
license: GPL-3.0-or-later
---

# DuckDB Spatial 空间 SQL 与 GeoParquet 分析规程

## 1. 调用方式（`application_open_geodata_query` -> `dataset: "duckdb_sql"`）
- 系统自动挂载 `runtime/duckdb-cli/ext/spatial.duckdb_extension` 并以 JSON 数组返回查询结果。
- 示例：统计落入高变形区多边形内的建筑物或道路长度：
  ```sql
  SELECT
    b.class,
    COUNT(*) AS feature_count,
    SUM(ST_Area_Spheroid(b.geom)) AS total_area_m2
  FROM ST_Read('buildings.shp') b
  JOIN ST_Read('insar_subsidence_zone.geojson') z
    ON ST_Intersects(b.geom, z.geom)
  GROUP BY b.class;
  ```
