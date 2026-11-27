关键词：按时间键配对像元相关、Pearson correlation、ArrayImage、GEE

## 1. 适用场景与输入要求

两套单位一致且有唯一 system:time_start 的单波段集合，传入共同网格；逐日或逐月序列应先聚合同一时间键。

## 2. 核心公式与参数避坑

内连接两套观测，两个量同时有效才计入样本。至少三对样本且均有变异。

## 3. 完整可运行 Python 代码

```python
import ee


def paired_pearson(aoi: ee.Geometry, start_date: str, end_date: str,
                   left: ee.ImageCollection, right: ee.ImageCollection,
                   left_band: str, right_band: str, grid: ee.Projection) -> ee.Image:
    left = left.filterDate(start_date, end_date).filterBounds(aoi)
    right = right.filterDate(start_date, end_date).filterBounds(aoi)
    match = ee.Filter.equals(leftField="system:time_start", rightField="system:time_start")
    pairs = ee.Join.inner().apply(left, right, match)

    def stack(obj):
        f = ee.Feature(obj)
        x = ee.Image(f.get("primary")).select(left_band).rename("x")
        y = ee.Image(f.get("secondary")).select(right_band).rename("y")
        return x.addBands(y).updateMask(x.mask().And(y.mask())).setDefaultProjection(grid)

    series = ee.ImageCollection.fromImages(pairs.toList(pairs.size()).map(stack))
    n = series.select("x").count().rename("n_pairs")
    result = series.reduce(ee.Reducer.pearsonsCorrelation()).select("correlation")
    sd = series.reduce(ee.Reducer.stdDev())
    varied = sd.select(0).gt(0).And(sd.select(1).gt(0))
    return result.updateMask(n.gte(3).And(varied)).addBands(n).setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

检查 r∈[−1,1]、配对数和重复日期。季节性、趋势和时间自相关会使常规显著性偏乐观。
