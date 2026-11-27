---
name: plugin-skill-open-geodata-lbs
description: 零配置获取民政部行政区划/地名、阿里云 DataV GeoJSON 边界、Open-Meteo ERA5 气象、Overpass OSM 矢量、WorldPop 人口、多镜像开放 DEM 瓦片，并执行 WGS84/GCJ02/BD09/CGCS2000 坐标互转。
author: EOStage
license: GPL-3.0-or-later
---

# 零配置开放地理数据与国内坐标系互转规程

## 1. 零密钥开放地理数据底座（`application_open_geodata_query`）
- **`open_dem_tiles`**：按 `bbox: [west, south, east, north]` 与 `dem_product: "cop30" | "cop90" | "nasadem" | "aw3d30"` 解析 1°×1° 瓦片在 AWS Open Data、OpenTopography S3 与微软 Planetary Computer 的多镜像下载地址及 `/vsicurl/` 在线流式路径；
- **`mca_xzqh` / `mca_placename`**：查询中华人民共和国民政部（`dmfw.mca.gov.cn`）最新行政区划代码与标准地名坐标；
- **`datav_bound`**：按 6 位 `code`（如 `"340100"` 合肥市）下载阿里云 DataV 行政边界 GeoJSON（支持 `full_children: true` 含下辖区县）；
- **`open_meteo_era5`**：按 `lat`, `lng`, `start_date`, `end_date` 拉取 Open-Meteo ERA5 逐日最高/最低/平均气温、累计降水与最大风速，用于 InSAR 形变-降雨相关性分析；
- **`overpass_osm`**：按 `bbox` 与 `osm_tag`（如 `"waterway"`, `"highway"`, `"building"`）双镜像拉取 OpenStreetMap 矢量要素；
- **`worldpop`**：按 `iso3`（如 `"CHN"`）与 `year`（2000–2020）生成 WorldPop 1km 人口网格 COG 与 `/vsicurl/` 流式路径。

## 2. 国内互联网坐标系高精度互转（`application_lbs_coord_transform`）
- 支持 `wgs84`、`cgcs2000`、`gcj02`（高德/腾讯火星坐标）、`bd09`（百度经纬度坐标）任意双向批量转换；
- 逆转换采用 6 步不动点迭代（`gcj02 -> wgs84` 与 `bd09 -> gcj02`），往返闭合残差 `< 1e-10°`（优于 0.01 毫米）。
