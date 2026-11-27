关键词：年度总初级生产力、MODIS/061/MOD17A2H、Terra MODIS、Gpp、gross primary productivity

## 1. 适用场景与输入要求

500 m，8 天产品。传入完整日历年。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

Gpp × 0.0001，输出单位为 kg C/m²。用 Psn_QC 筛选高质量观测。每期 GPP 已是累计量。切勿对年内 8 天产品直接求均值，否则无法得到年总量，应按各期累计量求和；年末短合成期直接计入。 [MODIS/061/MOD17A2H](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD17A2H)

## 3. 完整可运行 Python 代码

```python
import ee


def build_modis_annual_gpp(aoi: ee.Geometry, year: int) -> ee.Image:
    start_date = ee.Date.fromYMD(year, 1, 1)
    end_date = start_date.advance(1, "year")
    scenes = (ee.ImageCollection("MODIS/061/MOD17A2H")
              .filterBounds(aoi).filterDate(start_date, end_date))

    def decode(image):
        qc = image.select("Psn_QC")
        valid = qc.bitwiseAnd(1).eq(0).And(qc.rightShift(3).bitwiseAnd(3).eq(0))
        raw = image.select("Gpp")
        return raw.updateMask(valid).multiply(0.0001).rename("value")

    values = scenes.map(decode)
    count = values.count().rename("n_obs")
    result = values.sum().updateMask(count.eq(46))
    return result.rename("modis_annual_gpp").addBands(count).clip(aoi)
```

## 4. 结果质检标准

仅输出 46 个合成期均有效的像元；检查 n_obs 分布评估云区缺失。逐像元检查观测次数，空值保留掩膜。
