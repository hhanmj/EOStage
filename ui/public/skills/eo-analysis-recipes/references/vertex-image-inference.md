关键词：分块调用已部署的影像模型、CNN、forest classifier、Vertex AI、ee.Model

## 1. 适用场景与输入要求

用户提供已部署端点、训练时的标准化波段栈、米制训练网格和输入块参数。端点约定输出每像元一个浮点森林概率。

## 2. 核心公式与参数避坑

以训练网格固定取样。重叠整体按输入重叠尺寸设置，足够覆盖模型感受野。对本地有效掩膜外的预测进行屏蔽。 [Earth Engine 接口](https://developers.google.com/earth-engine/apidocs/ee-model-fromvertexai)

## 3. 完整可运行 Python 代码

```python
import ee


def forest_probability(aoi: ee.Geometry, predictors: ee.Image, bands: list[str],
                       endpoint: str, grid: ee.Projection, tile_px: int = 128,
                       overlap_px: int = 32) -> ee.Image:
    if tile_px <= 0 or not 0 <= overlap_px < tile_px:
        raise ValueError("invalid tile or overlap size")
    data = predictors.select(bands).toFloat()
    model = ee.Model.fromVertexAi(
        endpoint=endpoint, inputTileSize=[tile_px, tile_px],
        inputOverlapSize=[overlap_px, overlap_px], proj=grid, fixInputProj=True,
        outputBands={"probability": {"type": ee.PixelType.float(), "dimensions": 0}},
    )
    valid = data.mask().reduce(ee.Reducer.min())
    return model.predictImage(data).select("probability").updateMask(valid).clip(aoi)
```

## 4. 结果质检标准

先在训练以外的已知小区核对输入通道、输出尺寸、[0,1] 概率和块边伪影。云掩膜附近还需核对模型对上下文缺测的敏感性。
