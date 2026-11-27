# 时间序列谐波回归与非线性趋势检验 (Harmonic Regression & Trend Analysis)

## 1. 谐波傅里叶模型拟合季节物候

对于多年度 NDVI / EVI 序列，建立线性趋势 + 一阶谐波模型：
$$\text{NDVI}(t) = \beta_0 + \beta_1 t + \beta_2 \cos(2\pi \omega t) + \beta_3 \sin(2\pi \omega t)$$
其中时间 $t$ 以参考零点起算的**小数年**表示，$\omega = 1$（年周期）。振幅 $A = \sqrt{\beta_2^2 + \beta_3^2}$，相位 $\phi = \text{atan2}(\beta_3, \beta_2)$。

```python
import ee
import math

def add_harmonic_independents(img: ee.Image, epoch_ms: int = 1420070400000) -> ee.Image:
    # epoch_ms 默认 2015-01-01T00:00:00Z
    years = ee.Number(img.date().millis()).subtract(epoch_ms).divide(365.25 * 24 * 3600 * 1000)
    time_rad = ee.Image.constant(years.multiply(2 * math.pi))
    ndvi = img.normalizedDifference(["B8", "B4"]).rename("NDVI")
    return (
        img.addBands(ee.Image.constant(1).rename("constant"))
        .addBands(ee.Image.constant(years).float().rename("t"))
        .addBands(time_rad.cos().float().rename("cos"))
        .addBands(time_rad.sin().float().rename("sin"))
        .addBands(ndvi.float())
    )

def fit_harmonic_ols(col: ee.ImageCollection) -> ee.Image:
    independents = ee.List(["constant", "t", "cos", "sin"])
    harm_col = col.map(add_harmonic_independents)
    trend = harm_col.select(independents.add("NDVI")).reduce(
        ee.Reducer.linearRegression(independents.length(), 1)
    )
    coeffs = trend.select("coefficients").arrayProject([0]).arrayFlatten([independents])
    cos_b = coeffs.select("cos")
    sin_b = coeffs.select("sin")
    amplitude = cos_b.hypot(sin_b).rename("amplitude")
    phase = sin_b.atan2(cos_b).rename("phase")
    return coeffs.addBands([amplitude, phase])
```

## 2. Theil-Sen 斜率估计与 Mann-Kendall 单调趋势

对年度合成中值或峰值序列，推荐使用非参数 `ee.Reducer.sensSlope()` 规避异常云残留脉冲对 OLS 的污染；`kendallsCorrelation(2)` 必须传入自变量与因变量双波段（如 `["year", "NDVI"]`）：

```python
def annual_sens_trend(yearly_ndvi_col: ee.ImageCollection) -> ee.Image:
    # 每个影像必须带有以年份为单位的波段 'year' 与指标 'NDVI'
    sens = yearly_ndvi_col.select(["year", "NDVI"]).reduce(ee.Reducer.sensSlope())
    mk = yearly_ndvi_col.select(["year", "NDVI"]).reduce(ee.Reducer.kendallsCorrelation(2))
    return sens.addBands(mk)
```

## 3. 时序模型选型与 `eo-analysis-recipes` 配方分工

| 研究目标 | 推荐算法 | 适用特征 | 对应独立可执行配方 |
| --- | --- | --- | --- |
| 周期物候振幅与相位拟合 | 一阶谐波 OLS | 提取生长季峰值期、多季振幅 | 本章第 1 节 + `eo-analysis-recipes/references/s2-evi-phenology.md` |
| 年际长期单调趋势与显著性 | Theil-Sen + Mann-Kendall | 抗云噪声干扰的长时序变化速率 | 本章第 2 节 + `eo-analysis-recipes/references/sen-slope.md` |
| 年度扰动断代与恢复历时 | `LandTrendr` | 每年一景合成，定位火灾/采伐爆发年与恢复时长 | `eo-analysis-recipes/references/landtrendr-disturbance.md` |
| 亚年度高频突变断点 | `CCDC` | 保留全晴空观测、分季节拟合谐波检测细粒度突变 | `eo-analysis-recipes/references/ccdc-harmonic-breakpoints.md` |
