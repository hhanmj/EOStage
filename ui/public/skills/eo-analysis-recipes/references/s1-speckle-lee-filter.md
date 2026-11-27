关键词：Sentinel-1 相干斑噪声滤波、Lee 局部自适应滤波、线性功率域、COPERNICUS/S1_GRD、ENL

## 1. 适用场景与输入要求

需要对 Sentinel-1 IW GRD 雷达强度图进行保边缘降斑（减少盐胡椒噪声同时不糊化水体边界、道路与耕地轮廓）时使用。输入研究区 `aoi`、日期窗口 `[start_date, end_date)`、固定轨道方向（`"ASCENDING"` 或 `"DESCENDING"`）、目标极化（`"VV"` 或 `"VH"`）及窗口半径像素数（常用 `2`，即 `5×5` 窗）与等效视数（IW 10 m GRD 典型值 `ENL = 4.4`）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **严禁直接在分贝（dB）域做 Lee 滤波与空间均值**：`COPERNICUS/S1_GRD` 影像默认为对数分贝尺度 `dB`，而雷达相干斑是乘性噪声，必须在转回**线性强度功率域**（`I = 10^(dB / 10)`）后再执行局部均值、方差估计与经典 Lee（1980）滤波，处理完毕后再转换回 `dB` 导出。
- **入射角与轨道一致性**：混合升轨与降轨会引发斜坡几何透视收缩和叠掩混杂；同时应剔除侧视边缘入射角极端值（保留典型有效区 `29°–46°`）抑制条带边缘畸变。官方目录：[COPERNICUS/S1_GRD](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S1_GRD)。

## 3. 完整可运行 Python 代码

```python
import ee


def s1_lee_speckle_filter(
    aoi: ee.Geometry,
    start_date: str,
    end_date: str,
    orbit_pass: str = "DESCENDING",
    polarization: str = "VV",
    radius_px: int = 2,
    enl: float = 4.4,
) -> ee.Image:
    if orbit_pass not in ("ASCENDING", "DESCENDING"):
        raise ValueError("orbit_pass must be 'ASCENDING' or 'DESCENDING'")
    if polarization not in ("VV", "VH"):
        raise ValueError("polarization must be 'VV' or 'VH'")

    col = (
        ee.ImageCollection("COPERNICUS/S1_GRD")
        .filterBounds(aoi)
        .filterDate(start_date, end_date)
        .filter(ee.Filter.eq("instrumentMode", "IW"))
        .filter(ee.Filter.eq("orbitProperties_pass", orbit_pass))
        .filter(ee.Filter.listContains("transmitterReceiverPolarisation", polarization))
    )

    kernel = ee.Kernel.square(radius=radius_px, units="pixels")
    sigma_v2 = 1.0 / enl

    def filter_one(img: ee.Image) -> ee.Image:
        angle_mask = img.select("angle").gt(29.0).And(img.select("angle").lt(46.0))
        lin = ee.Image(10.0).pow(img.select(polarization).divide(10.0)).updateMask(angle_mask)

        stats = lin.reduceNeighborhood(
            reducer=ee.Reducer.mean().combine(ee.Reducer.variance(), sharedInputs=True),
            kernel=kernel,
        )
        mean = stats.select(f"{polarization}_mean")
        var_z = stats.select(f"{polarization}_variance")

        # Classic Lee (1980) multiplicative speckle weight clamped to [0, 1]
        weight = (
            var_z.subtract(mean.pow(2).multiply(sigma_v2))
            .divide(var_z.multiply(1.0 + sigma_v2))
            .clamp(0.0, 1.0)
        )
        filtered_lin = mean.add(weight.multiply(lin.subtract(mean))).max(1e-12)
        filtered_db = filtered_lin.log10().multiply(10.0).rename(f"{polarization}_lee_db")
        return filtered_db.copyProperties(img, ["system:time_start", "relativeOrbitNumber_start"])

    filtered_col = col.map(filter_one)
    # Average in linear domain across passes, then convert back to dB
    mean_lin = filtered_col.map(lambda im: ee.Image(10.0).pow(im.divide(10.0))).mean()
    return mean_lin.max(1e-12).log10().multiply(10.0).rename(f"{polarization}_lee_db").clip(aoi)
```

## 4. 结果质检标准

对比滤波前后同质水面或均匀裸土样本区的标准差应显著下降，同时水陆交界线或建筑物角反射器亮斑不被过度抹平；典型 C 波段水体与平稳地物在 `[-25, 0] dB` 区间内。
