关键词：土壤调节植被指数、Rasterio、GeoTIFF、SAVI、soil adjusted vegetation index

## 1. 适用场景与输入要求

同一天的红光与近红外地表反射率，须先在物理单位下解码。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

SAVI = 1.5(NIR−Red)/(NIR+Red+0.5)，这里固定 L=0.5，适于中等覆盖度。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_savi(red, nir):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in [red, nir]]
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
                r, n = data
                denom = n + r + 0.5
                result = np.ma.divide(1.5 * (n - r), denom)
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

红光与近红外相等时 SAVI 为零；检查 DN 未缩放导致的土壤项失效。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
