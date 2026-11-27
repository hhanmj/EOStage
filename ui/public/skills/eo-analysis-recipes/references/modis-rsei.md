关键词：MODIS 公共网格生态指数、RSEI、MODIS/061/MCD43A4、MODIS/061/MOD11A2、PCA

## 1. 适用场景与输入要求

同一生长季 500 m NBAR 与 1 km 晴空白昼 LST。输入区域须有四指标非退化的陆地样本。

## 2. 核心公式与参数避坑

湿度使用 MODIS NBAR 七波段系数；先对反射率按原生投影聚合到原生 LST 网格，再构建 PCA，避免把热信息上采样成虚假的精细变化。 [MODIS 湿度系数](https://andrewsforest.oregonstate.edu/sites/default/files/lter/pubs/pdf/pub4133.pdf) · [NBAR](https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MCD43A4)

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


def difference(a, b):
    total = a.add(b)
    return a.subtract(b).divide(total).updateMask(total.abs().gt(1e-6))


def pca_rsei(inputs, aoi, grid):
    # 输入仅保留共同有效、已排除水体的陆地像元。
    names = ["green", "wet", "dry", "heat"]
    x = inputs.select(names)
    x = x.updateMask(x.mask().reduce(ee.Reducer.min()))
    args = dict(geometry=aoi, crs=grid, maxPixels=1e9, tileScale=4)
    def stretch(image, bands):
        p = image.reduceRegion(ee.Reducer.percentile([2, 98]), **args)
        result = []
        for band in bands:
            lo = ee.Number(p.get(band + "_p2"))
            span = ee.Number(p.get(band + "_p98")).subtract(lo)
            result.append(image.select(band).subtract(lo).divide(span.max(1e-10))
                          .clamp(0, 1).updateMask(ee.Image.constant(span.gt(0))))
        return ee.Image.cat(result).rename(bands)
    unit = stretch(x, names)
    means = unit.reduceRegion(ee.Reducer.mean(), **args)
    centered = unit.subtract(ee.Image.constant(means.values(names)).rename(names))
    cov = centered.toArray().reduceRegion(ee.Reducer.centeredCovariance(), **args)
    eigen = ee.Array(cov.get("array")).eigen()
    coeff = eigen.slice(0, 0, 1).slice(1, 1)
    pc = (ee.Image(coeff).matrixMultiply(centered.toArray().toArray(1))
          .arrayGet([0, 0]).rename("pc1"))
    orient = (ee.Number(coeff.get([0, 0])).add(coeff.get([0, 1]))
              .subtract(coeff.get([0, 2])).subtract(coeff.get([0, 3])))
    signed = pc.multiply(ee.Number(ee.Algorithms.If(orient.gte(0), 1, -1)))
    return (stretch(signed, ["pc1"]).rename("rsei").setDefaultProjection(grid).clip(aoi)
            .set("loadings_raw", coeff.toList(), "orientation_score", orient,
                 "eigenvalues", eigen.slice(1, 0, 1).toList()))


def modis_rsei(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    ref = (ee.ImageCollection("MODIS/061/MCD43A4").filterBounds(aoi)
           .filterDate(start_date, end_date))
    thermal = (ee.ImageCollection("MODIS/061/MOD11A2").filterBounds(aoi)
               .filterDate(start_date, end_date))
    bands = ["red","nir","blue","green","swir_m","swir1","swir2"]
    def reflect(im):
        qa = im.select("BRDF_Albedo_Band_Mandatory_Quality_Band.*").reduce(ee.Reducer.max())
        source = [f"Nadir_Reflectance_Band{i}" for i in range(1, 8)]
        return im.select(source, bands).multiply(.0001).updateMask(qa.eq(0))
    def temperature(im):
        q = im.select("QC_Day")
        return im.select("LST_Day_1km").multiply(.02).updateMask(q.bitwiseAnd(3).eq(0).And(q.rightShift(6).eq(0)))
    grid = ee.Image(thermal.first()).select("LST_Day_1km").projection()
    native = ee.Image(ref.first()).select("Nadir_Reflectance_Band1").projection()
    sr = (ref.map(reflect).median().setDefaultProjection(native)
          .reduceResolution(ee.Reducer.mean(), maxPixels=64).reproject(grid))
    weights = ee.Image.constant([.1147,.2489,.2408,.3132,-.3122,-.6416,-.5087]).rename(bands)
    wet = sr.multiply(weights).reduce(ee.Reducer.sum()).rename("wet")
    green = difference(sr.select("nir"), sr.select("red")).rename("green")
    heat = thermal.map(temperature).median().setDefaultProjection(grid).rename("heat")
    land = difference(sr.select("green"), sr.select("swir1")).lte(0)
    x = ee.Image.cat([green, wet, dryness(sr), heat]).updateMask(land)
    return pca_rsei(x, aoi, grid)
```

## 4. 结果质检标准

检查 0–1、共同掩膜、载荷和晴空热样本。连续日期 NBAR 使用重叠观测，不可作为独立重复。
