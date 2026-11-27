关键词：同轨 SAR 事件前后变化、Sentinel-1、COPERNICUS/S1_GRD、SAR backscatter change、洪水、滑坡

## 1. 适用场景与输入要求

输入同轨向、同相对轨道事件前后两个互不重叠的日期窗口。使用 IW、10 m、VV。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

对比两个窗口的 dB 中位数。后期减前期为对数功率比；负变化是开阔水面候选信号，城市、湿土和山地可呈不同响应。 [COPERNICUS/S1_GRD](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S1_GRD)

## 3. 完整可运行 Python 代码

```python
import ee


def radar_change(aoi: ee.Geometry, start_date: str, end_date: str,
                 after_start: str, after_end: str, orbit: int, direction: str) -> ee.Image:
    base = (ee.ImageCollection("COPERNICUS/S1_GRD").filterBounds(aoi)
            .filter(ee.Filter.eq("instrumentMode", "IW"))
            .filter(ee.Filter.eq("resolution_meters", 10))
            .filter(ee.Filter.eq("relativeOrbitNumber_start", orbit))
            .filter(ee.Filter.eq("orbitProperties_pass", direction))
            .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VV"))
            .select("VV"))
    before = base.filterDate(start_date, end_date)
    after = base.filterDate(after_start, after_end)
    delta = after.median().subtract(before.median()).rename("delta_vv_db")
    return (delta.addBands(before.count().rename("n_before"))
            .addBands(after.count().rename("n_after")).clip(aoi))
```

## 4. 结果质检标准

检查两个窗口共有覆盖、雷达阴影及入射角。用同期水文或滑坡清单验证事件归因，按地区标定阈值。
