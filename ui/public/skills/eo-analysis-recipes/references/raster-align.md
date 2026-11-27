关键词：对齐到参考栅格、Rasterio、WarpedVRT、grid alignment、配准重采样

## 1. 适用场景与输入要求

输入已正确定位的源单波段和参考栅格，均需有效 CRS；源缺测须在元数据或掩膜中声明。 运行环境为 NumPy 和 Rasterio。

## 2. 核心公式与参数避坑

以参考影像的 CRS、仿射变换、尺寸执行真实重投影。只改地理变换会移动像元标签而不校正采样位置。类别用最近邻，连续场用双线性。

## 3. 完整可运行 Python 代码

```python
import numpy as np
import rasterio
from rasterio.enums import Resampling
from rasterio.vrt import WarpedVRT
from rasterio.windows import Window


def iter_aligned(source, reference, categorical: bool):
    with rasterio.open(source) as src, rasterio.open(reference) as ref:
        if not src.crs or not ref.crs or src.count != 1:
            raise ValueError("need georeferenced inputs and a single source band")
        method = Resampling.nearest if categorical else Resampling.bilinear
        with WarpedVRT(src, crs=ref.crs, transform=ref.transform, width=ref.width,
                       height=ref.height, resampling=method, dtype="float64",
                       nodata=np.nan) as aligned:
            for row in range(0, ref.height, 256):
                for col in range(0, ref.width, 256):
                    win = Window(col, row, min(256, ref.width-col), min(256, ref.height-row))
                    yield win, aligned.read(1, window=win, masked=True).filled(np.nan)
```

## 4. 结果质检标准

按参考网格写出窗口，缺测为 NaN。输出边界应与参考栅格吻合，离散类别集合不能增加。
