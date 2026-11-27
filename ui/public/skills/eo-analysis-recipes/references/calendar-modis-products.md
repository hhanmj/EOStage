关键词：按月汇总 MODIS 质量合格观测、MODIS/061/MOD11A1、MODIS/061/MOD15A2H、LAI、FPAR、LST

## 1. 适用场景与输入要求

输入有数据的单个日历月。LST 为 1 km 日产品；LAI、FPAR 为 500 m 8 日合成，按合成起始日在当月归属。

## 2. 核心公式与参数避坑

保留均值、最大值和有效观测数。LST 使用 QC_Day 的必需 QA 和误差位，×0.02−273.15；LAI ×0.1、FPAR ×0.01；FparLai_QC 筛主算法与清空像元。

## 3. 完整可运行 Python 代码

```python
import ee


def monthly_product(aoi: ee.Geometry, year: int, month: int, product: str) -> ee.Image:
    catalog = {"lst":("MODIS/061/MOD11A1","LST_Day_1km"), "lai":("MODIS/061/MOD15A2H","Lai_500m"),
               "fpar":("MODIS/061/MOD15A2H","Fpar_500m")}
    dataset, band = catalog[product]
    start = ee.Date.fromYMD(year,month,1)
    def scale(im):
        raw = im.select(band)
        if product == "lst":
            q = im.select("QC_Day")
            return raw.multiply(.02).subtract(273.15).updateMask(q.bitwiseAnd(3).eq(0).And(q.rightShift(6).eq(0)))
        q = im.select("FparLai_QC")
        clear = q.bitwiseAnd(1).eq(0).And(q.rightShift(3).bitwiseAnd(3).eq(0)).And(q.rightShift(5).lte(1))
        return raw.multiply(.1 if product=="lai" else .01).updateMask(clear.And(raw.lte(100)))
    good = ee.ImageCollection(dataset).filterBounds(aoi).filterDate(start,start.advance(1,"month")).map(scale)
    return ee.Image.cat([good.mean(),good.max(),good.count()]).rename(["mean","maximum","n"]).clip(aoi)
```

## 4. 结果质检标准

FPAR 用比例 0–1；LAI 约 0–10。最大值不小于均值；热浪或持续云时注意晴空样本偏差。
