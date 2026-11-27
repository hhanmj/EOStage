关键词：样地标定叶绿素或冠层水分、Cab、Cw、CIred-edge、NDMI、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

提供与 S2 观测同期的样地、目标性状与单位。叶绿素使用 B8A/B5（20 m），水分使用 B8A/B11（20 m）。fit_rows 中 proxy 和 observed 来自标定区。

## 2. 核心公式与参数避坑

叶绿素代理为 B8A/B5−1，水分代理为 (B8A−B11)/(B8A+B11)。对每种性状单独拟合斜率和截距，单位继承实测性状。

## 3. 完整可运行 Python 代码

```python
import ee


def calibrated_trait(aoi: ee.Geometry, sr: ee.Image, trait: str,
                     fit_rows: ee.FeatureCollection, unit: str) -> ee.Image:
    nir = sr.select("B8A")
    if trait == "cab":
        red_edge = sr.select("B5")
        proxy = nir.divide(red_edge).subtract(1).updateMask(red_edge.gt(1e-6))
    elif trait == "cw":
        swir = sr.select("B11")
        den = nir.add(swir)
        proxy = nir.subtract(swir).divide(den).updateMask(den.gt(1e-6))
    else:
        raise ValueError("trait must be cab or cw")
    fit = fit_rows.filter(ee.Filter.notNull(["proxy", "observed"])).reduceColumns(
        ee.Reducer.linearFit(), ["proxy", "observed"])
    valid = nir.gte(0)
    return (proxy.multiply(ee.Number(fit.get("scale"))).add(ee.Number(fit.get("offset")))
            .updateMask(valid).rename(trait).clip(aoi).set("unit", unit))
```

## 4. 结果质检标准

使用空间独立样地检查偏差与 RMSE。训练代理和影像须采用同一波段公式，跨季节、地类和叶面积条件另行验证，检查训练范围外的样点。
