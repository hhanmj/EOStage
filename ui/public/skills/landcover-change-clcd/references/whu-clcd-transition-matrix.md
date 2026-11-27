# 武汉大学 CLCD 土地利用转移矩阵与形变风险暴露计算

## 1. 两期土地利用转移矩阵（Transition Matrix）与 Sankey 统计

设初始年份类别矩阵为 $L_{t_1} \in \{1..9\}$，末期类别矩阵为 $L_{t_2} \in \{1..9\}$，注意应先投影至等面积投影（如 `EPSG:4524` CGCS2000 Albers / UTM）再按像元面积统计（切忌在经纬度 `EPSG:4326` 下直接数像元）。

```python
import numpy as np
import rasterio

def compute_clcd_transition_matrix(path_t1: str, path_t2: str, pixel_area_km2: float = 0.0009):
    with rasterio.open(path_t1) as s1, rasterio.open(path_t2) as s2:
        a1 = s1.read(1).astype(np.int32)
        a2 = s2.read(1).astype(np.int32)
    valid = (a1 >= 1) & (a1 <= 9) & (a2 >= 1) & (a2 <= 9)
    idx = (a1[valid] - 1) * 9 + (a2[valid] - 1)
    counts = np.bincount(idx, minlength=81).reshape(9, 9)
    return counts * pixel_area_km2
```

## 2. 城市不透水面与 InSAR 沉降漏斗风险叠加

将 CLCD Class `8`（Impervious 不透水面）重采样对齐至 MintPy `velocity.geo.tif` 网格后，统计 `|velocity| >= threshold_mm_yr` 区域的不透水面扩张区（新老城区）受灾比例，区分「新近填土固结沉降」与「深层地下水超采型区域沉降」。
