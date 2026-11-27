---
name: plugin-skill-external-gis-bridge
description: 协调 EOStage 内置 GDAL/PROJ 引擎与本机 RustGIS / GeoLibre / QGIS / ArcGIS Pro 工具链执行栅格重投影、镶嵌、公制缓冲区与 .qgs 工程交付。
author: EOStage
license: GPL-3.0-or-later
---

# EOStage 外部 GIS 工具链协同规程

## 1. 先查能力再分流（`gis_toolchains_status`）
- 调用 `gis_toolchains_status` 获取内置 GDAL/PROJ 与本机 `rustgis`、`geolibre`、`qgis`、`arcgis_pro` 的就绪状态与版本号。

## 2. 内置优先的科学精度处理（`gis_local_process`）
- 栅格重投影（`reproject`）、拼接镶嵌（`mosaic`）、裁切（`clip`）与 `GTiff`/`ENVI`(`.hdr`) 格式转换：默认保持原始像元数据类型（`preserve_dtype: true`），严禁静默降采样或压缩为 8 位整数。
- 矢量缓冲区（`buffer`）：使用 `domain: "vector"` 与公制距离 `distance_m`（自动投影到局部等距方位 `+proj=aeqd` 执行真实米级缓冲），严禁使用经纬度度数充当米级距离。
- 矢量格式互转（`convert`）：支持 GeoJSON、Shapefile (`.shp/.shx/.dbf/.prj`)、KML/KMZ 之间的无损转换。

## 3. 外部专业软件协同与交付（`gis_external_execute`）
- 无头算子执行：`action: "list_algorithms"` / `"run_algorithm"`，输出文件自动限制在受控输出目录并做脱敏与字节非空校验。
- 一键交付工程：`action: "deliver_project"` 自动写入带真实空间外接矩形的 `eostage_catalog.geojson` 与规范 XML 格式的 `eostage_delivery.qgs` 工程文件。
