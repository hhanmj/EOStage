关键词：空间独立验证的土地覆盖分类、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED、RF、random forest

## 1. 适用场景与输入要求

输入已去云的反射率合成、波段名和带整数 label 的独立标注点。split 字段来自预先划分的空间块，取 train 或 test。分类网格由调用方明确。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

在训练分区抽取特征并拟合随机森林，在保留分区生成混淆矩阵。类别须从 0 连续编码；每个类别在训练及验证中均要有样本。 [COPERNICUS/S2_SR_HARMONIZED](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S2_SR_HARMONIZED)

## 3. 完整可运行 Python 代码

```python
import ee


def classify(aoi: ee.Geometry, composite: ee.Image, bands: list[str],
             points: ee.FeatureCollection, grid: ee.Projection) -> ee.Image:
    features = composite.select(bands)
    rows = features.sampleRegions(collection=points, properties=["label", "split"],
                                  projection=grid, tileScale=4)
    train = rows.filter(ee.Filter.eq("split", "train"))
    test = rows.filter(ee.Filter.eq("split", "test"))
    model = ee.Classifier.smileRandomForest(150, seed=37).train(train, "label", bands)
    matrix = test.classify(model).errorMatrix("label", "classification")
    return (features.classify(model).clip(aoi)
            .set("validation_matrix", matrix.array(), "validation_n", test.size()))
```

## 4. 结果质检标准

检查混淆矩阵逐类召回率、样本数与空间分布。核对被云掩膜丢弃的验证点。
