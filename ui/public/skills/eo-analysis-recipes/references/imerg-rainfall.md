关键词：半小时降水率积分、GPM、IMERG、NASA/GPM_L3/IMERG_V07、precipitation accumulation

## 1. 适用场景与输入要求

约 0.1° 网格。传入 UTC 起止日，覆盖完整半小时槽；结束日不含。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

每槽降水深 = 降水率(mm/h) × 0.5 h。先去除重复时刻，再累计；缺任一时段时屏蔽总量。 [NASA/GPM_L3/IMERG_V07](https://developers.google.com/earth-engine/datasets/catalog/NASA_GPM_L3_IMERG_V07)

## 3. 完整可运行 Python 代码

```python
import ee


def rainfall(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    start, end = ee.Date(start_date), ee.Date(end_date)
    expected = end.difference(start, "hour").multiply(2)
    raw = (ee.ImageCollection("NASA/GPM_L3/IMERG_V07")
           .filterBounds(aoi).filterDate(start, end).select("precipitation")
           .sort("status"))
    unique = ee.ImageCollection(raw.distinct("system:time_start"))
    rates = unique.map(lambda im: im.updateMask(im.gte(0)))
    count = rates.count().rename("valid_slots")
    total = rates.sum().multiply(0.5).updateMask(count.eq(expected))
    return total.rename("rain_mm").addBands(count).clip(aoi)
```

## 4. 结果质检标准

完整 UTC 日有 48 槽。零表示完整观测期无降水，累计量应非负。对照雨量站时统一日界。
