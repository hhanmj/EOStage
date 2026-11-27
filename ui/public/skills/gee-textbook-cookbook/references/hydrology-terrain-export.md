# 水文水体动态、地形因子与 EOStage 分块导出 (Hydrology, Terrain & Chunked Export)

## 1. MNDWI 水体指数与 Otsu / 固定阈值对比 + JRC 水体频率交叉核验

```python
import ee

def extract_water_and_flood(s2_pre: ee.Image, s2_post: ee.Image, aoi: ee.Geometry):
    # Xu (2006) MNDWI = (Green - SWIR1) / (Green + SWIR1)
    mndwi_pre = s2_pre.normalizedDifference(["B3", "B11"]).rename("MNDWI_PRE")
    mndwi_post = s2_post.normalizedDifference(["B3", "B11"]).rename("MNDWI_POST")

    water_pre = mndwi_pre.gt(0.0)
    water_post = mndwi_post.gt(0.0)
    # 剔除山体阴影误识别：坡度 > 15 度区域不作为静水淹没区
    dem = ee.Image("COPERNICUS/DEM/GLO30").select("DEM")
    slope = ee.Terrain.slope(dem)
    new_flood = water_post.And(water_pre.Not()).And(slope.lt(15.0)).selfMask()

    # JRC Global Surface Water 永久水体基线过滤
    jrc = ee.Image("JRC/GSW1_4/GlobalSurfaceWater").select("occurrence")
    seasonal_or_new = new_flood.updateMask(jrc.unmask(0).lt(80))
    area_sqm = seasonal_or_new.multiply(ee.Image.pixelArea()).reduceRegion(
        reducer=ee.Reducer.sum(), geometry=aoi, scale=10, maxPixels=1e10
    )
    return seasonal_or_new, area_sqm
```

## 2. 在 EOStage 中安全落盘 GEE 栅格

1. 先调用 `gee_check_availability` 获取有效景数、时间覆盖率与建议分块数（确保单次输出不超过安全像元上限）。
2. 当研究区过大触发分块提示时，调用 `gee_chunked_download` 自动切片导出，并通过本地 GDAL `gdalbuildvrt` 无缝拼接为单幅配准 GeoTIFF，同步保留 `.provenance.json` 溯源记录。
