关键词：林火过火面积、dNBR、RdNBR、燃烧烈度分级、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

用于评估森林/灌草地火灾过火范围与燃烧烈度（Burn Severity）。输入研究区 `aoi`、火前晴空参照窗口 `[pre_start, pre_end)` 和火后评估窗口 `[post_start, post_end)`（通常取相同季节以降低物候干扰，或在火后 1–2 个月内完成即时评估）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **NBR 与 dNBR 定义**：健康植被在近红外（S2 `B8`, 10 m）高反射、短波红外 2（`B12`, 20 m）低反射；过火后刚好反转。`NBR = (B8 - B12) / (B8 + B12)`，`dNBR = NBR_pre - NBR_post`，正值越大说明过火越严重。
- **水体/农田干扰与 RdNBR 除零防护**：水面和收割农田在 `dNBR` 上易出现正负伪异常，分析火灾烈度时应结合地类或火前绿度筛选，计算相对差分指标 `RdNBR = dNBR / sqrt(|NBR_pre|)` 时必须屏蔽 `|NBR_pre| <= 0.001` 防止除零爆炸。官方目录：[COPERNICUS/S2_SR_HARMONIZED](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S2_SR_HARMONIZED)。

## 3. 完整可运行 Python 代码

```python
import ee


def s2_wildfire_dnbr_severity(
    aoi: ee.Geometry,
    pre_start: str,
    pre_end: str,
    post_start: str,
    post_end: str,
    cs_thresh: float = 0.60,
) -> ee.Image:
    s2 = ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED").filterBounds(aoi)
    cs = ee.ImageCollection("GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED").filterBounds(aoi)

    def period_nbr(t0: str, t1: str) -> ee.Image:
        col = s2.filterDate(t0, t1).linkCollection(cs.filterDate(t0, t1), ["cs"])

        def calc(img: ee.Image) -> ee.Image:
            clear = img.select("cs").gte(cs_thresh)
            sr = img.select(["B8", "B12"]).multiply(0.0001).updateMask(clear)
            return sr.normalizedDifference(["B8", "B12"]).rename("nbr")

        return col.map(calc).median().clip(aoi)

    nbr_pre = period_nbr(pre_start, pre_end).rename("nbr_pre")
    nbr_post = period_nbr(post_start, post_end).rename("nbr_post")
    dnbr = nbr_pre.subtract(nbr_post).rename("dnbr")
    rdnbr = (
        dnbr.divide(nbr_pre.abs().sqrt())
        .updateMask(nbr_pre.abs().gt(1e-3))
        .rename("rdnbr")
    )
    # USGS dNBR severity classes: 0 unburned, 1 low, 2 moderate-low, 3 moderate-high, 4 high
    severity = (
        ee.Image(0)
        .where(dnbr.gte(0.10), 1)
        .where(dnbr.gte(0.27), 2)
        .where(dnbr.gte(0.44), 3)
        .where(dnbr.gte(0.66), 4)
        .updateMask(dnbr.mask())
        .toInt8()
        .rename("severity_class")
    )
    return nbr_pre.addBands([nbr_post, dnbr, rdnbr, severity])
```

## 4. 结果质检标准

确认火前/火后植被未受强云烟残留污染；检查未过火参照林区 `dnbr` 均值应接近 `0`（绝对偏移通常 `< 0.05`），若出现系统性正向偏移需先做未过火对照样本校准或改用同物候窗合成。
