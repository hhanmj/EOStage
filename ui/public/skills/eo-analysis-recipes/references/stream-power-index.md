关键词：汇水面积与坡度估计水流强度、SPI、stream power index、DEM

## 1. 适用场景与输入要求

输入同网格单位等高线长度汇水面积（m²/m）和坡度（度）。流域上游不能被 AOI 提前裁断。 宿主需初始化 earthengine-api。

## 2. 核心公式与参数避坑

SPI = a × tan(β)。本卡使用未取对数的定义。汇流累积应在完整上游域完成后再裁剪。

## 3. 完整可运行 Python 代码

```python
import math
import ee


def stream_power(aoi: ee.Geometry, area_per_width: ee.Image,
                 slope_deg: ee.Image) -> ee.Image:
    slope = slope_deg.multiply(math.pi / 180).tan()
    valid = area_per_width.gte(0).And(slope_deg.gte(0)).And(slope_deg.lt(90))
    return area_per_width.multiply(slope).updateMask(valid).rename("spi").clip(aoi)
```

## 4. 结果质检标准

SPI 非负，平地为零。检查河道沿线异常尖峰及坡度单位；该指数为地形代理量。
