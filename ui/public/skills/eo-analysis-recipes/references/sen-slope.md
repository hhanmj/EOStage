关键词：年序列稳健变化率、Theil–Sen、Sen slope、趋势

## 1. 适用场景与输入要求

输入逐年同季、同网格单波段影像集合，带 system:time_start。每年仅一张影像。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

Sen 斜率取所有成对时间斜率的中位数；用年份作横轴才得到每年变化率。

## 3. 完整可运行 Python 代码

```python
import ee


def sen_slope(aoi: ee.Geometry, annual: ee.ImageCollection,
              start_date: str, end_date: str) -> ee.Image:
    def pair(im):
        y = im.select(0).toDouble().rename("value")
        year = ee.Date(im.get("system:time_start")).get("year")
        x = y.multiply(0).add(ee.Number(year)).rename("year")
        return x.addBands(y)

    series = annual.filterBounds(aoi).filterDate(start_date, end_date).map(pair)
    n = series.select("value").count().rename("n_years")
    slope = series.reduce(ee.Reducer.sensSlope()).select("slope")
    return slope.updateMask(n.gte(3)).addBands(n).clip(aoi)
```

## 4. 结果质检标准

恒定序列应为零。报告有效年份、时间跨度；用适合自相关序列的检验评估趋势显著性。
