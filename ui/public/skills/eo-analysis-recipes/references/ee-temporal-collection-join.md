关键词：ee.Join 集合时空配对、saveBest、跨传感器匹配、时间差过滤、空间相交、GEE

## 1. 适用场景与输入要求

当需要为 Sentinel-2 光学合成按最接近过境时间匹配 Sentinel-1 SAR，或给粗网格气象/热红外配对高分辨率影像，或合并共享 ID 的辅助质量波段且严禁在客户端用 `for` 循环逐个请求时使用。输入研究区 `aoi`、主集合与辅集合、最大允许时间差天数 `max_days`。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **时间窗与空间相交双约束**：单独用 `ee.Filter.maxDifference` 配对可能匹配到空间上与当前瓦片毫无重叠的同天远方景！必须用 `ee.Filter.and_(time_filter, ee.Filter.intersects(leftField=".geo", rightField=".geo"))` 联合过滤。
- **剔除未匹配主景与避免 `getInfo()`**：`ee.Join.saveBest(matchKey=..., measureKey=...)` 仅在找到合格辅景时写入 `matchKey` 属性；后续直接用 `.filter(ee.Filter.notNull([matchKey]))` 过滤后在 `.map()` 中提取配对波段，整套流程留在服务器端 DAG 执行。官方指南：[Earth Engine Joins](https://developers.google.com/earth-engine/guides/joins_intro)。

## 3. 完整可运行 Python 代码

```python
import ee


def pair_s2_with_nearest_s1(
    aoi: ee.Geometry,
    start_date: str,
    end_date: str,
    max_days: float = 3.0,
) -> ee.ImageCollection:
    s2 = (
        ee.ImageCollection("COPERNICUS/S2_SR_HARMONIZED")
        .filterBounds(aoi)
        .filterDate(start_date, end_date)
        .filter(ee.Filter.lt("CLOUDY_PIXEL_PERCENTAGE", 40))
    )
    s1 = (
        ee.ImageCollection("COPERNICUS/S1_GRD")
        .filterBounds(aoi)
        .filterDate(start_date, end_date)
        .filter(ee.Filter.eq("instrumentMode", "IW"))
        .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VV"))
        .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VH"))
    )

    max_diff_ms = max_days * 24.0 * 3600.0 * 1000.0
    join_filter = ee.Filter.And(
        ee.Filter.maxDifference(
            difference=max_diff_ms,
            leftField="system:time_start",
            rightField="system:time_start",
        ),
        ee.Filter.intersects(leftField=".geo", rightField=".geo"),
    )

    joined = ee.Join.saveBest(matchKey="nearest_s1", measureKey="time_diff_ms").apply(
        primary=s2, secondary=s1, condition=join_filter
    )

    def attach_bands(primary_img: ee.Image) -> ee.Image:
        sec_img = ee.Image(primary_img.get("nearest_s1")).select(["VV", "VH"], ["s1_VV", "s1_VH"])
        opt = primary_img.select(["B4", "B8", "B11"]).multiply(0.0001)
        diff_hours = ee.Number(primary_img.get("time_diff_ms")).divide(3600000.0)
        return opt.addBands(sec_img).set(
            {
                "system:time_start": primary_img.get("system:time_start"),
                "s1_s2_diff_hours": diff_hours,
            }
        )

    return ee.ImageCollection(joined.filter(ee.Filter.notNull(["nearest_s1"]))).map(attach_bands)
```

## 4. 结果质检标准

检查返回集合 `.size()`（配对成功后才大于 0）与各景属性 `s1_s2_diff_hours` 最大值应严格不超过 `max_days * 24`；若主/辅集合仅需基于精确相同系统 ID 关联独立 QA 波段（例如 Cloud Score+），应直接用更轻量的 `primary.linkCollection(secondary, bands)`。
