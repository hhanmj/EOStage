关键词：历史同期均值填洞、Rasterio、GeoTIFF、climatological gap filling

## 1. 适用场景与输入要求

目标栅格与其他年份相同物候期、同单位的参考栅格。使用同网格单波段数据及显式有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

以历史同期有效均值填充目标缺测，至少三个基准年有观测。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_historical_fill(target, history):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in [target, *history]]
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
                reference = data[1:]
                mean = np.ma.masked_where(reference.count(axis=0) < 3, reference.mean(axis=0))
                result = np.ma.where(np.ma.getmaskarray(data[0]), mean, data[0])
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

核查未缺测位置完全一致。极端事件期间优先保留缺测标记，历史均值会压低异常幅度。
