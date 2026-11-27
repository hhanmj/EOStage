关键词：参考范围归一化、Rasterio、GeoTIFF、min–max normalization

## 1. 适用场景与输入要求

同变量栅格及由训练或基准期有效像元估计的全域上下界。使用同网格单波段数据及显式有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

(x − low)/(high − low)。窗口共用同一对参数，保留基准期外越界值。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_minmax(source, low: float, high: float):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in [source]]
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
                if not np.isfinite([low, high]).all() or high <= low:
                    raise ValueError("invalid reference range")
                result = (data[0] - low) / (high - low)
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

参考区间的端点映射到 0 和 1。空间块边缘不应有由分块造成的数值跳跃。
