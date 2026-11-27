关键词：叶面积指数均值、MODIS/061/MOD15A2H、Terra MODIS、Lai_500m、leaf area index

## 1. 适用场景与输入要求

500 m，8 天产品。按合成期起始日期选片，返回所选有效观测的均值。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

Lai_500m × 0.1，输出单位为 m²/m²。用 FparLai_QC 筛选高质量观测。按期等权，跨月的合成期归入其起始日期所在窗口。 [MODIS/061/MOD15A2H](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD15A2H)

## 3. 完整可运行 Python 代码

```python
import ee


def build_modis_lai(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("MODIS/061/MOD15A2H")
              .filterBounds(aoi).filterDate(start_date, end_date))

    def decode(image):
        qc = image.select("FparLai_QC")
        valid = qc.bitwiseAnd(1).eq(0).And(qc.rightShift(3).bitwiseAnd(3).eq(0))
        raw = image.select("Lai_500m")
        return raw.updateMask(valid).multiply(0.1).rename("value")

    values = scenes.map(decode)
    count = values.count().rename("n_obs")
    result = values.mean()
    return result.rename("modis_lai").addBands(count).clip(aoi)
```

## 4. 结果质检标准

LAI 在 0–10。逐像元检查观测次数，空值保留掩膜。
