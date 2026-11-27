关键词：协调反射率多时相特征、HLS、NASA/HLS/HLSL30/v002、NASA/HLS/HLSS30/v002、NDVI

## 1. 适用场景与输入要求

30 m HLS 成品中的 OLI 与 MSI。两种产品的空间及光谱协调由生产方完成；普通 Sentinel-2 L2A 单传感器任务用 S2_SR_HARMONIZED。

## 2. 核心公式与参数避坑

统一红、窄近红外和两个短波红外波段；Fmask 位 1–4 排除云、邻云、阴影、雪。NDVI 使用同景比值，按时间窗提取分位数和观测数。 [L30](https://developers.google.com/earth-engine/datasets/catalog/NASA_HLS_HLSL30_v002) · [S30](https://developers.google.com/earth-engine/datasets/catalog/NASA_HLS_HLSS30_v002)

## 3. 完整可运行 Python 代码

```python
import ee


def hls_features(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    def series(dataset, bands):
        def prepare(im):
            clear = im.select("Fmask").bitwiseAnd(30).eq(0)
            sr = im.select(bands, ["red","nir","swir1","swir2"]).updateMask(clear)
            n, r = sr.select("nir"), sr.select("red")
            nd = n.subtract(r).divide(n.add(r)).updateMask(n.add(r).gt(0)).rename("ndvi")
            return sr.addBands(nd)
        return ee.ImageCollection(dataset).filterBounds(aoi).filterDate(start_date,end_date).map(prepare)
    l30 = series("NASA/HLS/HLSL30/v002", ["B4","B5","B6","B7"])
    s30 = series("NASA/HLS/HLSS30/v002", ["B4","B8A","B11","B12"])
    merged = l30.merge(s30)
    return (merged.reduce(ee.Reducer.percentile([10,50,90]))
            .addBands(merged.select("ndvi").count().rename("n")).clip(aoi))
```

## 4. 结果质检标准

检查不同传感器同日匹配样本的偏差与全期 n。HLS 编码依所用目录；这里统计 GEE 原生 HLS 数值，不额外套用 Landsat C2 的偏移或 Sentinel L2A 的系数。
