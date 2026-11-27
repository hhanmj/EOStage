关键词：年际变异系数、Rasterio、GeoTIFF、CV、coefficient of variation

## 1. 适用场景与输入要求

同网格多年正值比率尺度变量，指定与测量精度相符的最小均值。 使用 NumPy 与 Rasterio；输入须显式提供 CRS 和有效掩膜或 nodata 元数据。

## 2. 核心公式与参数避坑

CV = 样本标准差 / 时间均值；此处输出比例。近零分母使 CV 不稳定，摄氏温度不适合直接计算 CV。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_coefficient_variation(paths, min_mean: float):
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
                if min_mean < 0:
                    raise ValueError("min_mean must be nonnegative")
                avg = data.mean(axis=0)
                cv = np.ma.divide(data.std(axis=0, ddof=1), avg)
                invalid = (data.count(axis=0) < 2) | (avg <= min_mean) | (data.min(axis=0) < 0)
                result = np.ma.masked_where(invalid, cv)
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

有效 CV 非负，正的常量序列为零。极高值应检查均值、观测期数和单位。
