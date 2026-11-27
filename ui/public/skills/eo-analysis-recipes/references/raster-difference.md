关键词：栅格配对差值、Rasterio、GeoTIFF、paired difference、变化量

## 1. 适用场景与输入要求

先期、后期同变量且同单位的两幅单波段栅格。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

差值 = 后期 − 先期。缺测取两期掩膜的并集。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_difference(before, after):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in [before, after]]
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
                a, b = data
                result = b - a
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

顺序反转时差值符号应翻转；相同影像有效区为零。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
