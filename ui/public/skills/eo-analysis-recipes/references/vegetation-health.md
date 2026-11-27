关键词：同物候基准植被健康指数、VHI、VCI、TCI、vegetation health index、MODIS/061/MOD13Q1、MODIS/061/MOD11A1

## 1. 适用场景与输入要求

输入已去云、同月同网格的当期 NDVI/LST 和多年同月极值影像。先将 250 m NDVI 面积聚合到热产品约 1 km 网格。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

VCI = (NDVI−min)/(max−min)，TCI = (max−LST)/(max−min)。组合时给植被分量权重 α。历史极差为零时屏蔽该像元。 [MODIS/061/MOD13Q1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD13Q1)、[MODIS/061/MOD11A1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD11A1)

## 3. 完整可运行 Python 代码

```python
import ee


def vegetation_health(aoi: ee.Geometry, ndvi: ee.Image, lst: ee.Image,
                      ndvi_min: ee.Image, ndvi_max: ee.Image,
                      lst_min: ee.Image, lst_max: ee.Image,
                      alpha: float = 0.5) -> ee.Image:
    if not 0 < alpha < 1:
        raise ValueError("use a weight strictly between 0 and 1")
    dr = ndvi_max.subtract(ndvi_min)
    dt = lst_max.subtract(lst_min)
    vci = ndvi.subtract(ndvi_min).divide(dr)
    tci = lst_max.subtract(lst).divide(dt)
    health = vci.multiply(alpha).add(tci.multiply(1 - alpha)).multiply(100)
    return health.updateMask(dr.gt(0).And(dt.gt(0))).rename("vhi_pct").clip(aoi)

def vegetation_conditions(aoi: ee.Geometry, ndvi: ee.Image, lst: ee.Image,
                          ndvi_min: ee.Image, ndvi_max: ee.Image,
                          lst_min: ee.Image, lst_max: ee.Image) -> ee.Image:
    dr, dt = ndvi_max.subtract(ndvi_min), lst_max.subtract(lst_min)
    vci = ndvi.subtract(ndvi_min).multiply(100).divide(dr).updateMask(dr.gt(0)).rename("vci_pct")
    tci = lst_max.subtract(lst).multiply(100).divide(dt).updateMask(dt.gt(0)).rename("tci_pct")
    return vci.addBands(tci).clip(aoi)

```

## 4. 结果质检标准

在历史极值包络内为 0–100。超界可能是真实极端事件，应先检查温度单位、基准年份和有效观测数。
