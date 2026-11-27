关键词：空间留出样本下的分类器比较、RF、SVM、CART、GBT、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

输入已按 SCL 去云并缩放的固定特征栈、0 起连续整数 label 和空间独立 train/test 标注点。所有模型共用同批有效样本、同一网格。

## 2. 核心公式与参数避坑

比较随机森林、RBF SVM、单棵决策树和梯度提升树。在统一单位的反射率特征上选定超参数，调参时另用训练区内空间折；独立测试样本保留至最终评价。 [GEE 分类](https://developers.google.com/earth-engine/guides/classification)

## 3. 完整可运行 Python 代码

```python
import ee


def compare(aoi: ee.Geometry, features: ee.Image, bands: list[str], points: ee.FeatureCollection,
            grid: ee.Projection, class_order: list[int]) -> ee.Image:
    samples = features.select(bands).sampleRegions(collection=points, properties=["label","split"],
                                                   projection=grid, tileScale=4)
    train = samples.filter(ee.Filter.eq("split","train"))
    test = samples.filter(ee.Filter.eq("split","test"))
    models = {"rf": ee.Classifier.smileRandomForest(150, seed=37),
              "svm": ee.Classifier.libsvm(kernelType="RBF", gamma=1, cost=10),
              "cart": ee.Classifier.smileCart(),
              "gbt": ee.Classifier.smileGradientTreeBoost(100, seed=37)}
    outputs, scores = [], {}
    for name, classifier in models.items():
        model = classifier.train(train, "label", bands)
        matrix = test.classify(model).errorMatrix("label","classification",class_order)
        scores[name] = matrix.array()
        outputs.append(features.select(bands).classify(model).rename(name))
    return ee.Image.cat(outputs).clip(aoi).set(scores).set("test_n", test.size())
```

## 4. 结果质检标准

逐类核对召回率、样本覆盖与混淆矩阵行列顺序。比较时不得以预测影像自身生成测试标签。
