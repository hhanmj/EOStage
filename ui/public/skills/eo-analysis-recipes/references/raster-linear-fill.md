关键词：双侧时序插值填洞、Rasterio、GeoTIFF、linear gap filling、时间插值

## 1. 适用场景与输入要求

同一物理量按时间排序的前期、目标期和后期栅格，权重为目标距前期时间占比。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

只替换目标期缺测像元；两端必须都有观测。应先限制最大允许时间间隔。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_linear_fill(before, target, after, weight):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in [before, target, after]]
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
                if not 0 < weight < 1:
                    raise ValueError("weight must be between zero and one")
                left, mid, right = data
                estimate = left * (1 - weight) + right * weight
                result = np.ma.where(np.ma.getmaskarray(mid), estimate, mid)
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

原始有效值须逐像元不变；插补值应落在两端之间。积雪突变、洪峰期避免跨事件插值。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
