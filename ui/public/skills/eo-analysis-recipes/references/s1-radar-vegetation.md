关键词：双极化雷达植被指数、Sentinel-1、COPERNICUS/S1_GRD、dual-pol RVI、radar vegetation index

## 1. 适用场景与输入要求

IW、VV/VH 双极化、10 m 网格，限定同一相对轨道和轨向。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

RVI = 4 VH/(VV+VH)，强度先由 dB 转为线性功率，再逐景计算。该经验近似受土壤水分和散射几何影响。 [COPERNICUS/S1_GRD](https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S1_GRD)

## 3. 完整可运行 Python 代码

```python
import ee


def radar_vegetation(aoi: ee.Geometry, start_date: str, end_date: str,
                     orbit: int, orbit_pass: str) -> ee.Image:
    scenes = (ee.ImageCollection("COPERNICUS/S1_GRD").filterBounds(aoi)
              .filterDate(start_date, end_date).filter(ee.Filter.eq("instrumentMode", "IW"))
              .filter(ee.Filter.eq("orbitProperties_pass", orbit_pass))
              .filter(ee.Filter.eq("relativeOrbitNumber_start", orbit))
              .filter(ee.Filter.eq("resolution_meters", 10))
              .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VH"))
              .filter(ee.Filter.listContains("transmitterReceiverPolarisation", "VV")))

    def index(im):
        power = im.select(["VV", "VH"]).divide(10).multiply(ee.Number(10).log()).exp()
        vv, vh = power.select("VV"), power.select("VH")
        return vh.multiply(4).divide(vv.add(vh)).rename("rvi")

    return scenes.map(index).median().clip(aoi)
```

## 4. 结果质检标准

理论范围 0–4。检查山地阴影、叠掩和轨道边缘。时间窗口之间保持一致的几何条件。
