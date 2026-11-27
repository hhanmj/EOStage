关键词：跨代 Landsat 植被覆盖度、FVC、TM、ETM+、OLI、LANDSAT/LT05/C02/T1_L2

## 1. 适用场景与输入要求

输入同物候时间窗及已在研究区验证的土壤、植被 NDVI 端元。30 m；1990 年代需使用早期传感器。

## 2. 核心公式与参数避坑

分别映射 TM/ETM+ 红/近红外 B3/B4 与 OLI B4/B5，C2 L2 均 ×0.0000275−0.2，QA_PIXEL 位 0–5 及 QA_RADSAT 屏蔽；FVC=(NDVI−土壤端元)/(植被端元−土壤端元)。

## 3. 完整可运行 Python 代码

```python
import ee


def landsat_fvc(aoi: ee.Geometry, start_date: str, end_date: str, soil: float, canopy: float) -> ee.Image:
    if not -1 <= soil < canopy <= 1:
        raise ValueError("invalid NDVI endmembers")
    all_scenes = ee.ImageCollection([])
    for satellite in ["LT05","LE07","LC08","LC09"]:
        bands = ["SR_B3","SR_B4"] if satellite in ("LT05","LE07") else ["SR_B4","SR_B5"]
        def ndvi(im):
            clear = im.select("QA_PIXEL").bitwiseAnd(63).eq(0).And(im.select("QA_RADSAT").eq(0))
            sr = im.select(bands,["red","nir"]).multiply(.0000275).subtract(.2)
            n, r = sr.select("nir"),sr.select("red")
            return n.subtract(r).divide(n.add(r)).rename("ndvi").updateMask(clear.And(n.add(r).gt(.01)))
        scenes = ee.ImageCollection(f"LANDSAT/{satellite}/C02/T1_L2").filterDate(start_date, end_date).filterBounds(aoi)
        all_scenes = all_scenes.merge(scenes.map(ndvi))
    fvc = all_scenes.median().subtract(soil).divide(canopy-soil).clamp(0,1)
    return fvc.rename("fvc").addBands(all_scenes.count().rename("n")).clip(aoi)
```

## 4. 结果质检标准

FVC 为 0–1。检查重叠年传感器偏差、LE07 条带和端元饱和比例。该合成只处理波段和尺度；多年定量趋势须验证并校正光谱差异。
