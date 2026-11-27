关键词：固定色标时间序列帧、MODIS/061/MOD13Q1、NDVI、animation、palette

## 1. 适用场景与输入要求

短时段 MOD13Q1，250 m、16 天。输出按时间排列的多波段 RGB 影像，供桌面逐帧解码。

## 2. 核心公式与参数避坑

SummaryQA=0 后 NDVI ×0.0001；固定区间和色标，输出属性记录日期、色标与取值范围。

## 3. 完整可运行 Python 代码

```python
import ee


def animation_frames(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    palette = ["7b3294", "f7f7f7", "a6dba0", "008837"]
    series = ee.ImageCollection("MODIS/061/MOD13Q1").filterBounds(aoi).filterDate(start_date, end_date).sort("system:time_start")
    def frame(im):
        nd = im.select("NDVI").multiply(.0001).updateMask(im.select("SummaryQA").eq(0))
        return nd.visualize(min=-.2, max=.9, palette=palette).set("system:index", im.date().format("yyyyMMdd"))
    return series.map(frame).toBands().clip(aoi).set({"dates_ms": series.aggregate_array("system:time_start"),
                                                       "palette": palette, "ndvi_range": [-.2, .9]})
```

## 4. 结果质检标准

每期三波段，检查日期顺序、统一拉伸和掩膜透明度；toBands 的 5000 波段上限要求长序列分批。
