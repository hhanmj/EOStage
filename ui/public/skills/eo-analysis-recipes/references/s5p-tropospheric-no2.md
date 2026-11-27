关键词：Sentinel-5P TROPOMI、对流层二氧化氮、大气痕量气体、COPERNICUS/S5P/OFFL/L3_NO2、cloud_fraction

## 1. 适用场景与输入要求

用于城市群排放评估、工业源排查、封控/节假日大气 `NO2` 对比与区域柱浓度监测，时间跨度覆盖 2018-06 至今。输入研究区 `aoi`、时间区间 `[start_date, end_date)` 及有效云量分数阈值（通常 `cloud_fraction < 0.3`）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **禁止一刀切屏蔽全部负值**：清洁背景区或极低排放海面受反演仪器随机噪声影响，会产生小幅负的柱浓度观测（如 `-0.0002 mol/m²`）；若把所有负值全部剔除，会系统性高估背景区的长期平均浓度！官方与科研常规仅排除极端物理异常值（例如 `< -0.001 mol/m²`）并用配套的 `cloud_fraction` 控制高云掩盖。
- **单位换算与尺度**：L3 格网原始单位为 `mol/m²`，数值极小，可视化和表格报告常乘 `1e6` 转为 `µmol/m²`；原产品像元分辨率约 `1.1 km` (`1113.2 m`)，统计区域均值时 `scale` 不应无故加密至 10–30 m。官方目录：[COPERNICUS/S5P/OFFL/L3_NO2](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S5P_OFFL_L3_NO2)。

## 3. 完整可运行 Python 代码

```python
import ee


def s5p_tropospheric_no2(
    aoi: ee.Geometry,
    start_date: str,
    end_date: str,
    max_cloud_fraction: float = 0.30,
) -> ee.Image:
    if not 0.0 < max_cloud_fraction <= 1.0:
        raise ValueError("max_cloud_fraction must be in (0, 1]")

    col = ee.ImageCollection("COPERNICUS/S5P/OFFL/L3_NO2").filterBounds(aoi).filterDate(start_date, end_date)

    def prep(img: ee.Image) -> ee.Image:
        no2 = img.select("tropospheric_NO2_column_number_density")
        cf = img.select("cloud_fraction")
        valid = cf.lt(max_cloud_fraction).And(no2.gte(-0.001))
        return no2.multiply(1e6).updateMask(valid).rename("no2_umol_m2")

    cleaned = col.map(prep)
    mean_no2 = cleaned.mean().clip(aoi)
    obs_count = cleaned.select("no2_umol_m2").count().toInt16().rename("obs_count").clip(aoi)
    return mean_no2.addBands(obs_count)
```

## 4. 结果质检标准

检查 `obs_count` 在多云雨季是否过少；城市核心区均值一般位于 `30–300 µmol/m²`，清洁远郊与海洋在 `5–25 µmol/m²` 左右；注意 GEE 中 L3 产品已完成 QA > 0.75（对流层 NO2）预筛选，不要对单景再错误强制乘错误系数。
