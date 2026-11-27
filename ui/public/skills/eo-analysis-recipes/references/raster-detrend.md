关键词：保留缺测的线性去趋势、Rasterio、GeoTIFF、linear detrending

## 1. 适用场景与输入要求

多期同网格栅格、日期数值和待输出残差的时间索引。使用同网格单波段数据及显式有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

每个像元在其有效时相拟合直线，再输出指定时相的观测减拟合。保留缺测掩膜可以避免虚假的长期变化。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_detrend(paths, years, index: int):
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
                t = np.asarray(years, dtype=float)
                if len(t) != len(data) or not np.isfinite(t).all() or np.any(np.diff(t) <= 0):
                    raise ValueError("invalid acquisition times")
                time = np.ma.array(np.broadcast_to(t[:, None, None], data.shape),
                                   mask=np.ma.getmaskarray(data))
                tc = time - time.mean(axis=0)
                yc = data - data.mean(axis=0)
                slope = np.ma.divide((tc * yc).sum(axis=0), (tc ** 2).sum(axis=0))
                result = np.ma.masked_where(data.count(axis=0) < 3, yc[index] - slope * tc[index])
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

完美线性序列的残差应接近零；原始缺测位置仍为 NaN。
