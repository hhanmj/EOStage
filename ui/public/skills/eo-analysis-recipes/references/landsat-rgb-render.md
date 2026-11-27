关键词：真假彩色显示波段、true color、false color、RGB、LANDSAT/LC08/C02/T1_L2

## 1. 适用场景与输入要求

Landsat 8 30 m 光学影像，同期至少一景；调用方选择 natural 或 vegetation。

## 2. 核心公式与参数避坑

先缩放反射率，再对各通道使用同一固定显示范围。输出 RGB 8 位，只用于制图。

## 3. 完整可运行 Python 代码

```python
import ee


def render_rgb(aoi: ee.Geometry, start_date: str, end_date: str, style: str = "natural") -> ee.Image:
    bands = {"natural": ["SR_B4", "SR_B3", "SR_B2"], "vegetation": ["SR_B5", "SR_B4", "SR_B3"]}[style]
    def decode(im):
        clear = im.select("QA_PIXEL").bitwiseAnd(63).eq(0).And(im.select("QA_RADSAT").eq(0))
        return im.select(bands).multiply(.0000275).subtract(.2).updateMask(clear)
    scenes = ee.ImageCollection("LANDSAT/LC08/C02/T1_L2").filterBounds(aoi).filterDate(start_date, end_date)
    return scenes.map(decode).median().visualize(bands=bands, min=0, max=.35).clip(aoi)
```

## 4. 结果质检标准

比较图共享拉伸范围。检查过曝、零值与透明掩膜；假彩色植被应以红色为主。
