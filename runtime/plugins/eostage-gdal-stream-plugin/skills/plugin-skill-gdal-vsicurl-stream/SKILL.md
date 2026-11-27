---
name: plugin-skill-gdal-vsicurl-stream
description: 免整景下载直接通过 GDAL /vsicurl/ 流式探测与窗口计算云端 COG / STAC 资产（坡度、山体阴影、TPI/TRI、NDVI/NDWI/NBR）。
author: EOStage
license: GPL-3.0-or-later
---

# GDAL `/vsicurl/` 云端 COG 在线流式读取与窗口计算规程

## 1. 适用场景
- 当用户需要查看或裁剪 Microsoft Planetary Computer、AWS Open Data（如 `copernicus-dem-30m`、Sentinel-2 L2A COG、Landsat Collection 2 L2 COG）或 WorldPop 云端栅格，但**不希望下载动辄数百 MB 的整景文件**时；
- 直接传入 `https://...` 或 `/vsicurl/https://...` URL，底层 GDAL 自动通过 HTTP `Range: bytes=...` 请求仅拉取 `bbox_wgs84` 覆盖的瓦片块（Block/Tile）并在内存中完成计算。

## 2. 标准调用链
1. **流式元数据探测（`application_gis_raster_inspect`）**：
   - 输入：`{ "source": "https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_N31_00_E117_00_DEM/Copernicus_DSM_COG_10_N31_00_E117_00_DEM.tif" }`
   - 输出：栅格宽高、波段数、数据类型、NoData、GeoTransform、坐标系与 WGS84 外接矩形。
2. **流式窗口处理与地形/光谱派生（`application_gis_raster_process`）**：
   - 支持算子（`operation`）：
     - `"clip"` / `"translate"`：按 `bbox_wgs84: [west, south, east, north]` 在线窗口裁剪并输出本地压缩 GeoTIFF；
     - `"slope"` / `"aspect"` / `"hillshade"` / `"tpi"` / `"tri"` / `"roughness"`：Horn 3×3 球面米制自适应梯度法原位计算 DEM 坡度、坡向、山体阴影（可设 `azimuth_deg` / `altitude_deg` / `z_factor`）、地形位置指数（TPI）、地形起伏度（TRI）与粗糙度；
     - `"ndvi"` / `"ndwi"` / `"nbr"` / `"band_math"`：支持单文件多波段或双远程 COG 源（`source` + `secondary_source`）在线窗口对齐计算归一化指数 `(A - B) / (A + B)`。
