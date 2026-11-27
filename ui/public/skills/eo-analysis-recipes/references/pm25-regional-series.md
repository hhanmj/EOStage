关键词：PM2.5 浓度区域时序、PM2.5、annual PM2.5、面均值、GEE ImageCollection

## 1. 适用场景与输入要求

输入获授权的、已完成单位换算的年度 PM2.5 浓度集合，波段 pm25、system:time_start，单位 µg/m³；示例原资产属于社区产品，必须由调用方确认访问与版本。

## 2. 核心公式与参数避坑

每期算 Σ(浓度×有效面积)/Σ有效面积；另报有效面积。区域面积平均和人口暴露加权平均有不同含义。

## 3. 完整可运行 Python 代码

```python
import ee


def pm25_series(aoi: ee.Geometry, start_date: str, end_date: str, annual: ee.ImageCollection) -> ee.FeatureCollection:
    selected = annual.filterDate(start_date,end_date).filterBounds(aoi)
    def summarize(obj):
        im = ee.Image(obj).select("pm25")
        area = ee.Image.pixelArea().updateMask(im.mask()).rename("m2")
        x = im.multiply(area).rename("mass_area").addBands(area)
        vals = x.reduceRegion(ee.Reducer.sum(),aoi,crs=im.projection(),
                              maxPixels=1e9)
        m2 = ee.Number(ee.Algorithms.If(vals.get("m2"),vals.get("m2"),0))
        avg = ee.Algorithms.If(m2.gt(0),ee.Number(vals.get("mass_area")).divide(m2),None)
        return ee.Feature(None,{"time_ms":im.date().millis(),"mean_ug_m3":avg,"valid_m2":m2})
    return ee.FeatureCollection(selected.toList(selected.size()).map(summarize))
```

## 4. 结果质检标准

区域均值应落在有效像元最小和最大浓度之间。不得将卫星气溶胶光学厚度或 NO₂ 柱浓度当作地表 PM2.5。
