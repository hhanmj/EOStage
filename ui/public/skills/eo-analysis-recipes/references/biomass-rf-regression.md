关键词：随机森林地上生物量回归、AGB、RF regression、NASA/ORNL/biomass_carbon_density/v1

## 1. 适用场景与输入要求

训练表须含连续响应 agb（Mg 干物质/ha）、协变量及空间块 split=train/test。输入已按样地或标签网格匹配尺度的预测栈。

## 2. 核心公式与参数避坑

使用 REGRESSION 输出，保留独立块计算 RMSE、偏差和样本数。原始粗分辨率参考图不能拆成多个 10 m 像元充当独立标签。

## 3. 完整可运行 Python 代码

```python
import ee


def estimate_agb(aoi: ee.Geometry, stack: ee.Image, bands: list[str],
                 samples: ee.FeatureCollection) -> ee.Image:
    rows = samples.filter(ee.Filter.notNull(bands + ["agb","split"]))
    train = rows.filter(ee.Filter.eq("split","train"))
    model = (ee.Classifier.smileRandomForest(150, seed=37).setOutputMode("REGRESSION")
             .train(train, "agb", bands))
    def error(f):
        e = ee.Number(f.get("prediction")).subtract(f.getNumber("agb"))
        return f.set({"error": e, "sq": e.pow(2)})
    test = rows.filter(ee.Filter.eq("split","test")).classify(model,"prediction").map(error)
    return (stack.select(bands).classify(model,"agb_Mg_ha").clip(aoi)
            .set("test_rmse", ee.Number(test.aggregate_mean("sq")).sqrt(),
                 "test_bias", test.aggregate_mean("error"), "test_n", test.size()))
```

## 4. 结果质检标准

验证集应非空且覆盖高低生物量。报告训练范围、高值饱和及外推区。AGB 和地上碳密度换算需要独立给定碳因子。
