关键词：二分像元植被覆盖度、FVC、fractional vegetation cover、MODIS/061/MOD13Q1、NDVI

## 1. 适用场景与输入要求

已校准、已掩膜的单波段 NDVI，以及从同传感器同物候期参考样地确定的裸土和全植被端元。继承输入分辨率。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

FVC = (NDVI − NDVIsoil)/(NDVIveg − NDVIsoil)。固定跨期可比端元，输出裁至 [0,1] 同时标记越界。 [MODIS/061/MOD13Q1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD13Q1)

## 3. 完整可运行 Python 代码

```python
import ee


def fractional_cover(aoi: ee.Geometry, ndvi: ee.Image, soil: float,
                     vegetation: float) -> ee.Image:
    if not -1 <= soil < vegetation <= 1:
        raise ValueError("invalid NDVI endmembers")
    x = ndvi.select(0).subtract(soil).divide(vegetation - soil)
    outside = x.lt(0).Or(x.gt(1)).rename("outside_endmembers")
    return x.clamp(0, 1).rename("fvc").addBands(outside).clip(aoi)
```

## 4. 结果质检标准

FVC 在 0–1。统计端元越界比例，集中在水域或农田收割期时重查土地覆盖掩膜与参考样地。
