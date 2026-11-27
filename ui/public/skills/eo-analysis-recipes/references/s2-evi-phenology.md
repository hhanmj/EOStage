关键词：单季 EVI 阈值物候、phenology、SOS、EOS、LOS、COPERNICUS/S2_SR_HARMONIZED

## 1. 适用场景与输入要求

输入完整单生长季、含返青前和衰退后的日期窗，S2 蓝红近红外 10 m，SCL 为 20 m。适合单峰植被。

## 2. 核心公式与参数避坑

先按 SCL 去云、反射率 /10000，再算 EVI=2.5(N−R)/(N+6R−7.5B+1)。用季内 P10、P90 定相对阈值，输出观测序列首次及末次达标距起点的天数。

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

def phenology(aoi: ee.Geometry, start_date: str, end_date: str, fraction: float = .3) -> ee.Image:
    if not 0 < fraction < 1:
        raise ValueError("fraction must be between 0 and 1")
    start = ee.Date(start_date)
    def index(image):
        n, r, b = [image.select(x) for x in ("B8", "B4", "B2")]
        den = n.add(r.multiply(6)).subtract(b.multiply(7.5)).add(1)
        evi = n.subtract(r).multiply(2.5).divide(den).rename("evi")
        return (evi.updateMask(den.gt(1e-6).And(evi.gte(-1)).And(evi.lte(1)))
                .set("utc_day", image.date().format("yyyy-MM-dd")))

    scenes = clear_s2(aoi, start_date, end_date).map(index)
    grid = ee.Image(scenes.first()).projection()
    days = ee.List(scenes.aggregate_array("utc_day")).distinct().sort()

    def daily(key):
        evi = scenes.filter(ee.Filter.eq("utc_day", key)).median()
        day = ee.Date(key).difference(start, "day").floor()
        day_band = evi.multiply(0).add(day).toInt32().rename("day")
        return evi.addBands(day_band).setDefaultProjection(grid)

    series = ee.ImageCollection.fromImages(days.map(daily))
    bounds = series.select("evi").reduce(ee.Reducer.percentile([10, 90]))
    amp = bounds.select(1).subtract(bounds.select(0))
    cut = bounds.select(0).add(amp.multiply(fraction))
    def dates(image):
        return image.select("day").updateMask(image.select("evi").gte(cut))
    active = series.map(dates)
    sos, eos = active.min(), active.max()
    ends = series.select("day")
    n = series.select("evi").count().rename("n")
    ok = n.gte(6).And(amp.gt(.1)).And(sos.gt(ends.min())).And(eos.lt(ends.max()))
    return (sos.rename("sos_day").addBands(eos.rename("eos_day"))
            .addBands(eos.subtract(sos).rename("los_days")).updateMask(ok).addBands(n).setDefaultProjection(grid).clip(aoi))
```

## 4. 结果质检标准

应有 0 ≤ SOS ≤ EOS < 窗口天数。n 为不同 UTC 有效观测日数。观测不规则会导致日期误差，检查阈值附近云缺口、多峰、异常值；阈值和最小振幅须按地类检验。
