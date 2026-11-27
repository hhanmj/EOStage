关键词：多尺度重标极差 Hurst 指数、Hurst exponent、R/S、Landsat NDVI

## 1. 适用场景与输入要求

传入来自同一格网的等间隔年度序列，波段 value，严格每年一景，n_years 为实际长度，至少 32 年。需要完整有效序列。

## 2. 核心公式与参数避坑

各尺度按不重叠分段，求离均差累积序列的极差 R 与段内标准差 S，拟合 log(平均 R/S) 对 log(段长) 的斜率 H。单一窗口无法估计斜率。

## 3. 完整可运行 Python 代码

```python
import ee


def hurst(aoi: ee.Geometry, series: ee.ImageCollection, n_years: int) -> ee.Image:
    scales = [2**k for k in range(2, n_years.bit_length()) if 2**k <= n_years // 2]
    if len(scales) < 3:
        raise ValueError("need three window sizes and at least two blocks per size")
    seq = series.sort("system:time_start").select("value")
    images = seq.toList(n_years)
    points = []
    for width in scales:
        ratios = []
        for i in range(0, n_years - width + 1, width):
            block = ee.ImageCollection.fromImages(images.slice(i, i + width))
            mean = block.mean()
            sd = block.reduce(ee.Reducer.stdDev())
            def accumulate(image, previous):
                state = ee.List(previous)
                return state.add(ee.Image(state.get(-1)).add(ee.Image(image).subtract(mean)))
            zero = mean.multiply(0)
            path = ee.ImageCollection.fromImages(ee.List(block.iterate(accumulate, ee.List([zero]))))
            ratios.append(path.max().subtract(path.min()).divide(sd).updateMask(sd.gt(0)).rename("rs"))
        valid = ee.ImageCollection.fromImages(ratios)
        y = valid.mean().updateMask(valid.count().eq(len(ratios))).log().rename("y")
        x = ee.Image.constant(width).log().rename("x")
        points.append(x.addBands(y).updateMask(y.mask()).toDouble())
    log_points = ee.ImageCollection.fromImages(points)
    result = log_points.reduce(ee.Reducer.linearFit()).select("scale").rename("hurst")
    return result.updateMask(seq.count().eq(n_years)).updateMask(
        log_points.select("y").count().eq(len(scales))).clip(aoi)
```

## 4. 结果质检标准

常量与缺测像元屏蔽。H≈0.5 是某些无记忆过程的参照；有限长度、传感器切换或趋势都会产生偏差。不要将 H>0.5 当作趋势方向。
