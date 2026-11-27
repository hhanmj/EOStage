关键词：光谱阈值布尔掩膜、Boolean mask、NDVI、updateMask、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

输入已缩放的单幅 S2 地表反射率和 AOI。10 m B4、B8。

## 2. 核心公式与参数避坑

波段减法求差，加法求总量，以有效正分母计算 NDVI。用逐像元 And 阈值，保留未知区域掩膜。

## 3. 完整可运行 Python 代码

```python
import ee


def vegetation_mask(aoi: ee.Geometry, sr: ee.Image, threshold: float = .35) -> ee.Image:
    if not -1 <= threshold <= 1:
        raise ValueError("threshold must be in [-1, 1]")
    red, nir = sr.select("B4"), sr.select("B8")
    den = nir.add(red)
    valid = den.gt(1e-6).And(red.gte(0)).And(nir.gte(0))
    return nir.subtract(red).divide(den).gte(threshold).updateMask(valid).rename("vegetated").clip(aoi)
```

## 4. 结果质检标准

输出有效区只有 0 和 1，检查阴影和作物物候敏感性。
