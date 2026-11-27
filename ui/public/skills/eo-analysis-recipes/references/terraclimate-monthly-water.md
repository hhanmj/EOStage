关键词：月降水与实际蒸散水量差、TerraClimate、IDAHO_EPSCOR/TERRACLIMATE、P−AET、water balance

## 1. 适用场景与输入要求

约 4.6 km 月度陆地产品；传入年、月。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

pr 为月降水 mm，aet × 0.1 为月实际蒸散 mm。差值反映两项收支，解释径流时还需考虑储水变化。 [IDAHO_EPSCOR/TERRACLIMATE](https://developers.google.com/earth-engine/datasets/catalog/IDAHO_EPSCOR_TERRACLIMATE)

## 3. 完整可运行 Python 代码

```python
import ee


def monthly_balance(aoi: ee.Geometry, year: int, month: int) -> ee.Image:
    start = ee.Date.fromYMD(year, month, 1)
    im = (ee.ImageCollection("IDAHO_EPSCOR/TERRACLIMATE")
          .filterDate(start, start.advance(1, "month")).first())
    rain = im.select("pr")
    aet = im.select("aet").multiply(0.1)
    return rain.subtract(aet).rename("p_minus_aet_mm").clip(aoi)
```

## 4. 结果质检标准

山区按海拔带检查偏差。负值可表示蒸散消耗储水；与站点比较时统一月份与单位。
