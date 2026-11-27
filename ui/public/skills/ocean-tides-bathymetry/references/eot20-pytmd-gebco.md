# EOT20 海洋潮汐、潮汐负荷改正与 GEBCO/ETOPO 水深集成

## 1. 沿海与海岛 InSAR 中的海洋潮汐负荷 (OTL) 与瞬时水位改正

近海三角洲、港口填海区、跨海大桥及极地冰架 Interferometry（Sentinel-1 / NISAR）必须扣除**固体潮（Solid Earth Tide）与海洋潮汐负荷位移（Ocean Tide Loading, OTL）**：
- 即使距海岸线 $50\sim 100\,\text{km}$，大潮汐幅度区（如黄渤海、杭州湾、圣米歇尔湾）垂直负荷形变仍可达 $2\sim 6\,\text{cm}$，在差分干涉中会表现为大尺度虚假形变梯度。
- 对于红树林/滩涂水位反演，需用 EOT20 或 FES2014 精确计算两景 SAR 成像精确 UTC 时刻的潮高差 $\Delta h_{\text{tide}}(t_2, t_1)$。

## 2. `pyTMD` + EOT20 标准 Python 示例

```python
import pyTMD.io
import pyTMD.predict
import pyTMD.time
import numpy as np

def predict_eot20_tide(lon: np.ndarray, lat: np.ndarray, utc_times: list[str], model_dir: str):
    """基于 DGFI-TUM EOT20 预测指定经纬度与影像成像时刻瞬时海面潮高（米）"""
    model = pyTMD.io.model(model_dir, format="netcdf").elevation("EOT20")
    tide_time = pyTMD.time.convert_calendar_dates(utc_times)
    amp, ph, constituents = pyTMD.io.FES.extract_constants(
        lon, lat, model.model_file, type=model.type, version=model.version,
        method="spline", scale=model.scale
    )
    return pyTMD.predict.time_series(tide_time, amp, ph, constituents)
```

## 3. GEBCO vs ETOPO 2022 选型说明

- **GEBCO (`science.gebco.bathymetry-2026.v1`)**：国际水深权威联合编图计划，含 TID 网格来源标识符，适合海域水深分级与大陆坡掩膜；
- **ETOPO 2022 (`science.noaa.etopo-2022.v1`)**：同步提供 `Bed`（基岩高程）与 `Ice Surface`（冰盖表面），在格陵兰/南极及陆海过渡带推荐优先使用。
