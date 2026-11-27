关键词：缨帽分量生物物理组成指数、BCI、biophysical composition index、LANDSAT/LC08/C02/T1_L2

## 1. 适用场景与输入要求

输入同一 OLI SR 影像的 brightness、greenness、wetness，以及按固定参照区间标定的三组上下限。

## 2. 核心公式与参数避坑

各分量单独归一化，H=亮度，V=绿度，L=湿度；BCI=((H+L)/2−V)/((H+L)/2+V)。跨期比较共用校准范围。

## 3. 完整可运行 Python 代码

```python
import ee


def bci(aoi: ee.Geometry, tc: ee.Image, bounds: dict) -> ee.Image:
    def normalize(name):
        low, high = bounds[name]
        if low >= high:
            raise ValueError("reference bounds must increase")
        return tc.select(name).unitScale(low,high).clamp(0,1)
    background = normalize("brightness").add(normalize("wetness")).multiply(.5)
    veg = normalize("greenness")
    den = background.add(veg)
    return background.subtract(veg).divide(den).updateMask(den.gt(1e-6)).rename("bci").clip(aoi)
```

## 4. 结果质检标准

BCI 在 [−1,1]。检查不透水面与裸土混淆；报告归一化范围及被截断像元的比例。
