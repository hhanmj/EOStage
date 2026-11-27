关键词：MODIS 时段植被指数、MODIS/061/MOD13Q1、Terra MODIS、NDVI、normalized difference vegetation index

## 1. 适用场景与输入要求

250 m，16 天产品。按合成期起始日期选片，返回所选有效观测的均值。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

NDVI × 0.0001，输出单位为 无量纲。用 SummaryQA 筛选高质量观测。按期等权，跨月的合成期归入其起始日期所在窗口。 [MODIS/061/MOD13Q1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD13Q1)

## 3. 完整可运行 Python 代码

```python
import ee


def build_modis_ndvi(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("MODIS/061/MOD13Q1")
              .filterBounds(aoi).filterDate(start_date, end_date))

    def decode(image):
        qc = image.select("SummaryQA")
        valid = qc.eq(0)
        raw = image.select("NDVI")
        return raw.updateMask(valid).multiply(0.0001).rename("value")

    values = scenes.map(decode)
    count = values.count().rename("n_obs")
    result = values.mean()
    return result.rename("modis_ndvi").addBands(count).clip(aoi)
```

## 4. 结果质检标准

有效产品范围 -0.2 至 1。逐像元检查观测次数，空值保留掩膜。
