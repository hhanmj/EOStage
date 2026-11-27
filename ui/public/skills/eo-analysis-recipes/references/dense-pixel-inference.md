关键词：逐像元全连接网络推断、DNN、ReLU、sigmoid、matrixMultiply、GEE

## 1. 适用场景与输入要求

提供已训练的二分类网络权重矩阵和偏置，预测变量按训练定义归一化与排序。

## 2. 核心公式与参数避坑

ReLU(W₁x+b₁) 后计算 W₂h+b₂，以 sigmoid 给出概率。权重按 [输出,输入] 排列。

## 3. 完整可运行 Python 代码

```python
import ee


def dense_probability(aoi: ee.Geometry, features: ee.Image, bands: list[str],
                      weights: list[list[float]], bias: list[float],
                      output_weights: list[float], output_bias: float) -> ee.Image:
    if not weights or len(weights) != len(bias) or len(weights) != len(output_weights):
        raise ValueError("hidden layer dimensions do not match")
    if not bands or any(len(row) != len(bands) for row in weights):
        raise ValueError("input dimensions do not match")
    x = features.select(bands).toDouble().toArray().toArray(1)
    def matrix(values):
        return ee.Image(ee.Array(values))
    hidden = matrix(weights).matrixMultiply(x).add(matrix([[v] for v in bias]))
    hidden = hidden.add(hidden.abs()).multiply(.5)
    logits = matrix([output_weights]).matrixMultiply(hidden).arrayGet([0, 0]).add(output_bias)
    probability = logits.clamp(-60, 60).multiply(-1).exp().add(1).pow(-1)
    return probability.rename("probability").setDefaultProjection(features.select(bands[0]).projection()).clip(aoi)
```

## 4. 结果质检标准

用训练环境对相同特征点逐个比对概率，检查预处理和权重转置。概率阈值单独在验证集标定。
