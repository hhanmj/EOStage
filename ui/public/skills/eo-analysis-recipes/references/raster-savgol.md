关键词：局部多项式平滑时序、Rasterio、GeoTIFF、Savitzky–Golay、SG、遥感时序

## 1. 适用场景与输入要求

输入同网格等间隔栅格，中心时相两侧均有观测。窗口半宽以时间步计。 使用 NumPy 与 Rasterio；输入须显式提供 CRS 和有效掩膜或 nodata 元数据。

## 2. 核心公式与参数避坑

在完整对称窗口上做二次最小二乘拟合并取中心值。样本缺失时保持掩膜，避免把不规则时间间隔当作等间隔。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_savgol(paths, index: int, radius: int):
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
                if radius < 2 or index < radius or index + radius >= len(data):
                    raise ValueError("need a complete centered window of at least five dates")
                t = np.arange(-radius, radius + 1, dtype=float)
                design = np.column_stack((np.ones_like(t), t, t*t))
                weights = np.linalg.pinv(design)[0]
                part = data[index-radius:index+radius+1]
                smoothed = np.tensordot(weights, part.filled(0), axes=(0, 0))
                result = np.ma.array(smoothed, mask=np.ma.getmaskarray(part).any(axis=0))
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

常值、线性和二次测试序列的中心值应保持。检查峰值附近过冲及窗口导致的快速事件衰减。
