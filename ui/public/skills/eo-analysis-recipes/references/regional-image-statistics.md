关键词：多区连续栅格统计、zonal statistics、reduceRegions、ee.FeatureCollection

## 1. 适用场景与输入要求

输入单波段连续量和有稳定 ID 的多边形集合，指定统计投影、尺度（米）；日期由输入影像定义。

## 2. 核心公式与参数避坑

同一遍聚合求均值和有效像元数。区域按 AOI 交集限界，保留区划属性。

## 3. 完整可运行 Python 代码

```python
import ee


def regional_stats(aoi: ee.Geometry, image: ee.Image, zones: ee.FeatureCollection,
                   band: str, crs: str, scale: float) -> ee.FeatureCollection:
    if scale <= 0:
        raise ValueError("scale must be positive")
    regions = zones.filterBounds(aoi).map(lambda f: f.setGeometry(f.geometry().intersection(aoi, 1)))
    reducer = ee.Reducer.mean().combine(ee.Reducer.count(), sharedInputs=True)
    return image.select(band).reduceRegions(collection=regions, reducer=reducer, crs=crs, scale=scale, tileScale=4)
```

## 4. 结果质检标准

检查空区和有效像元数；小区域边界权重与像元中心计数口径可不同。密度转总量时需另乘面积。
