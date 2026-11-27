# 光学去云掩膜与定标合成 (Optical Cloud Masking & Harmonized Compositing)

> 具体单传感器、单指标的原子合成配方见 `eo-analysis-recipes` 中的 `s2-cloud-score-plus.md`、`s2-clear-composite.md` 与 `landsat-surface-reflectance.md`。本文档重点阐述方法选型原则、跨期辐射基线统一与避坑规程。

## 1. Sentinel-2 L2A Harmonized 云掩膜选型 (`COPERNICUS/S2_SR_HARMONIZED`)

- **统一基线防双扣**：务必选用 `COPERNICUS/S2_SR_HARMONIZED`，官方已将 2022-01-25（Processing Baseline 04.00）之后的 `-1000` 偏移量统一拉齐；仅需乘以 `0.0001` 转为地表反射率 `[0, 1]`，**严禁**手动加减 `1000`。
- **方法首选 Cloud Score+（`GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED`）**：通过 `s2.linkCollection(cs_plus, ["cs", "cs_cdf"])` 挂载逐像元晴空评分，能有效清除 `QA60` 漏检的半透明薄雾、卷云与云边缘阴影；推荐优先阈值 `cs >= 0.60`，常年多云热带区可改用 `cs_cdf >= 0.50`。
- **`QA60` 已弃用 / `SCL` 兜底原则**：`QA60` 在 Baseline 04.00 后已不推荐使用且对低空薄云与云影遗漏严重；无 Cloud Score+ 配对时请优先基于场景分类波段 `SCL` 剔除 `{3: 云阴影, 8: 中概率云, 9: 高概率云, 10: 薄卷云, 11: 冰雪}`（积雪研究显式保留 11）。

```python
import ee

def prep_s2_cloud_score_plus(
    aoi: ee.Geometry,
    start: str,
    end: str,
    qa_band: str = "cs",
    thresh: float = 0.60,
    use_scl_guard: bool = True,
) -> ee.ImageCollection:
    s2 = (
        ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED")
        .filterBounds(aoi)
        .filterDate(start, end)
        .filter(ee.Filter.lt("CLOUDY_PIXEL_PERCENTAGE", 70))
    )
    cs = ee.ImageCollection("GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED").filterBounds(aoi).filterDate(start, end)

    def mask_one(img: ee.Image) -> ee.Image:
        clear = img.select(qa_band).gte(thresh)
        if use_scl_guard:
            scl = img.select("SCL")
            clear = clear.And(scl.neq(3)).And(scl.neq(8)).And(scl.neq(9)).And(scl.neq(10))
        sr = img.select(["B2", "B3", "B4", "B8", "B11", "B12"]).multiply(0.0001)
        return sr.updateMask(clear).copyProperties(img, ["system:time_start"])

    return s2.linkCollection(cs, [qa_band]).map(mask_one)
```

## 2. Landsat C2 L2 `QA_PIXEL` 位掩膜与官方缩放系数

Landsat Collection 2 Level-2 严格使用官方线性系数（**严禁使用已下线的 Collection 1 系数**，且 NDVI 等比值指数计算前也**必须**先完成 `− 0.2` 偏移定标）：
- 光学反射率：`SR = DN * 0.0000275 - 0.2`
- 热红外地表温度（ST_B10）：`K = DN * 0.00341802 + 149.0`
- `QA_PIXEL` 位标志 + 辐射饱和检查：屏蔽 Bit 0（Fill）、Bit 1（Dilated Cloud）、Bit 2（Cirrus）、Bit 3（Cloud）、Bit 4（Cloud Shadow）、Bit 5（Snow），并检查 `QA_RADSAT == 0`。

```python
def prep_landsat_c2_l2(img: ee.Image) -> ee.Image:
    qa = img.select("QA_PIXEL")
    mask_bits = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5)
    clear = qa.bitwiseAnd(mask_bits).eq(0).And(img.select("QA_RADSAT").eq(0))

    sr = img.select("SR_B.").multiply(0.0000275).add(-0.2)
    st = img.select("ST_B10").multiply(0.00341802).add(149.0).rename("ST_K")
    return img.addBands(sr, None, True).addBands(st, None, True).updateMask(clear)
```
