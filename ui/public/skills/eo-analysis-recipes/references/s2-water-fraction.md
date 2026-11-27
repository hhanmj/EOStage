关键词：样本标定的水体面积分数、fractional surface water、DSWF、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

输入同日高分影像汇总到目标格网的真实水面分数 [0,1]，校准/验证样本 spatial split；预测变量与标签须空间同支持。

## 2. 核心公式与参数避坑

以连续分数训练回归模型，限制输出并保留原预测用于诊断。原材料为论文解读；此卡是可重用的监督回归方法，复现论文自训练仍需其伪标签、训练集及参数。

## 3. 完整可运行 Python 代码

```python
import ee


def water_fraction(aoi: ee.Geometry, stack: ee.Image, bands: list[str], labels: ee.FeatureCollection) -> ee.Image:
    rows = labels.filter(ee.Filter.notNull(bands+["fraction","split"])).filter(
        ee.Filter.And(ee.Filter.gte("fraction",0),ee.Filter.lte("fraction",1)))
    model = ee.Classifier.smileRandomForest(120,seed=17).setOutputMode("REGRESSION").train(
        rows.filter(ee.Filter.eq("split","train")),"fraction",bands)
    def err(f):
        return f.set("abs_err",f.getNumber("prediction").subtract(f.getNumber("fraction")).abs())
    valid = rows.filter(ee.Filter.eq("split","test")).classify(model,"prediction").map(err)
    raw = stack.select(bands).classify(model,"raw")
    return raw.addBands(raw.clamp(0,1).rename("water_fraction")).clip(aoi).set(
        "mae",valid.aggregate_mean("abs_err"),"test_n",valid.size())
```

## 4. 结果质检标准

在独立区域报告 MAE 并检查混合岸线、阴影、浑水及季节迁移。水体面积为 Σ(分数×像元面积)。
