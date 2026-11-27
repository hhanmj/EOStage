关键词：月度夜光观测加权辐亮度、VIIRS DNB、NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG

## 1. 适用场景与输入要求

约 464 m，月合成，传入按整月对齐的时间窗。

## 2. 核心公式与参数避坑

以 cf_cvg 为有效观测数，输出 Σ(avg_rad × cf_cvg)/Σcf_cvg。没有覆盖的月不作零光照。 [官方目录](https://developers.google.com/earth-engine/datasets/catalog/NOAA_VIIRS_DNB_MONTHLY_V1_VCMSLCFG)

## 3. 完整可运行 Python 代码

```python
import ee


def night_lights(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    def weighted(im):
        w = im.select("cf_cvg").updateMask(im.select("avg_rad").mask())
        return im.select("avg_rad").multiply(w).rename("weighted").addBands(w).updateMask(w.gt(0))
    totals = (ee.ImageCollection("NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG")
              .filterDate(start_date, end_date).filterBounds(aoi).map(weighted).sum())
    count = totals.select("cf_cvg")
    return totals.select("weighted").divide(count).rename("radiance").addBands(count).clip(aoi)
```

## 4. 结果质检标准

单位为 nW cm⁻² sr⁻¹，背景可略负。检查观测数、火点、渔船和极昼缺测，不把亮度直接解释为人口。
