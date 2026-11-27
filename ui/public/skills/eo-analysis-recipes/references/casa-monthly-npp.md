关键词：月尺度光能利用率模型、CASA、NPP、net primary productivity、light use efficiency

## 1. 适用场景与输入要求

输入对齐的月总短波辐射 MJ/m²、FPAR、当地植被 εmax (g C/MJ PAR)、两个温度胁迫及水分胁迫因子。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

APAR = 短波辐射 × 0.5 × FPAR，NPP = APAR × εmax × T1 × T2 × W。0.5 是 PAR 占短波的近似；日均 W/m² 必须先按月份长度积分。

## 3. 完整可运行 Python 代码

```python
import ee


def casa_npp(aoi: ee.Geometry, radiation: ee.Image, fpar: ee.Image,
             epsilon: ee.Image, t1: ee.Image, t2: ee.Image, water: ee.Image) -> ee.Image:
    efficiency = epsilon.multiply(t1).multiply(t2).multiply(water)
    valid = radiation.gte(0).And(epsilon.gt(0))
    for factor in (fpar, t1, t2, water):
        valid = valid.And(factor.gte(0)).And(factor.lte(1))
    npp = radiation.multiply(0.5).multiply(fpar).multiply(efficiency)
    return npp.updateMask(valid).rename("npp_gC_m2_month").clip(aoi)
```

## 4. 结果质检标准

为零的 FPAR 或胁迫因子应给出零 NPP。年度合计要求全年覆盖，以同单位独立 NPP 产品或样地数据检查偏差。
