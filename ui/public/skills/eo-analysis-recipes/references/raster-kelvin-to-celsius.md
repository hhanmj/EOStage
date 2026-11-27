关键词：温度单位换算、Rasterio、GeoTIFF、Kelvin to Celsius、LST

## 1. 适用场景与输入要求

以物理 K 为单位的温度栅格，带产品缩放的 DN 须先解码。所有输入须有 CRS、同网格，并在元数据中声明缺测或提供有效掩膜。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

T°C = TK − 273.15。零开尔文以下为无效物理输入。

## 3. 完整可运行 Python 代码

```python
from contextlib import ExitStack

import numpy as np
import rasterio
from rasterio.windows import Window


def iter_kelvin_to_celsius(temperature):
    """Yield (Window, float32 array) on the first input grid; nodata is NaN."""
    with ExitStack() as stack:
        sources = [stack.enter_context(rasterio.open(p)) for p in [temperature]]
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
                kelvin = data[0]
                result = np.ma.masked_less(kelvin, 0) - 273.15
                yield window, np.ma.asarray(result).astype("float32").filled(np.nan)
```

## 4. 结果质检标准

273.15 K 应得 0 °C。检查是否把已转换的摄氏度再次减去偏移。 输出按首幅栅格的 CRS 和仿射变换写入，声明 nodata=NaN。
