关键词：同轨雷达洪水候选区、Sentinel-1、flood extent、COPERNICUS/S1_GRD、JRC/GSW1_4/GlobalSurfaceWater

## 1. 适用场景与输入要求

前后日期窗均要有同一相对轨道和通行方向的 IW VV 数据，输入坡度（度）。有效像元尺度依波束和地形而异。

## 2. 核心公式与参数避坑

积水候选需要 VV 下降并达到事件后暗散射阈值，屏蔽常年水体与陡坡。默认值仅用于当地验证起点。

## 3. 完整可运行 Python 代码

```python
import ee


def flood_candidates(aoi: ee.Geometry, start_date: str, event_date: str, end_date: str,
                     orbit: int, direction: str, slope: ee.Image,
                     drop_db: float = 3, water_db: float = -16) -> ee.Image:
    base = (ee.ImageCollection("COPERNICUS/S1_GRD").filterBounds(aoi)
            .filter(ee.Filter.eq("instrumentMode", "IW"))
            .filter(ee.Filter.eq("orbitProperties_pass", direction))
            .filter(ee.Filter.eq("relativeOrbitNumber_start", orbit))
            .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VV")).select("VV"))
    before = base.filterDate(start_date, event_date)
    prior = before.median()
    after = base.filterDate(event_date, end_date).median()
    delta = after.subtract(prior).rename("delta_db")
    permanent = ee.Image("JRC/GSW1_4/GlobalSurfaceWater").select("seasonality").unmask(0).eq(12)
    flooded = delta.lte(-drop_db).And(after.lte(water_db)).And(slope.lte(5)).And(permanent.Not())
    return flooded.rename("flood_candidate").addBands(delta).setDefaultProjection(
        ee.Image(before.first()).projection()).clip(aoi)
```

## 4. 结果质检标准

用现场水位或清晰的近同期光学图验证面积。山影、风浪、植被淹水与城市双程散射可产生误检或漏检。
