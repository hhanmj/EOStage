关键词：单配对邻域反射率变化转移、spatiotemporal fusion、STARFM family、Landsat、MODIS

## 1. 适用场景与输入要求

输入 t₀ 细分辨率反射率、t₀/t₁ 两期粗分辨率反射率，三幅单波段同波谱域且已去云，提供米制细网格。

## 2. 核心公式与参数避坑

采用 STARFM 思路的简化单配对模型。由滑动窗口约束空间范围，以参考期细像元的光谱相似性加权，转移粗尺度时间变化。校准需外部数据和配对场景。

## 3. 完整可运行 Python 代码

```python
import ee


def transfer_change(aoi: ee.Geometry, fine0: ee.Image, coarse0: ee.Image, coarse1: ee.Image,
                    grid: ee.Projection, radius: int = 2, reflectance_sigma: float = .04) -> ee.Image:
    if not 1 <= radius <= 5 or reflectance_sigma <= 0:
        raise ValueError("invalid radius or spectral scale")
    fine = fine0.rename("v").reproject(grid)
    kernel = ee.Kernel.square(radius, "pixels", False)
    low0 = coarse0.rename("v").reproject(grid)
    low1 = coarse1.rename("v").reproject(grid)
    changes = low1.subtract(low0).neighborhoodToBands(kernel)
    neighbors = fine.neighborhoodToBands(kernel)
    sigma = float(reflectance_sigma)
    similarity = neighbors.subtract(fine).divide(sigma).pow(2).multiply(-.5).exp()
    prediction = neighbors.add(changes)
    weight = similarity.updateMask(prediction.mask())
    total = weight.reduce(ee.Reducer.sum())
    estimate = prediction.multiply(weight).reduce(ee.Reducer.sum()).divide(total)
    return estimate.updateMask(total.gt(1e-6)).rename("predicted_reflectance").setDefaultProjection(grid).clip(aoi)
```

## 4. 结果质检标准

保留预测不确定度。独立细分辨率 t₁ 对照计算 MAE，按地类检查突变、边缘与粗网格块状伪影。
