关键词：可重复分批提取大量点值、sampleRegions、randomColumn、batch、GEE

## 1. 适用场景与输入要求

点必须具有稳定且唯一的 point_id。每次由宿主调用一个批次，影像与投影保持一致。

## 2. 核心公式与参数避坑

以 stable ID 和固定随机种子生成确定性桶号。避免按几何切块时边界重复落点。

## 3. 完整可运行 Python 代码

```python
import ee


def sample_batch(aoi: ee.Geometry, image: ee.Image, points: ee.FeatureCollection,
                 bands: list[str], grid: ee.Projection, batch: int, total: int) -> ee.FeatureCollection:
    if not 0 <= batch < total:
        raise ValueError("batch must satisfy 0 <= batch < total")
    ranked = points.filterBounds(aoi).randomColumn("bucket_key", 42, "uniform", ["point_id"])
    members = ranked.filter(ee.Filter.gte("bucket_key", batch / total)).filter(
        ee.Filter.lt("bucket_key", (batch + 1) / total))
    return image.select(bands).sampleRegions(collection=members, properties=["point_id"],
                                            projection=grid, tileScale=4, geometries=False)
```

## 4. 结果质检标准

合并后按 point_id 查重并与输入核对。任何波段掩膜都可能丢弃点，需另报匹配率。
