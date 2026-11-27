关键词：ERA5-Land 逐日气候距平、气温与降水距平、ECMWF/ERA5_LAND/DAILY_AGGR、climatology baseline

## 1. 适用场景与输入要求

用于定量评估目标月份相对多年同期气候态基准（如 1991–2020 年）的暖干/冷湿距平，服务于干旱评估、物候异常解释与生态模型气候驱动对齐。输入研究区 `aoi`、目标年份 `target_year`、目标月份 `month` (`1..12`) 与气候基准起止年份 `[base_start_year, base_end_year]`。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **严格单位换算**：`ECMWF/ERA5_LAND/DAILY_AGGR` 中 `temperature_2m` 原始单位为开尔文（K），须减去 `273.15` 转为摄氏度（°C）；`total_precipitation_sum` 原始单位为米/天（m），须乘 `1000.0` 转为毫米/天（mm）。
- **逐月合计与多年同期基准对齐**：气温比较月均温差 `ΔT = T_target - T_base`（°C）；降水距平需先比较全月总降水量再算相对百分距平 `(P_target - P_base) / P_base × 100%`（并屏蔽 `P_base <= 0.5 mm` 的极端干旱零分母区）。格网原分辨率约 `9 km`（`~11132 m` 名义网格），切勿无依据超采样。官方目录：[ECMWF/ERA5_LAND/DAILY_AGGR](https://developers.google.com/earth-engine/datasets/catalog/ECMWF_ERA5_LAND_DAILY_AGGR)。

## 3. 完整可运行 Python 代码

```python
import ee


def era5_land_monthly_anomaly(
    aoi: ee.Geometry,
    target_year: int,
    month: int,
    base_start_year: int = 1991,
    base_end_year: int = 2020,
) -> ee.Image:
    if not 1 <= month <= 12:
        raise ValueError("month must be in 1..12")
    if base_end_year < base_start_year:
        raise ValueError("base_end_year must be >= base_start_year")

    def prep_daily(img: ee.Image) -> ee.Image:
        t_c = img.select("temperature_2m").subtract(273.15).rename("temp_c")
        pr_mm = img.select("total_precipitation_sum").multiply(1000.0).rename("precip_mm")
        return t_c.addBands(pr_mm).copyProperties(img, ["system:time_start"])

    col = ee.ImageCollection("ECMWF/ERA5_LAND/DAILY_AGGR").filterBounds(aoi).map(prep_daily)

    t0 = ee.Date.fromYMD(target_year, month, 1)
    target_col = col.filterDate(t0, t0.advance(1, "month"))
    target_t = target_col.select("temp_c").mean().rename("temp_c")
    target_p = target_col.select("precip_mm").sum().rename("precip_mm")

    base_years = list(range(base_start_year, base_end_year + 1))

    def month_for_year(y):
        s = ee.Date.fromYMD(y, month, 1)
        mcol = col.filterDate(s, s.advance(1, "month"))
        return mcol.select("temp_c").mean().addBands(mcol.select("precip_mm").sum())

    base_ic = ee.ImageCollection.fromImages(ee.List(base_years).map(month_for_year))
    base_t = base_ic.select("temp_c").mean().rename("base_temp_c")
    base_p = base_ic.select("precip_mm").mean().rename("base_precip_mm")

    t_anom = target_t.subtract(base_t).rename("temp_anom_c")
    p_anom_pct = (
        target_p.subtract(base_p)
        .divide(base_p)
        .multiply(100.0)
        .updateMask(base_p.gt(0.5))
        .rename("precip_anom_pct")
    )
    return ee.Image.cat([target_t, target_p, t_anom, p_anom_pct]).clip(aoi)
```

## 4. 结果质检标准

核查 `temp_c` 与 `precip_mm` 量级是否符合研究区当月气候常识；极旱区（基准月均降水 `<= 0.5 mm`）的降水百分距平会被自动掩膜，避免产生数万百分比虚高噪点。
