关键词：固定同月基准植被条件、VCI、TCI、VHI、MODIS、LST

## 1. 适用场景与输入要求

提供已去云、同网格的独立月度 NDVI/LST 合集，均为 ndvi/lst_k 两波段。日期为每月首日。

## 2. 核心公式与参数避坑

每个月以同月基准极值计算 VCI 和 TCI，再作等权 VHI。至少三个独立基准年像元有效。

## 3. 完整可运行 Python 代码

```python
import ee


def monthly_conditions(aoi: ee.Geometry, year: int, month: int, monthly: ee.ImageCollection,
                       baseline_start: str, baseline_end: str, grid: ee.Projection) -> ee.Image:
    if not 1 <= month <= 12:
        raise ValueError("month must be 1 to 12")
    t0 = ee.Date.fromYMD(year, month, 1)
    observed = ee.Image(monthly.filterDate(t0, t0.advance(1, "month")).first()).select(["ndvi", "lst_k"])
    baseline = monthly.filterDate(baseline_start, baseline_end).filter(
        ee.Filter.calendarRange(month, month, "month")).select(["ndvi", "lst_k"])
    low, high = baseline.min(), baseline.max()
    span = high.subtract(low)
    ok = span.gt(0).And(baseline.count().gte(3))
    score = observed.subtract(low).divide(span).multiply(100).updateMask(ok)
    vci = score.select("ndvi").rename("vci_pct")
    tci = score.select("lst_k").multiply(-1).add(100).rename("tci_pct")
    return vci.addBands(tci).addBands(vci.add(tci).multiply(.5).rename("vhi_pct")).setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

各月供数和质量一致，目标期若超出基准极值可出现低于 0 或高于 100。缺测区域维持掩膜。
