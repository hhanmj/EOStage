关键词：昼间地表温度均值、MODIS/061/MOD11A1、Terra MODIS、LST_Day_1km、land surface temperature

## 1. 适用场景与输入要求

1000 m，1 天产品。按合成期起始日期选片，返回所选有效观测的均值。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

LST_Day_1km × 0.02 − 273.15，输出单位为 °C。用 QC_Day 筛选高质量观测。按期等权，跨月的合成期归入其起始日期所在窗口。 [MODIS/061/MOD11A1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD11A1)

## 3. 完整可运行 Python 代码

```python
import ee


def build_modis_day_lst(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("MODIS/061/MOD11A1")
              .filterBounds(aoi).filterDate(start_date, end_date))

    def decode(image):
        qc = image.select("QC_Day")
        valid = qc.bitwiseAnd(3).eq(0).And(qc.rightShift(6).bitwiseAnd(3).eq(0))
        raw = image.select("LST_Day_1km")
        return raw.updateMask(valid).multiply(0.02).subtract(273.15).rename("value")

    values = scenes.map(decode)
    count = values.count().rename("n_obs")
    result = values.mean()
    return result.rename("modis_day_lst").addBands(count).clip(aoi)
```

## 4. 结果质检标准

晴空样本均值受持续云覆盖影响，按月份检查 n_obs。逐像元检查观测次数，空值保留掩膜。
