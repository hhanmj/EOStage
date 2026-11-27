关键词：跨源校准填补影像空洞、MODIS、Landsat、gap fill、RSEI、SLC-off

## 1. 适用场景与输入要求

输入同物理量、同季节的单波段细网格目标与粗网格参考，两个投影须反映真实格网。适用较小且有足够共同观测的区域。

## 2. 核心公式与参数避坑

在粗格网聚合目标，仅用覆盖充分的像元拟合 target=a+b×reference，再将预测填入原无效像元。各 RSEI 分量应各自标定；不得直接以另一个传感器的 RSEI 混填。

## 3. 完整可运行 Python 代码

```python
import ee


def fill_gaps(aoi: ee.Geometry, fine: ee.Image, coarse: ee.Image,
              fine_grid: ee.Projection, coarse_grid: ee.Projection) -> ee.Image:
    y = fine.rename("y").setDefaultProjection(fine_grid)
    x = coarse.rename("x").setDefaultProjection(coarse_grid)
    def aggregate(im):
        return im.reduceResolution(ee.Reducer.mean(),maxPixels=4096).reproject(coarse_grid)
    coverage = aggregate(y.mask().unmask(0))
    train = x.addBands(aggregate(y)).updateMask(coverage.gte(.9))
    fit = train.reduceRegion(ee.Reducer.linearFit(),aoi,crs=coarse_grid,
                             maxPixels=1e8,tileScale=4)
    pred = x.multiply(ee.Number(fit.get("scale"))).add(ee.Number(fit.get("offset"))).resample("bilinear")
    result = y.unmask(pred).rename("value")
    flag = y.mask().unmask(0).eq(0).updateMask(result.mask()).rename("filled")
    return result.addBands(flag).clip(aoi)
```

## 4. 结果质检标准

拟合需要有效重叠区以及非恒定参考值。用人为遮盖的保留区评估 RMSE、偏差；填补标志不得参与观测数。插值区域仍只有粗尺度信息。
