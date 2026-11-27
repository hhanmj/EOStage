关键词：晴空多波段中位数合成、median composite、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

同一季节或年份，B2/B3/B4/B8 为 10 m，SCL 为 20 m。至少一景相交。

## 2. 核心公式与参数避坑

保留 SCL 4、5、6，DN / 10000。按波段取有效样本中位数，另返回可见光红波段通过掩膜的场景数。

## 3. 完整可运行 Python 代码

```python
import ee


def clear_s2(aoi, start_date, end_date):
    def decode(image):
        scl = image.select("SCL")
        clear = scl.eq(4).Or(scl.eq(5)).Or(scl.eq(6))
        return (image.select("B.*").divide(10000).updateMask(clear)
                .copyProperties(image, ["system:time_start"]))
    return (ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED")
            .filterDate(start_date, end_date).filterBounds(aoi).map(decode))

def composite(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    clean = clear_s2(aoi, start_date, end_date).select(["B2", "B3", "B4", "B8"])
    grid = ee.Image(clean.first()).select("B4").projection()
    return clean.median().addBands(clean.select("B4").count().rename("scene_count")).setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

检查阴影、薄云和无观测边缘。波段中位数组合可能来自不同日期，相邻瓦片同日可计两景。
