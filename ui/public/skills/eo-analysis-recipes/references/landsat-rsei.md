关键词：Landsat 陆地生态指数 PCA、RSEI、NDVI、WET、NDBSI、LST、LANDSAT/LC08/C02/T1_L2

## 1. 适用场景与输入要求

Landsat 8 L2SP，统一季节、地类范围、30 m 等面积计算格网；AOI 必须有充足共同有效像元且四指标均有变异。

## 2. 核心公式与参数避坑

光学与温度分别按 C2 官方系数处理，排除 QA 水体及 MNDWI>0 区域。四指标截尾归一化、中心化后取协方差 PC1，以绿湿正、干热负的总体方向统一符号。 [PCA](https://developers.google.com/earth-engine/guides/arrays_eigen_analysis) · [OLI SR 系数](https://doi.org/10.1016/j.rse.2022.112992)

## 3. 完整可运行 Python 代码

```python
import ee


def decode_oli(image):
    valid = image.select("QA_PIXEL").bitwiseAnd(63).eq(0).And(image.select("QA_RADSAT").eq(0))
    sr = image.select(["SR_B2", "SR_B3", "SR_B4", "SR_B5", "SR_B6", "SR_B7"],
                      ["blue", "green", "red", "nir", "swir1", "swir2"]).multiply(0.0000275).subtract(0.2)
    temp = image.select("ST_B10").multiply(0.00341802).add(149.0).rename("lst_k")
    return sr.addBands(temp).updateMask(valid).copyProperties(image, ["system:time_start"])


def components(sr):
    names = ["blue", "green", "red", "nir", "swir1", "swir2"]
    matrix = [[.3690, .4271, .4689, .5073, .3824, .2406],
              [-.2870, -.2685, -.4087, .8145, .0637, -.1052],
              [.0382, .2137, .3536, .2270, -.6108, -.6351]]
    vector = sr.select(names).toArray().toArray(1)
    return (ee.Image(ee.Array(matrix)).matrixMultiply(vector)
            .arrayProject([0]).arrayFlatten([["brightness", "greenness", "wetness"]]))


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


def landsat_rsei(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("LANDSAT/LC08/C02/T1_L2").filterBounds(aoi)
              .filterDate(start_date, end_date).filter(ee.Filter.eq("PROCESSING_LEVEL", "L2SP")))
    def land(im):
        return ee.Image(decode_oli(im)).updateMask(im.select("QA_PIXEL").bitwiseAnd(128).eq(0))
    sr = scenes.map(land).median()
    green = difference(sr.select("nir"), sr.select("red")).rename("green")
    land_mask = difference(sr.select("green"), sr.select("swir1")).lte(0)
    x = ee.Image.cat([green, components(sr).select("wetness").rename("wet"),
                      dryness(sr), sr.select("lst_k").rename("heat")]).updateMask(land_mask)
    return pca_rsei(x, aoi, ee.Projection("EPSG:6933").atScale(30))
```

## 4. 结果质检标准

输出 [0,1]。核对载荷方向和第一特征值占比，方向近零或生态含义冲突时不解释 PC1。逐年独立拟合的指数仅为各年相对值；比较绝对变化时固定标尺、载荷和掩膜。
