关键词：逐景多站点取值、sample time series、Landsat LST、LANDSAT/LC08/C02/T1_L2

## 1. 适用场景与输入要求

输入具有稳定 site_id 的点和已掩膜、具时间戳的单景集合；提供波段名和米制采样尺度。输出维持站点×场景结构。

## 2. 核心公式与参数避坑

在每景的波段投影上对每个点取 first。云掩膜产生空值；保留时间、场景和站点 ID，便于区分同日过境。

## 3. 完整可运行 Python 代码

```python
import ee


def station_series(aoi: ee.Geometry, start_date: str, end_date: str,
                   scenes: ee.ImageCollection, points: ee.FeatureCollection, band: str, scale: float) -> ee.FeatureCollection:
    stations = points.filterBounds(aoi).select(["site_id"])
    subset = scenes.filterDate(start_date,end_date).filterBounds(aoi).select(band)
    def extract(obj):
        im = ee.Image(obj)
        rows = im.rename("value").reduceRegions(collection=stations,reducer=ee.Reducer.first().setOutputs(["value"]),
                                               scale=scale,crs=im.projection(),tileScale=4)
        return rows.map(lambda f: f.set({"time_ms":im.date().millis(),"scene":im.get("system:index")}))
    return ee.FeatureCollection(subset.toList(subset.size()).map(extract)).flatten()

def landsat_station_lst(aoi: ee.Geometry, start_date: str, end_date: str,
                        points: ee.FeatureCollection) -> ee.FeatureCollection:
    def decode(im):
        clear = im.select("QA_PIXEL").bitwiseAnd(63).eq(0)
        clear = clear.And(im.select("QA_RADSAT").eq(0))
        lst = im.select("ST_B10").multiply(.00341802).add(149).rename("lst_k")
        return (lst.updateMask(clear).copyProperties(im, ["system:time_start"])
                .set("system:index", im.get("system:index")))

    l8 = ee.ImageCollection("LANDSAT/LC08/C02/T1_L2")
    l9 = ee.ImageCollection("LANDSAT/LC09/C02/T1_L2")
    thermal = l8.merge(l9).filterBounds(aoi).filterDate(start_date, end_date)
    thermal = thermal.filter(ee.Filter.eq("PROCESSING_LEVEL", "L2SP")).map(decode)
    return station_series(aoi, start_date, end_date, thermal, points, "lst_k", 30)
```

## 4. 结果质检标准

短时间段分批执行以控制站点×景数。检查站点重复、有效率和温度单位。Landsat 每景须先按 QA_PIXEL/QA_RADSAT 去云、仅用 L2SP，ST_B10 ×0.00341802+149 得 K。
