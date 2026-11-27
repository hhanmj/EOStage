关键词：Sentinel-2 Cloud Score+、晴空概率评分、linkCollection、COPERNICUS/S2_SR_HARMONIZED、GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED

## 1. 适用场景与输入要求

需要比传统 `SCL`/`QA60` 更稳健地去除薄卷云、云边缘半透明雾霾和地形/建筑阴影时优先使用，适用于 2015 年中至今的 Sentinel-2 L2A 光学合成。输入研究区边界 `aoi`、左闭右开日期 `[start_date, end_date)`、可用性波段（`cs` 对薄雾敏感，`cs_cdf` 容忍少量薄霾）和晴空阈值（常用 `0.55–0.65`）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **严禁混用旧集合**：使用调和后的 `COPERNICUS/S2_SR_HARMONIZED` 并以 `/ 10000.0`（或 `* 0.0001`）定标至 `[0, 1]` 地表反射率，无需手动加减 `+1000`。
- **联合绑定 (`linkCollection`)**：Google Cloud Score+ 与 Sentinel-2 通过共享产品 ID 精确配对，`linkCollection(cs_plus, [qa_band])` 自动跳过缺失评估的景并把质量波段写入对应影像；若研究区常年多云导致可用像元不足，可把 `qa_band` 设为 `"cs_cdf"` 且阈值放宽至 `0.50`，切勿无约束把阈值降至 `0.2` 以下以免大量云边缘污染均值/中位数。官方目录：[GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED](https://developers.google.com/earth-engine/datasets/catalog/GOOGLE_CLOUD_SCORE_PLUS_V1_S2_HARMONIZED)。

## 3. 完整可运行 Python 代码

```python
import ee


def s2_cloud_score_plus_composite(
    aoi: ee.Geometry,
    start_date: str,
    end_date: str,
    qa_band: str = "cs",
    clear_thresh: float = 0.60,
    max_cloud_pct: float = 80.0,
) -> ee.Image:
    if qa_band not in ("cs", "cs_cdf"):
        raise ValueError("qa_band must be 'cs' or 'cs_cdf'")
    if not 0.0 < clear_thresh < 1.0:
        raise ValueError("clear_thresh must be in (0, 1)")

    bands = ["B2", "B3", "B4", "B8", "B11", "B12"]
    s2 = (
        ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED")
        .filterBounds(aoi)
        .filterDate(start_date, end_date)
        .filter(ee.Filter.lt("CLOUDY_PIXEL_PERCENTAGE", max_cloud_pct))
    )
    cs_plus = (
        ee.ImageCollection("GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED")
        .filterBounds(aoi)
        .filterDate(start_date, end_date)
    )
    linked = s2.linkCollection(cs_plus, [qa_band])

    def prep(img: ee.Image) -> ee.Image:
        qa = img.select(qa_band)
        valid = qa.gte(clear_thresh)
        sr = (
            img.select(bands)
            .multiply(0.0001)
            .updateMask(valid)
            .set("system:time_start", img.get("system:time_start"))
        )
        return sr

    clean = linked.map(prep)
    composite = clean.median().clip(aoi)
    clear_count = clean.select("B4").count().toInt16().rename("clear_count").clip(aoi)
    return composite.addBands(clear_count)
```

## 4. 结果质检标准

检查 `clear_count` 波段的空间覆盖率（重点区域建议有效像元观测 `>= 3`）；反射率数值合理区间一般位于 `[0, 1]`，中位数合成无明显云边缘灰白残留晕圈或条带拼接缝。
