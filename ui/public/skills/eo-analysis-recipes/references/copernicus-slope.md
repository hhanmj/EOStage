关键词：从分幅 DSM 计算坡度、COPERNICUS/DEM/GLO30_2024_1、Copernicus DSM、slope、EGM2008

## 1. 适用场景与输入要求

局地 AOI 及适合该区域的米制投影。名义 30 m，静态产品。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

采用 2024_1 集合。DEM 是相对 EGM2008 的正高，包含树冠和建筑。先拼有缓冲的瓦片，再在目标 30 m 网格上求梯度。 [COPERNICUS/DEM/GLO30_2024_1](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_DEM_GLO30_2024_1)

## 3. 完整可运行 Python 代码

```python
import ee


def surface_slope(aoi: ee.Geometry, metric_crs: str) -> ee.Image:
    tiles = (ee.ImageCollection("COPERNICUS/DEM/GLO30_2024_1")
             .filterBounds(aoi.buffer(120)).select("DEM"))
    grid = ee.Projection(metric_crs).atScale(30)
    surface = tiles.mosaic().setDefaultProjection(grid)
    return ee.Terrain.slope(surface).rename("slope_deg").clip(aoi)
```

## 4. 结果质检标准

坡度在 0–90°。检查拼接边缘和城市陡坡，后者可能对应建筑。与 GNSS 比对前统一垂直基准。
