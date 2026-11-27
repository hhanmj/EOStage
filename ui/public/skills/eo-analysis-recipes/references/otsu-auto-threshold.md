关键词：Otsu 自适应阈值、双峰直方图分割、类间方差最大化、MNDWI 水体、ee.Reducer.histogram

## 1. 适用场景与输入要求

需要对 MNDWI/NDWI 水体提取、dNBR 过火识别或 SAR 后向散射水陆分离寻找客观数据驱动阈值、避免拍脑袋硬编码阈值时使用。输入单波段指数影像 `index_img`（波段名明确）及范围较聚焦的局部研究区 `aoi`（使前景与背景均占有一定比例，呈明显双峰）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **Otsu 类间方差最大化**：枚举直方图切分点，最大化前景占比与背景占比及两者均值差的平方积 `σB² = w0 * w1 * (μ0 - μ1)²`。
- **边缘直方图采样边界**：当研究区内几乎全是纯水体或纯陆地（单峰分布）时，Otsu 阈值会在尾部噪音处漂移；应在统计直方图时设置合理的 `maxBuckets`（如 `255`）与 `minBucketWidth`，必要时使用边缘缓冲区（例如边缘带 Sampling）确保两类样本均衡，且全部运算放在 `ee.List` / `ee.Array` 服务端完成，避免把直方图 `getInfo()` 嵌进大规模循环。

## 3. 完整可运行 Python 代码

```python
import ee


def otsu_threshold_mask(
    index_img: ee.Image,
    band_name: str,
    aoi: ee.Geometry,
    scale: float = 10.0,
    min_bucket_width: float = 0.01,
    greater_than_is_target: bool = True,
) -> tuple[ee.Number, ee.Image]:
    hist_dict = ee.Dictionary(
        index_img.select(band_name)
        .reduceRegion(
            reducer=ee.Reducer.histogram(maxBuckets=255, minBucketWidth=min_bucket_width),
            geometry=aoi,
            scale=scale,
            maxPixels=1e9,
            bestEffort=True,
            tileScale=4,
        )
        .get(band_name)
    )
    counts = ee.Array(hist_dict.get("histogram"))
    means = ee.Array(hist_dict.get("bucketMeans"))
    size = counts.length().get([0])
    total = counts.reduce(ee.Reducer.sum(), [0]).get([0])
    sum_all = means.multiply(counts).reduce(ee.Reducer.sum(), [0]).get([0])
    mean_all = sum_all.divide(total)
    indices = ee.List.sequence(1, size.subtract(1))

    def eval_split(idx):
        i = ee.Number(idx)
        a_counts = counts.slice(0, 0, i)
        a_means = means.slice(0, 0, i)
        a_total = a_counts.reduce(ee.Reducer.sum(), [0]).get([0])
        a_mean = a_counts.multiply(a_means).reduce(ee.Reducer.sum(), [0]).get([0]).divide(a_total)
        b_total = total.subtract(a_total)
        b_mean = sum_all.subtract(a_total.multiply(a_mean)).divide(b_total)
        bss = a_total.multiply(a_mean.subtract(mean_all).pow(2)).add(
            b_total.multiply(b_mean.subtract(mean_all).pow(2))
        )
        return bss

    bss_list = ee.Algorithms.If(size.gt(1), indices.map(eval_split), ee.List([0]))
    bss_arr = ee.Array(bss_list)
    best_thresh = ee.Number(
        ee.Algorithms.If(
            size.gt(1),
            ee.List(means.toList()).get(bss_arr.argmax().get(0)),
            means.get([0]),
        )
    )
    target_mask = (
        index_img.select(band_name).gt(best_thresh)
        if greater_than_is_target
        else index_img.select(band_name).lt(best_thresh)
    ).rename(f"{band_name}_otsu_mask")
    return best_thresh, target_mask.clip(aoi)
```

## 4. 结果质检标准

打印并核查返回的 `best_thresh` 是否落在物理常识区间（例如 MNDWI 通常在 `-0.1` 到 `+0.25` 之间）；若研究区单一类别占 98% 以上导致直方图无明显双谷，应切换为基于先验水体/非水体边界样本区的直方图采样。
