关键词：按变量类型统一栅格、resampling、reduceResolution、projection、GEE

## 1. 适用场景与输入要求

输入有原生仿射网格的单波段影像、目标投影；类别、连续量及面积平均需分别处理。

## 2. 核心公式与参数避坑

最近邻保持类别编码；双线性适用于连续面插值；细网格到粗网格的平均使用重叠面积加权。

## 3. 完整可运行 Python 代码

```python
import ee


def align_grid(aoi: ee.Geometry, image: ee.Image, grid: ee.Projection, method: str) -> ee.Image:
    if method == "mean":
        aligned = image.reduceResolution(reducer=ee.Reducer.mean(), maxPixels=4096)
    elif method == "bilinear":
        aligned = image.resample("bilinear")
    elif method == "nearest":
        aligned = image
    else:
        raise ValueError("choose mean, bilinear or nearest")
    return aligned.reproject(grid).clip(aoi)
```

## 4. 结果质检标准

核对源与目标像元中心及 CRS。聚合前保留 AOI 边界外邻近像元；人口等像元总量需守恒重分配。
