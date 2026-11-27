关键词：年度分区人口汇总、WorldPop/GP/100m/R2025A/v1/pop、population count

## 1. 适用场景与输入要求

国家分片年度人口，R2025A 提供 2015–2030 年。输入有面积的 AOI，输出估计人口（人）。

## 2. 核心公式与参数避坑

按年份选取空间相交国家分片，共用原生格网镶嵌后求和。禁止把人口计数再乘像元面积。早期发布版与新版的历史年份并不完全相同。 [官方目录](https://developers.google.com/earth-engine/datasets/catalog/WorldPop_GP_100m_R2025A_v1_pop)

## 3. 完整可运行 Python 代码

```python
import ee


def population(aoi: ee.Geometry, year: int) -> ee.Dictionary:
    if not 2015 <= year <= 2030:
        raise ValueError("year outside R2025A coverage")
    tiles = (ee.ImageCollection("WorldPop/GP/100m/R2025A/v1/pop")
             .filter(ee.Filter.eq("year", year)).filterBounds(aoi).select("population"))
    grid = ee.Image(tiles.first()).projection()
    return tiles.mosaic().reduceRegion(ee.Reducer.sum(), aoi, crs=grid,
        maxPixels=1e10, tileScale=4).set("year", year)
```

## 4. 结果质检标准

仅在有覆盖的 AOI 运行。留意国界边缘、跨日界线、预测年份；面总量允许出现小数。
