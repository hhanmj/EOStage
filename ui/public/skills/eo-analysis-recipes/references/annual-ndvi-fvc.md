关键词：年度植被指数与 FVC 汇总、NDVI、FVC、annual mean、MODIS/061/MOD13Q1

## 1. 适用场景与输入要求

输入指定年份及固定土壤、冠层 NDVI 端元，MODIS 原始采样 250 m、16 天。年度总 NPP 走年度 NPP 卡，独立报告物理单位。

## 2. 核心公式与参数避坑

按合成起始日归年，逐期先用固定端元转换 FVC 并限制在 [0,1]，再汇总有效期。

## 3. 完整可运行 Python 代码

```python
import ee


def annual_vegetation(aoi: ee.Geometry, year: int, soil: float, canopy: float) -> ee.Image:
    if not -1 <= soil < canopy <= 1:
        raise ValueError("invalid NDVI endmembers")
    t0 = ee.Date.fromYMD(year, 1, 1)
    raw = ee.ImageCollection("MODIS/061/MOD13Q1").filterBounds(aoi).filterDate(t0, t0.advance(1, "year"))

    def metrics(im):
        stored = im.select("NDVI")
        clear = im.select("SummaryQA").eq(0).And(stored.gte(-2000)).And(stored.lte(10000))
        ndvi = stored.multiply(.0001).updateMask(clear).rename("ndvi")
        cover = ndvi.subtract(soil).divide(canopy - soil).clamp(0, 1).rename("fvc")
        return ndvi.addBands(cover)

    series = raw.map(metrics)
    grid = ee.Image(raw.first()).select("NDVI").projection()
    return series.mean().addBands(series.select("ndvi").count().rename("n")).setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

FVC∈[0,1]。检查跨年份端元和有效期数是否一致，以及长期云或雪造成的季节偏样。
