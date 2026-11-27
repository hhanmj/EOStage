关键词：冠层吸收光合有效辐射比例、MODIS/061/MOD15A2H、Terra MODIS、Fpar_500m、fraction of absorbed PAR

## 1. 适用场景与输入要求

500 m，8 天产品。按合成期起始日期选片，返回所选有效观测的均值。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

Fpar_500m × 0.01，输出单位为 比例。用 FparLai_QC 筛选高质量观测。按期等权，跨月的合成期归入其起始日期所在窗口。 [MODIS/061/MOD15A2H](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD15A2H)

## 3. 完整可运行 Python 代码

```python
import ee


def build_modis_fpar(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("MODIS/061/MOD15A2H")
              .filterBounds(aoi).filterDate(start_date, end_date))

    def decode(image):
        qc = image.select("FparLai_QC")
        valid = qc.bitwiseAnd(1).eq(0).And(qc.rightShift(3).bitwiseAnd(3).eq(0))
        raw = image.select("Fpar_500m")
        return raw.updateMask(valid).multiply(0.01).rename("value")

    values = scenes.map(decode)
    count = values.count().rename("n_obs")
    result = values.mean()
    return result.rename("modis_fpar").addBands(count).clip(aoi)
```

## 4. 结果质检标准

FPAR 在 0–1。逐像元检查观测次数，空值保留掩膜。
