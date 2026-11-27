关键词：核归一化植被指数、kernel NDVI、kNDVI、LANDSAT/LC08/C02/T1_L2

## 1. 适用场景与输入要求

Landsat 8 的光学 30 m 数据；日期区间应覆盖目标物候期。

## 2. 核心公式与参数避坑

采用自适应带宽近似 kNDVI=tanh(NDVI²)。逐景先缩放、去云，分母接近零时屏蔽。

## 3. 完整可运行 Python 代码

```python
import ee


def kernel_ndvi(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = ee.ImageCollection("LANDSAT/LC08/C02/T1_L2").filterBounds(aoi).filterDate(start_date, end_date)
    def transform(image):
        red, nir = [image.select(b).multiply(.0000275).subtract(.2) for b in ["SR_B4", "SR_B5"]]
        den = nir.add(red)
        clear = image.select("QA_PIXEL").bitwiseAnd(63).eq(0).And(image.select("QA_RADSAT").eq(0))
        return nir.subtract(red).divide(den).pow(2).tanh().updateMask(clear.And(red.gte(0)).And(nir.gte(0)).And(den.gt(0)))
    return scenes.map(transform).median().rename("kndvi").clip(aoi)
```

## 4. 结果质检标准

非负反射率下范围为 0 至 tanh(1)。水体的负 NDVI 经平方也可得高值，应单独筛除。
