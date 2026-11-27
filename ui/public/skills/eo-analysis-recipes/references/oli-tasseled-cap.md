关键词：OLI 地表反射率缨帽分量、TCT、TCB、TCG、TCW、LANDSAT/LC08/C02/T1_L2

## 1. 适用场景与输入要求

Landsat 8 OLI 六个光学波段，30 m。输入区域内有有效影像的季节窗。

## 2. 核心公式与参数避坑

采用 Zhai 等为 Landsat 8 地表反射率标定的六波段矩阵。QA_PIXEL 位 0–5、QA_RADSAT 掩膜；光学 ×0.0000275−0.2。 [Zhai 等，表 1](https://doi.org/10.1016/j.rse.2022.112992)

## 3. 完整可运行 Python 代码

```python
import ee


def decode_oli(image):
    valid = image.select("QA_PIXEL").bitwiseAnd(63).eq(0).And(image.select("QA_RADSAT").eq(0))
    sr = image.select(["SR_B2", "SR_B3", "SR_B4", "SR_B5", "SR_B6", "SR_B7"],
                      ["blue", "green", "red", "nir", "swir1", "swir2"]).multiply(0.0000275).subtract(0.2)
    return sr.updateMask(valid)


def components(sr):
    names = ["blue", "green", "red", "nir", "swir1", "swir2"]
    matrix = [[.3690, .4271, .4689, .5073, .3824, .2406],
              [-.2870, -.2685, -.4087, .8145, .0637, -.1052],
              [.0382, .2137, .3536, .2270, -.6108, -.6351]]
    vector = sr.select(names).toArray().toArray(1)
    return (ee.Image(ee.Array(matrix)).matrixMultiply(vector)
            .arrayProject([0]).arrayFlatten([["brightness", "greenness", "wetness"]]))

def oli_components(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    raw = ee.ImageCollection("LANDSAT/LC08/C02/T1_L2").filterBounds(aoi).filterDate(start_date, end_date)
    grid = ee.Image(raw.first()).select("SR_B2").projection()
    return raw.map(decode_oli).map(components).median().setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

湿度分量可为负。不要把 TOA 系数或本矩阵直接套入 HLS、TM 或 MSI。
