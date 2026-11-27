关键词：谐波拟合填补季节序列、Rasterio、GeoTIFF、HANTS、harmonic analysis of time series

## 1. 适用场景与输入要求

输入同网格单波段植被序列、相对观测日数组、目标序号以及以天为单位的周期。用于有周期性且 QA 后仍有低值离群点的连续指标。 使用 NumPy 与 Rasterio；输入须显式提供 CRS 和有效掩膜或 nodata 元数据。

## 2. 核心公式与参数避坑

用常数与一阶年谐波拟合季节变化，迭代剔除超过容差的最大负残差。仅重建目标缺测值。容差与输入物理单位一致。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_harmonic_gap_fill(paths, days, index: int, tolerance: float, period: float = 365.2425):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in paths]
        if not sources:
            raise ValueError("empty raster sequence")
        grid = sources[0]
        for src in sources:
            if (not src.crs or src.crs != grid.crs or src.transform != grid.transform
                    or src.shape != grid.shape or src.count != 1):
                raise ValueError("expected aligned, single-band georeferenced inputs")
        for row in range(0, grid.height, 256):
            for col in range(0, grid.width, 256):
                window = Window(col, row, min(256, grid.width - col),
                                min(256, grid.height - row))
                data = np.ma.stack([
                    np.ma.masked_invalid(src.read(1, window=window, masked=True)
                                         .astype("float64")) for src in sources
                ])
                t = np.asarray(days, dtype=float)
                if (t.size != len(data) or not np.isfinite(t).all() or np.any(np.diff(t) <= 0)
                        or period <= 0 or tolerance <= 0):
                    raise ValueError("invalid times, period or rejection threshold")
                phase = 2 * np.pi * t / period
                design = np.column_stack((np.ones_like(t), np.cos(phase), np.sin(phase)))
                result = data[index].copy()
                for y, x in zip(*np.where(np.ma.getmaskarray(result))):
                    series = data[:, y, x]
                    keep = ~np.ma.getmaskarray(series)
                    while keep.sum() >= 5:
                        samples = np.flatnonzero(keep)
                        coef, _, rank, _ = np.linalg.lstsq(design[keep], series.data[keep], rcond=None)
                        if rank < 3:
                            break
                        residual = series.data[keep] - design[keep] @ coef
                        worst = residual.argmin()
                        if residual[worst] >= -tolerance:
                            result[y, x] = design[index] @ coef
                            break
                        keep[samples[worst]] = False
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

对留出的真实观测做重建误差检验，分物候阶段报告 RMSE。常绿地、双季作物或灾变后的波形未必满足单谐波假设。
