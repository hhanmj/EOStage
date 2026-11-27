关键词：带粗网格残差约束的地表温度细化、thermal sharpening、LST、Landsat、Sentinel-2

## 1. 适用场景与输入要求

输入已配准的细尺度连续特征、单波段粗 LST（K）和米制粗细网格。用邻近过境时间和相似云掩膜，训练与验证区域应互斥。

## 2. 核心公式与参数避坑

先将特征面积聚合到热网格，拟合多元线性回归。在细网格运用回归，再把粗网格上预测均值与观测的差值加回。

## 3. 完整可运行 Python 代码

```python
import ee


def sharpen_temperature(aoi: ee.Geometry, predictors: ee.Image, bands: list[str],
                        lst: ee.Image, coarse_grid: ee.Projection, fine_grid: ee.Projection,
                        training_area: ee.Geometry) -> ee.Image:
    if not bands:
        raise ValueError("supply at least one predictor")
    fine = predictors.select(bands).reproject(fine_grid)

    def coarse_mean(im):
        return im.reduceResolution(reducer=ee.Reducer.mean(), maxPixels=4096).reproject(coarse_grid)

    terms = ["intercept", *bands]
    coarse = coarse_mean(fine)
    design = coarse.select(0).multiply(0).add(1).rename("intercept").addBands(coarse)
    y = lst.select(0).rename("lst_k").reproject(coarse_grid)
    fit = design.addBands(y).reduceRegion(
        reducer=ee.Reducer.linearRegression(len(terms), 1), geometry=training_area,
        crs=coarse_grid, maxPixels=1e8, tileScale=4)
    weights = ee.Image(ee.Array(fit.get("coefficients"))).arrayProject([0]).arrayFlatten([terms])
    high = fine.select(0).multiply(0).add(1).rename("intercept").addBands(fine)
    raw = high.multiply(weights).reduce(ee.Reducer.sum()).rename("lst_k").reproject(fine_grid)
    residual = y.subtract(coarse_mean(raw)).reproject(fine_grid)
    return raw.add(residual).rename("sharpened_k").setDefaultProjection(fine_grid).clip(aoi)
```

## 4. 结果质检标准

粗细网格整倍数对齐时，回聚合应尽量保留原 LST。验证使用留出粗热像元与独立细尺度参考。标注输出为估计值。
