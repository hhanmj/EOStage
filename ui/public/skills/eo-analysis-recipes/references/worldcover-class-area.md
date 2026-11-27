关键词：按行政区汇总地类面积、ESA/WorldCover/v200、WorldCover、zonal statistics、pixelArea

## 1. 适用场景与输入要求

输入面要素集，每个面保留自己的标识。2021 年年度产品，约 10 m。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

面积波段以 km² 为单位，按 Map 类别分组求和。保留原始分类网格的仿射变换，避免格网平移。 [ESA/WorldCover/v200](https://developers.google.com/earth-engine/datasets/catalog/ESA_WorldCover_v200)

## 3. 完整可运行 Python 代码

```python
import ee


def class_area(zones: ee.FeatureCollection) -> ee.FeatureCollection:
    cover = ee.ImageCollection("ESA/WorldCover/v200").first().select("Map")
    grid = cover.projection()
    areas = ee.Image.pixelArea().divide(1e6).rename("km2").addBands(cover)
    reducer = ee.Reducer.sum().group(groupField=1, groupName="class")
    return areas.reduceRegions(collection=zones, reducer=reducer,
                               crs=grid,
                               tileScale=4)
```

## 4. 结果质检标准

groups 中各类 sum 均非负。类别 80 为永久水体。各类之和应接近有效覆盖面积；重叠分区会重复计入。
