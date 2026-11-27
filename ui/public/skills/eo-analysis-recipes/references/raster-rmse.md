关键词：配对时序误差、Rasterio、GeoTIFF、RMSE、root mean square error

## 1. 适用场景与输入要求

相同日期顺序的实测与预测栅格列表，变量和单位一致。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

仅有效配对进入均方根误差。该指标对大残差敏感。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_rmse(observed, predicted):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in list(observed) + list(predicted)]
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
                n = len(observed)
                if n != len(predicted) or n == 0:
                    raise ValueError("need paired observations")
                error = data[n:] - data[:n]
                result = np.ma.sqrt((error ** 2).mean(axis=0))
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

RMSE 非负，完全一致的序列误差为零。高值区核对时相匹配和网格。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
