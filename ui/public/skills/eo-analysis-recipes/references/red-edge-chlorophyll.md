关键词：红边叶绿素代理指数、CIred-edge、Sentinel-2 MSI、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

使用 B5 与 B8A 的 20 m 像元；优先分析无云、植被覆盖的生长季。

## 2. 核心公式与参数避坑

CIred-edge=B8A/B5−1。逐景先除以 10000，再计算比例。它是叶绿素代理；反演绝对含量需要现场标定。

## 3. 完整可运行 Python 代码

```python
import ee


def clear_s2(aoi, start_date, end_date):
    def decode(image):
        scl = image.select("SCL")
        clear = scl.eq(4).Or(scl.eq(5)).Or(scl.eq(6))
        return (image.select("B.*").divide(10000).updateMask(clear)
                .copyProperties(image, ["system:time_start"]))
    return (ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED")
            .filterDate(start_date, end_date).filterBounds(aoi).map(decode))

def red_edge(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    def index(im):
        red = im.select("B5")
        return im.select("B8A").divide(red).subtract(1).updateMask(red.gt(.001)).rename("ci_re")
    values = clear_s2(aoi,start_date,end_date).map(index)
    return values.median().addBands(values.count().rename("n")).clip(aoi)
```

## 4. 结果质检标准

比较冠层类型与叶绿素实测，检查云残留、低反射率分母和混合像元；同时评估水分时另算 NDMI。
