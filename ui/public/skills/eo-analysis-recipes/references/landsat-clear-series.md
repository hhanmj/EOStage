关键词：Landsat 逐景晴空反射率序列、Landsat 8/9、QA_PIXEL、clear scene stack、LANDSAT/LC08/C02/T1_L2、LANDSAT/LC09/C02/T1_L2

## 1. 适用场景与输入要求

同一小区域和短时间段，输出按时间排序的逐景六光学波段堆栈与时间表，可用于单景核对。

## 2. 核心公式与参数避坑

每景应用 QA_PIXEL 0–5 和 QA_RADSAT；先缩放反射率，再叠成以场景索引命名的波段。

## 3. 完整可运行 Python 代码

```python
import ee


def clear_landsat_stack(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    bands = ["SR_B2", "SR_B3", "SR_B4", "SR_B5", "SR_B6", "SR_B7"]
    l8 = ee.ImageCollection("LANDSAT/LC08/C02/T1_L2")
    l9 = ee.ImageCollection("LANDSAT/LC09/C02/T1_L2")
    raw = l8.merge(l9).filterBounds(aoi).filterDate(start_date, end_date).sort("system:time_start")

    def clean(im):
        qa = im.select("QA_PIXEL").bitwiseAnd(63).eq(0).And(im.select("QA_RADSAT").eq(0))
        return im.select(bands).multiply(.0000275).subtract(.2).updateMask(qa)

    return raw.map(clean).toBands().clip(aoi).set("scene_times_ms", raw.aggregate_array("system:time_start"))
```

## 4. 结果质检标准

每景应有 6 个波段，原始掩膜保留。跨路径像元网格可不同，导出由调用方固定目标 CRS 和仿射变换；长序列按时段分批。
