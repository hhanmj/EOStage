关键词：逐像元配对相关、Rasterio、GeoTIFF、Pearson r、时序相关

## 1. 适用场景与输入要求

两组同日期的遥感变量，至少三对有效样本。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

先取共同观测，再分别中心化。任一序列方差为零时相关无定义。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_pearson(first, second):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in list(first) + list(second)]
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
                n = len(first)
                if n != len(second):
                    raise ValueError("series lengths differ")
                x, y = data[:n], data[n:]
                mask = np.ma.getmaskarray(x) | np.ma.getmaskarray(y)
                x, y = np.ma.array(x, mask=mask), np.ma.array(y, mask=mask)
                dx, dy = x - x.mean(axis=0), y - y.mean(axis=0)
                denom = np.ma.sqrt((dx ** 2).sum(axis=0) * (dy ** 2).sum(axis=0))
                result = np.ma.masked_where(x.count(axis=0) < 3, np.ma.divide((dx * dy).sum(axis=0), denom))
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

r 在 [-1,1]。季节共变可造成高相关，推断显著性前处理自相关。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
