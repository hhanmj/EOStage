关键词：时空筛选与场景元数据、ImageCollection、server-side map、metadata、system:time_start

## 1. 适用场景与输入要求

输入有日期和空间范围的影像集合 ID；示例默认 Landsat 8 C2。

## 2. 核心公式与参数避坑

返回集合计数和最多 limit 条按时间排序的元数据；先在服务端筛选，空集合返回空列表。

## 3. 完整可运行 Python 代码

```python
import ee


def inventory(aoi: ee.Geometry, start_date: str, end_date: str,
              collection_id: str = "LANDSAT/LC08/C02/T1_L2", limit: int = 10) -> ee.Dictionary:
    if not 1 <= limit <= 100:
        raise ValueError("limit must be 1 to 100")
    scenes = ee.ImageCollection(collection_id).filterBounds(aoi).filterDate(start_date, end_date)
    def describe(obj):
        im = ee.Image(obj)
        return ee.Dictionary({"id": im.get("system:index"), "time_ms": im.date().millis(),
                              "bands": im.bandNames(), "first_band_crs": im.select(0).projection().crs()})
    return ee.Dictionary({"count": scenes.size(), "scenes": scenes.sort("system:time_start").limit(limit).toList(limit).map(describe)})
```

## 4. 结果质检标准

先核对波段名、时间和投影，再选产品与分析尺度；有相交场景仍可能全云。
