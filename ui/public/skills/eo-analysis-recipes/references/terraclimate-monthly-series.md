关键词：月气候驱动区域序列、TerraClimate、IDAHO_EPSCOR/TERRACLIMATE、CASA

## 1. 适用场景与输入要求

输入连续整月窗与 AOI；约 4.6 km。每个输出要素代表一个月，有时间戳。

## 2. 核心公式与参数避坑

tmmn/tmmx ×0.1 得 °C，均值取两者中点；pr 为 mm/月，aet ×0.1 得 mm/月，srad ×0.1 得 W/m²，再乘当月天数和 0.0864 得 MJ/m²/月。区域均值按有效像元面积加权。 [官方波段表](https://developers.google.com/earth-engine/datasets/catalog/IDAHO_EPSCOR_TERRACLIMATE)

## 3. 完整可运行 Python 代码

```python
import ee


def climate_series(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.FeatureCollection:
    ic = ee.ImageCollection("IDAHO_EPSCOR/TERRACLIMATE").filterDate(start_date,end_date)
    def month(obj):
        im = ee.Image(obj)
        grid = im.select("pr").projection()
        d = im.date()
        days = d.advance(1,"month").difference(d,"day")
        x = ee.Image.cat([im.select("pr").rename("rain_mm"),im.select("aet").multiply(.1).rename("aet_mm"),
                          im.select("tmmn").add(im.select("tmmx")).multiply(.05).rename("temp_c"),
                          im.select("srad").multiply(.1).multiply(days).multiply(.0864).rename("solar_MJ_m2")])
        area = ee.Image.pixelArea().updateMask(x.mask().reduce(ee.Reducer.min())).rename("area")
        sums = x.multiply(area).addBands(area).reduceRegion(ee.Reducer.sum(),aoi,crs=grid,
            maxPixels=1e9)
        total = ee.Number(sums.get("area"))
        vals = x.bandNames().map(lambda k: ee.Algorithms.If(total.gt(0),ee.Number(sums.get(k)).divide(total),None))
        return ee.Feature(None,ee.Dictionary.fromLists(x.bandNames(),vals)).set("system:time_start",d.millis())
    return ee.FeatureCollection(ic.sort("system:time_start").toList(ic.size()).map(month))
```

## 4. 结果质检标准

检查月份连续、单位与雨量非负。只在有数据的陆地 ROI 运行；多年合成先按月份分组。
