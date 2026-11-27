关键词：地形湿度指数、TWI、topographic wetness index、DEM

## 1. 适用场景与输入要求

输入来自水文校正 DEM 的单位等高线长度汇水面积 a（m²/m）及同网格坡度（度）。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

TWI = ln(a/tan β)。把 β 转为弧度。若汇流栅格是像元数，应根据流向算法将面积换算为单位等高线宽度的汇水面积。

## 3. 完整可运行 Python 代码

```python
import math
import ee


def wetness(aoi: ee.Geometry, area_per_width: ee.Image,
            slope_deg: ee.Image) -> ee.Image:
    tangent = slope_deg.multiply(math.pi / 180).tan()
    valid = area_per_width.gt(0).And(slope_deg.gt(0)).And(slope_deg.lt(90))
    return area_per_width.divide(tangent).log().updateMask(valid).rename("twi").clip(aoi)
```

## 4. 结果质检标准

谷底常高于分水岭。检查平地屏蔽比例、洼地填平区和流域边界截断；TWI 可以为负。
