关键词：CCDC 连续变化检测与分类、密集时序谐波断点、ee.Algorithms.TemporalSegmentation.Ccdc、tBreak

## 1. 适用场景与输入要求

相比年度尺度的 LandTrendr，CCDC 基于全部晴空观测密集序列同时拟合季节谐波 + 趋势段，适合精确测定亚年度级土地利用突变日期（森林砍伐、耕地撂荒、城市扩建或植被退化突变）。输入研究区 `aoi` 与多年度起止日期（建议跨越 >= 3 年且有效观测 >= 12 次/像元）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **日期格式与内部云检验波段**：推荐保持 `dateFormat=2`（小数年，如 `2021.45`）方便直接读出突变年份；提供 `tmaskBands=["green", "swir2"]` 可让 CCDC 内置迭代 Tmask 再次清除残余薄云和雪；输入集合务必已统一至同一反射率尺度。
- **数组维度随像元断点数量变化**：`tBreak` 和 `magnitude` 的第 0 轴长度等于该像元检测到的**时间段数量**（1 段表示全程无显著断点），导出或抽取最新一次断点前须先用 `arrayLength(0)` 判断长度是否 `>= 1`（及发生断点前段），避免数组越界崩溃。API 文档：[Temporal Segmentation CCDC](https://developers.google.com/earth-engine/apidocs/ee-algorithms-temporalsegmentation-ccdc)。

## 3. 完整可运行 Python 代码

```python
import ee


def ccdc_latest_breakpoint(
    aoi: ee.Geometry,
    start_date: str = "2018-01-01",
    end_date: str = "2025-01-01",
    chi_square_prob: float = 0.99,
) -> ee.Image:
    def prep_l89(col_id: str) -> ee.ImageCollection:
        def fn(img: ee.Image) -> ee.Image:
            qa = img.select("QA_PIXEL")
            clear = qa.bitwiseAnd(0b111111).eq(0).And(img.select("QA_RADSAT").eq(0))
            sr = img.select(["SR_B3", "SR_B4", "SR_B5", "SR_B6", "SR_B7"]).multiply(0.0000275).add(-0.2)
            renamed = sr.rename(["green", "red", "nir", "swir1", "swir2"])
            ndvi = renamed.normalizedDifference(["nir", "red"]).rename("ndvi")
            return renamed.addBands(ndvi).updateMask(clear).copyProperties(img, ["system:time_start"])

        return ee.ImageCollection(col_id).filterBounds(aoi).filterDate(start_date, end_date).map(fn)

    col = prep_l89("LANDSAT/LC08/C02/T1_L2").merge(prep_l89("LANDSAT/LC09/C02/T1_L2")).sort("system:time_start")

    ccdc_params = {
        "collection": col,
        "breakpointBands": ["nir", "swir1", "swir2", "ndvi"],
        "tmaskBands": ["green", "swir2"],
        "minObservations": 6,
        "chiSquareProbability": chi_square_prob,
        "minNumOfYearsScaler": 1.33,
        "dateFormat": 2,
        "lambda": 100,
        "maxIterations": 25000,
    }
    ccdc = ee.Algorithms.TemporalSegmentation.Ccdc(**ccdc_params)

    t_break = ccdc.select("tBreak")
    valid_breaks_raw = t_break.arrayMask(t_break.gt(0))
    break_count = valid_breaks_raw.arrayLength(0).rename("break_count")
    has_break = break_count.gt(0)
    valid_breaks = valid_breaks_raw.updateMask(has_break)

    latest_break_year = (
        valid_breaks.arraySlice(0, -1)
        .arrayGet([0])
        .rename("latest_break_year")
    )
    ndvi_mag = (
        ccdc.select("ndvi_magnitude")
        .arrayMask(t_break.gt(0))
        .updateMask(has_break)
        .arraySlice(0, -1)
        .arrayGet([0])
        .rename("latest_ndvi_magnitude")
    )
    return ee.Image.cat([break_count, latest_break_year, ndvi_mag]).clip(aoi)
```

## 4. 结果质检标准

在 CCDC 结果上先核查 `break_count` 的空间分布：农田轮作区或高纬度积雪区若误设过小 `chi_square_prob` 会频繁触发季节性伪断点；城市新增建设或采矿扰动区应出现与影像年份吻合的 `latest_break_year` 及负向 `latest_ndvi_magnitude`。
