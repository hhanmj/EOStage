关键词：地表干度 NDBSI、MODIS/061/MOD09A1、soil index、IBI

## 1. 适用场景与输入要求

500 m、8 日地表反射率，适合无积雪的陆地区域。

## 2. 核心公式与参数避坑

NDBSI=(SI+IBI)/2，SI 为土壤指数，IBI 为建筑指数。原材料将干度与烧毁指数 NBR 混用，二者使用的波段和物理解释不同。 [MOD09A1](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD09A1)

## 3. 完整可运行 Python 代码

```python
import ee


def dryness(sr):
    b, g, r, n, s = [sr.select(k) for k in ("blue", "green", "red", "nir", "swir1")]
    def ratio(a, d):
        return a.divide(d).updateMask(d.abs().gt(1e-6))
    si = ratio(s.add(r).subtract(n.add(b)), s.add(r).add(n).add(b))
    a = ratio(s.multiply(2), s.add(n))
    v = ratio(n, n.add(r)).add(ratio(g, g.add(s)))
    ibi = ratio(a.subtract(v), a.add(v))
    return si.add(ibi).multiply(.5).rename("dry")

def modis_dryness(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    def decode(im):
        q = im.select("StateQA")
        clear = q.bitwiseAnd(7).eq(0).And(q.bitwiseAnd(1 << 12).eq(0))
        sr = im.select(["sur_refl_b03","sur_refl_b04","sur_refl_b01","sur_refl_b02","sur_refl_b06"],
                       ["blue","green","red","nir","swir1"]).multiply(.0001)
        return dryness(sr.updateMask(clear).updateMask(sr.reduce(ee.Reducer.min()).gte(0)))
    series = ee.ImageCollection("MODIS/061/MOD09A1").filterDate(start_date, end_date).filterBounds(aoi)
    return series.map(decode).median().clip(aoi)
```

## 4. 结果质检标准

非负反射率下两项归一化比值各在 [−1,1]。检查水体、积雪与分母接近零处。
