关键词：年度净初级生产力、NPP、MODIS/061/MOD17A3HGF、MOD17

## 1. 适用场景与输入要求

年度 500 m MOD17A3HGF 净初级生产力，选择有成品的年份。

## 2. 核心公式与参数避坑

Npp ×0.0001 得 kg C/m²/年。年度净固碳输入不能由年 GPP 直接替代；年产品也不能当月值重复累加。 [官方目录](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD17A3HGF)

## 3. 完整可运行 Python 代码

```python
import ee


def annual_npp(aoi: ee.Geometry, year: int) -> ee.Image:
    start = ee.Date.fromYMD(year,1,1)
    im = ee.Image(ee.ImageCollection("MODIS/061/MOD17A3HGF").filterDate(start,start.advance(1,"year")).first())
    return im.select("Npp").multiply(.0001).rename("npp_kgC_m2_year").addBands(
        im.select("Npp_QC").rename("gapfilled_pct")).clip(aoi)
```

## 4. 结果质检标准

Npp_QC 指输入 LAI/FPAR 中填补比例；检查高填补区与非植被掩膜。与 NDVI/FVC 合并时保持年度与网格一致。
