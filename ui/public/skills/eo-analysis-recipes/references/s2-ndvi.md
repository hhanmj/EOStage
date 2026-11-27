关键词：植被绿度中位数、Sentinel-2、COPERNICUS/S2_SR_HARMONIZED、NDVI normalized difference vegetation index

## 1. 适用场景与输入要求

使用 B8 和 B4，分析网格 10 m，SCL 为 20 m。选定物候或水文窗口，需有覆盖 AOI 的观测。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

每景反射率先除以 10000，再算 (B8 − B4) / (B8 + B4)，最后逐像元取中位数。SCL 白名单排除云、云影和雪。 [COPERNICUS/S2_SR_HARMONIZED](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S2_SR_HARMONIZED)

## 3. 完整可运行 Python 代码

```python
import ee


def build_s2_ndvi(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED")
              .filterBounds(aoi).filterDate(start_date, end_date))

    def index(image):
        scl = image.select("SCL")
        clear = scl.eq(4).Or(scl.eq(5)).Or(scl.eq(6))
        left = image.select("B8").divide(10000)
        right = image.select("B4").divide(10000)
        total = left.add(right)
        return (left.subtract(right).divide(total).rename("ndvi")
                .updateMask(clear.And(total.gt(0))))

    values = scenes.map(index)
    return values.median().addBands(values.count().rename("n_obs")).clip(aoi)
```

## 4. 结果质检标准

NDVI 在 [-1,1]，稠密绿植通常为正。裸地也可能为正，不能仅据符号划分森林。 同时查看 n_obs，单景有效像元不能用于判断时序稳定性。
