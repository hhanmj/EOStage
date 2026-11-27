关键词：分层地理探测器 q 统计量、GeoDetector、factor detector、地理探测器、Rasterio

## 1. 适用场景与输入要求

单波段连续响应和同网格整数分层。每像元等权；用于面积代表性的推断时采用等面积网格。 使用 NumPy 与 Rasterio；输入须显式提供 CRS 和有效掩膜或 nodata 元数据。

## 2. 核心公式与参数避坑

q = 1 − Σ组内离差平方和 / 总离差平方和。仅共同有效像元参与。跨窗口合并计数、均值和离差平方和，保持全域统计口径。

## 3. 完整可运行 Python 代码

```python
import numpy as np
import rasterio
from rasterio.windows import Window


def factor_q(response, strata) -> dict:
    def merge(a, b):
        n, mean, ss = a
        m, other, ss2 = b
        delta = other - mean
        return n+m, mean + delta*m/(n+m), ss + ss2 + delta**2*n*m/(n+m)

    groups = {}
    with rasterio.open(response) as ysrc, rasterio.open(strata) as hsrc:
        if (not ysrc.crs or ysrc.crs != hsrc.crs or ysrc.transform != hsrc.transform
                or ysrc.shape != hsrc.shape or ysrc.count != 1 or hsrc.count != 1):
            raise ValueError("expected two aligned single-band rasters")
        for row in range(0, ysrc.height, 256):
            for col in range(0, ysrc.width, 256):
                win = Window(col, row, min(256, ysrc.width-col), min(256, ysrc.height-row))
                y = np.ma.masked_invalid(ysrc.read(1, window=win, masked=True))
                h = np.ma.masked_invalid(hsrc.read(1, window=win, masked=True))
                valid = ~(np.ma.getmaskarray(y) | np.ma.getmaskarray(h))
                y, h = y.data[valid].astype(float), h.data[valid]
                if not np.all(h == np.floor(h)):
                    raise ValueError("strata must contain integer codes")
                for label in np.unique(h):
                    values = y[h == label]
                    mean = values.mean()
                    part = len(values), mean, ((values-mean)**2).sum()
                    groups[label] = merge(groups.get(label, (0, 0., 0.)), part)
    total = (0, 0., 0.)
    for summary in groups.values():
        total = merge(total, summary)
    within = sum(group[2] for group in groups.values())
    return {"q": 1-within/total[2] if total[2] > 0 else None,
            "n": total[0], "strata": len(groups)}
```

## 4. 结果质检标准

q 通常在 [0,1]，常量响应返回 None。每像元一个分层会得到无意义的完美拟合，报告各层样本量并检查离散化敏感性。
