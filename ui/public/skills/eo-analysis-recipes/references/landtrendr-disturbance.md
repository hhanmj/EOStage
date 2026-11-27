关键词：LandTrendr、年度光谱时间分割、森林干扰年份与强度、ee.Algorithms.TemporalSegmentation.LandTrendr

## 1. 适用场景与输入要求

用于 1985–现今年度尺度的森林砍伐、火灾扰动与植被长期恢复分段检测，定位扰动发生年份（YOD）、变化幅度和持续年数。输入研究区 `aoi`、起始年份与结束年份（至少 6 个连续有效年份）以及生长季筛选月份。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **每年必须恰好一景且第 1 波段驱动断点**：`LandTrendr` 默认只以输入集合的**第 1 个波段**寻找转折点并将所有波段拟合到同组断点上，跨年份序列缺某一年会导致整条像元数组缩短；应在服务端对每年生长季合成去云 `NBR`（`TM/ETM+` 用 `SR_B4, SR_B7`，`OLI` 用 `SR_B5, SR_B7`），且在每年输出上设置 `system:time_start`。
- **提取 4 行 `LandTrendr` 矩阵结构**：返回 2D 数组 `LandTrendr` 的第 0–3 行依次为 `[year, source_val, fitted_val, is_vertex]`；筛选 `is_vertex == 1` 后计算相邻顶点拟合值差，**植被退化/干扰对应 NBR 骤降**（`fitted_start - fitted_end > 0`），不要直接与反符号阈值混淆。指南参考：[Temporal Segmentation LandTrendr](https://developers.google.com/earth-engine/apidocs/ee-algorithms-temporalsegmentation-landtrendr)。

## 3. 完整可运行 Python 代码

```python
import ee


def landtrendr_nbr_disturbance(
    aoi: ee.Geometry,
    start_year: int = 2000,
    end_year: int = 2024,
    season_start_month: int = 6,
    season_end_month: int = 9,
) -> ee.Image:
    if end_year - start_year + 1 < 6:
        raise ValueError("LandTrendr requires at least 6 annual observations")

    def prep_c2(col_id: str, nir: str, swir2: str) -> ee.ImageCollection:
        def fn(img: ee.Image) -> ee.Image:
            qa = img.select("QA_PIXEL")
            clear = qa.bitwiseAnd(0b111111).eq(0).And(img.select("QA_RADSAT").eq(0))
            sr = img.select([nir, swir2]).multiply(0.0000275).add(-0.2)
            nbr = sr.normalizedDifference([nir, swir2]).updateMask(clear).rename("NBR")
            return nbr.copyProperties(img, ["system:time_start"])

        return ee.ImageCollection(col_id).filterBounds(aoi).map(fn)

    all_obs = (
        prep_c2("LANDSAT/LT05/C02/T1_L2", "SR_B4", "SR_B7")
        .merge(prep_c2("LANDSAT/LE07/C02/T1_L2", "SR_B4", "SR_B7"))
        .merge(prep_c2("LANDSAT/LC08/C02/T1_L2", "SR_B5", "SR_B7"))
        .merge(prep_c2("LANDSAT/LC09/C02/T1_L2", "SR_B5", "SR_B7"))
        .filter(ee.Filter.calendarRange(season_start_month, season_end_month, "month"))
    )

    def build_annual(yr):
        y = ee.Number(yr)
        t0 = ee.Date.fromYMD(y, 1, 1)
        med = all_obs.filterDate(t0, t0.advance(1, "year")).median()
        return med.set("system:time_start", t0.millis())

    annual_col = ee.ImageCollection.fromImages(
        ee.List.sequence(start_year, end_year).map(build_annual)
    )
    lt = ee.Algorithms.TemporalSegmentation.LandTrendr(
        timeSeries=annual_col.select(["NBR"]),
        maxSegments=6,
        spikeThreshold=0.9,
        vertexCountOvershoot=3,
        preventOneYearRecovery=True,
        recoveryThreshold=0.25,
        pvalThreshold=0.05,
        bestModelProportion=0.75,
        minObservationsNeeded=6,
    ).select("LandTrendr")

    vertex_mask = lt.arraySlice(0, 3, 4)
    vertices_all = lt.arrayMask(vertex_mask)
    n_v = vertices_all.arrayLength(1)
    vertices = vertices_all.updateMask(n_v.gte(2))
    n_v_ok = vertices.arrayLength(1)
    years = vertices.arraySlice(0, 0, 1)
    fitted = vertices.arraySlice(0, 2, 3)

    start_yr = years.arraySlice(1, 0, n_v_ok.subtract(1))
    end_yr = years.arraySlice(1, 1, n_v_ok)
    start_val = fitted.arraySlice(1, 0, n_v_ok.subtract(1))
    end_val = fitted.arraySlice(1, 1, n_v_ok)

    mag = start_val.subtract(end_val)  # positive = NBR drop (disturbance)
    dur = end_yr.subtract(start_yr)
    dist_info = ee.Image.cat([end_yr, mag, dur]).toArray(0)
    sorted_seg = dist_info.arraySort(mag.multiply(-1)).arraySlice(1, 0, 1).arrayProject([0])
    out = sorted_seg.arrayFlatten([["yod", "dist_mag", "dist_dur"]])
    return out.updateMask(out.select("dist_mag").gt(0.1)).clip(aoi)
```

## 4. 结果质检标准

检查 `yod` 是否落在 `[start_year, end_year]` 合理年份且主要连片干扰区与人工解译林火/采伐矢量一致；极多云山区的孤立单像元 `yod` 往往对应云阴影残差，建议后续应用最小面积连通域平滑过滤。
