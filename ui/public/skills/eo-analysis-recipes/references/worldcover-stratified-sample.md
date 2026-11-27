关键词：分层抽取地类像元、ESA/WorldCover/v200、stratified sampling、WorldCover

## 1. 适用场景与输入要求

输入 AOI 和每类目标样本数。2021 年 10 m 分类，可用于样点设计。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

按整数类别分层、固定随机种子；稀有类可返回少于目标数的点。训练与验证须按空间块独立划分。 [ESA/WorldCover/v200](https://developers.google.com/earth-engine/datasets/catalog/ESA_WorldCover_v200)

## 3. 完整可运行 Python 代码

```python
import ee


def sample_classes(aoi: ee.Geometry, per_class: int = 100,
                   seed: int = 17) -> ee.FeatureCollection:
    if per_class < 1:
        raise ValueError("per_class must be positive")
    image = ee.ImageCollection("ESA/WorldCover/v200").first().select("Map")
    return image.stratifiedSample(numPoints=per_class, classBand="Map",
                                  region=aoi, projection=image.projection(),
                                  seed=seed, geometries=True, tileScale=4)
```

## 4. 结果质检标准

统计 Map 频数与重复坐标。用独立参考标签核验精度；分层样本的简单总体平均会高估稀有类权重。
