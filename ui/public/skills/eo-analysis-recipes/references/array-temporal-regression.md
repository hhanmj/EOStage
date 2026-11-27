关键词：逐像元数组线性回归、array image、matrixSolve、OLS、GEE

## 1. 适用场景与输入要求

统一空间网格的单波段序列，传入时间戳、波段和日期窗。

## 2. 核心公式与参数避坑

将 [截距, 距起点年数, 观测值] 叠成 [时间, 变量]，按轴切分 X 和 y，再求最小二乘。

## 3. 完整可运行 Python 代码

```python
import ee


def temporal_fit(aoi: ee.Geometry, start_date: str, end_date: str,
                 series: ee.ImageCollection, band: str, grid: ee.Projection) -> ee.Image:
    start = ee.Date(start_date)
    series = series.filterBounds(aoi).filterDate(start_date, end_date).sort("system:time_start")

    def design(im):
        y = im.select(band).toDouble().rename("y")
        one = y.multiply(0).add(1).rename("intercept")
        t = one.multiply(im.date().difference(start, "year")).rename("time")
        return one.addBands(t).addBands(y)

    array = series.map(design).toArray()
    n = array.arrayLength(0).rename("n")
    design_matrix = array.arraySlice(1, 0, 2)
    observed = array.arraySlice(1, 2, 3)
    solved = design_matrix.updateMask(n.gte(3)).matrixSolve(observed.updateMask(n.gte(3)))
    coeff = solved.arrayProject([0]).arrayFlatten([["intercept", "slope_per_year"]])
    return coeff.addBands(n).setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

样本要有至少两个不同时间。与已知斜率的序列对照，检查时间轴方向、单位、残差及缺测。
