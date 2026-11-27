关键词：样地标定的碳密度线性反演、carbon density、OLS、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

输入质量合格且已统一网格和单位的协变量栈，标注表含同名协变量、碳密度 carbon_MgC_ha 和空间 split。训练样本数大于参数个数且设计矩阵满秩。

## 2. 核心公式与参数避坑

显式截距，OLS 求解碳密度 β，跨空间留出验证。总碳量应再乘各格有效面积（ha）；不得把标准化指数直接当碳量。

## 3. 完整可运行 Python 代码

```python
import ee


def carbon(aoi: ee.Geometry, predictors: ee.Image, bands: list[str], rows: ee.FeatureCollection) -> ee.Image:
    names = ["constant"] + bands
    clean = rows.filter(ee.Filter.notNull(bands + ["carbon_MgC_ha","split"])).map(lambda f: f.set("constant",1))
    fit = clean.filter(ee.Filter.eq("split","train")).reduceColumns(
        ee.Reducer.linearRegression(len(names), 1), names + ["carbon_MgC_ha"])
    coeff = ee.Array(fit.get("coefficients")).project([0]).toList()
    x = ee.Image.constant(1).rename("constant").addBands(predictors.select(bands))
    def score(f):
        obs = ee.Array(f.toDictionary(names).values(names))
        pred = obs.dotProduct(ee.Array(coeff))
        return f.set("sqerr", ee.Number(pred).subtract(f.getNumber("carbon_MgC_ha")).pow(2))
    test = clean.filter(ee.Filter.eq("split","test")).map(score)
    valid = predictors.select(bands).mask().reduce(ee.Reducer.min())
    return (x.multiply(ee.Image.constant(coeff).rename(names)).reduce(ee.Reducer.sum())
            .updateMask(valid).rename("carbon_MgC_ha").clip(aoi)
            .set("coefficients",coeff,"test_n",test.size(),"test_rmse",ee.Number(test.aggregate_mean("sqerr")).sqrt()))
```

## 4. 结果质检标准

检查负值、残差偏差和外推。禁止用训练 R² 替代空间留出误差。
