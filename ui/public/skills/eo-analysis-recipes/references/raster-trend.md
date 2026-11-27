关键词：逐像元线性趋势、Rasterio、GeoTIFF、OLS slope、长期趋势

## 1. 适用场景与输入要求

多期连续变量和对应数值年份，至少三个不同时间点。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

在每个像元自己的有效年份上拟合最小二乘斜率。切勿将 NoData 填零，否则易制造虚假下降趋势。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_trend(paths, years):
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
                    raise ValueError("years must increase and match the rasters")
                time = np.ma.array(np.broadcast_to(t[:, None, None], data.shape),
                                   mask=np.ma.getmaskarray(data))
                centered = time - time.mean(axis=0)
                slope = np.ma.divide((centered * (data - data.mean(axis=0))).sum(axis=0),
                                     (centered ** 2).sum(axis=0))
                result = np.ma.masked_where(data.count(axis=0) < 3, slope)
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

斜率单位为变量单位/年。对常值区应得零，抽查有缺年的像元与一维回归是否一致。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
