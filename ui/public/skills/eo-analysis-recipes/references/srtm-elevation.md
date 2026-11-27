关键词：SRTM 正高裁剪、DEM、USGS/SRTMGL1_003、NASA/NASADEM_HGT/001、EGM96

## 1. 适用场景与输入要求

SRTM V3 与 NASADEM 均基于 SRTM 雷达测量，约 1 弧秒，主体覆盖约 60°N 至 56°S。静态数据无需日期窗。不同版本的去空洞和重处理流程有差异。

## 2. 核心公式与参数避坑

高程单位米，垂直参考为 EGM96 正高。采用 Copernicus DEM 的 EGM2008 时需显式转换垂直基准再求差。 [SRTM V3](https://developers.google.com/earth-engine/datasets/catalog/USGS_SRTMGL1_003) · [NASADEM](https://developers.google.com/earth-engine/datasets/catalog/NASA_NASADEM_HGT_001)

## 3. 完整可运行 Python 代码

```python
import ee


def elevation(aoi: ee.Geometry, product: str = "srtm") -> ee.Image:
    catalog = {"srtm": "USGS/SRTMGL1_003", "nasadem": "NASA/NASADEM_HGT/001"}
    source = ee.Image(catalog[product])
    return source.select("elevation").rename("elevation_egm96_m").clip(aoi)
```

## 4. 结果质检标准

海平面附近和低地可出现负值。比较基准点，排查空洞填补、冠层与建筑物表面。
