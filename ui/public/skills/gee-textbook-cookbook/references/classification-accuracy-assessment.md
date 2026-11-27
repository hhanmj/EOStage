# 遥感监督分类、空间独立验证与面积无偏估计 (Classification & Olofsson 2014 Accuracy)

## 1. 严防空间自相关泄露：空间网格分组验证

禁止直接对同一多边形内部像元随机划分训练/测试集！必须按网格单元 ID 划分独立训练块与验证块：

```python
import ee

def train_rf_spatial_cv(composite: ee.Image, samples: ee.FeatureCollection, bands: list[str], grid_deg: float = 0.05):
    def tag_block(feat: ee.Feature) -> ee.Feature:
        coords = feat.geometry().centroid(1).coordinates()
        bx = ee.Number(coords.get(0)).divide(grid_deg).floor()
        by = ee.Number(coords.get(1)).divide(grid_deg).floor()
        return feat.set("block_hash", bx.multiply(10000).add(by).mod(5))

    tagged = samples.map(tag_block)
    train_pts = tagged.filter(ee.Filter.neq("block_hash", 0))
    test_pts = tagged.filter(ee.Filter.eq("block_hash", 0))

    training = composite.select(bands).sampleRegions(
        collection=train_pts, properties=["landcover"], scale=10, tileScale=4
    )
    classifier = ee.Classifier.smileRandomForest(numberOfTrees=150, minLeafPopulation=3).train(
        features=training, classProperty="landcover", inputProperties=bands
    )
    classified = composite.select(bands).classify(classifier)

    validation = composite.select(bands).sampleRegions(
        collection=test_pts, properties=["landcover"], scale=10, tileScale=4
    ).classify(classifier)
    cm = validation.errorMatrix("landcover", "classification")
    return classified, cm
```

## 2. Olofsson (2014) 分层无偏面积与 95% 置信区间公式

像元计数 $\neq$ 真实地表物理面积！存在错分/漏分误差时，设第 $i$ 类制图面积比例为 $W_i = A_{m,i}/A_{\text{tot}}$，混淆矩阵行（制图类 $i$）对参考类 $j$ 的条件比例为 $\hat{p}_{ij} = W_i \cdot (n_{ij} / n_{i\cdot})$，则第 $k$ 类参考无偏面积比例及其标准误为：

$$\hat{p}_{\cdot k} = \sum_i W_i \frac{n_{ik}}{n_{i\cdot}}, \qquad S(\hat{p}_{\cdot k}) = \sqrt{\sum_i W_i^2 \frac{\frac{n_{ik}}{n_{i\cdot}}\left(1 - \frac{n_{ik}}{n_{i\cdot}}\right)}{n_{i\cdot} - 1}}$$

95% 面积置信区间：$\hat{A}_k \pm 1.96 \cdot A_{\text{tot}} \cdot S(\hat{p}_{\cdot k})$。
