关键词：有效观测时段均值、Rasterio、GeoTIFF、temporal mean、日均、八日均值

## 1. 适用场景与输入要求

同一变量多期等间隔观测，每个路径对应一个时间点。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

按像元有效观测等权求均值。缺测值填零会系统性拉低结果。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_mean(paths):
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
                result = data.mean(axis=0)
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

均值应位于有效输入的最小值与最大值之间。全缺测像元返回 NaN。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
