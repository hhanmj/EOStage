关键词：同轨坡地雷达异常筛查、Sentinel-1、landslide candidates、MAD、COPERNICUS/S1_GRD

## 1. 适用场景与输入要求

相同相对轨道、极化、观察方向。输入坡度和事件前后时间窗，前期至少五次有效观测。

## 2. 核心公式与参数避坑

相对背景中位数计算有符号变化，以 1.4826×MAD 稳健标准化。允许滑坡信号变亮或变暗。

## 3. 完整可运行 Python 代码

```python
import ee


def slope_anomaly(aoi: ee.Geometry, start_date: str, event_date: str, end_date: str,
                  orbit: int, direction: str, slope: ee.Image, threshold: float = 3) -> ee.Image:
    images = (ee.ImageCollection("COPERNICUS/S1_GRD").filterBounds(aoi)
              .filter(ee.Filter.eq("instrumentMode", "IW"))
              .filter(ee.Filter.eq("relativeOrbitNumber_start", orbit))
              .filter(ee.Filter.eq("orbitProperties_pass", direction))
              .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VV")).select("VV"))
    prior = images.filterDate(start_date, event_date)
    center = prior.median()
    noise = prior.map(lambda im: im.subtract(center).abs()).median().multiply(1.4826)
    delta = images.filterDate(event_date, end_date).median().subtract(center)
    z = delta.divide(noise.max(.5)).updateMask(prior.count().gte(5)).rename("robust_z")
    candidates = z.abs().gte(threshold).And(slope.gte(10)).rename("slope_candidate")
    return z.addBands(candidates).setDefaultProjection(ee.Image(prior.first()).projection()).clip(aoi)
```

## 4. 结果质检标准

结合叠掩阴影图、降雨、光学或滑坡清单验证。低背景噪声、植被季节性、积雪及采伐均会产生异常。
