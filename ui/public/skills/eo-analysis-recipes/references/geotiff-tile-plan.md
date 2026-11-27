关键词：桌面分块拉取网格、coveringGrid、GeoTIFF、getDownloadURL、GEE

## 1. 适用场景与输入要求

输入 AOI、米制投影和像元大小。宿主取得每块区域后拉取影像，表格按区块或 ID 分页。

## 2. 核心公式与参数避坑

在同一投影上按整数个像元切块并返回完整矩形；影像先掩膜至 AOI，再按矩形读取可拼接栅格。 [下载接口](https://developers.google.com/earth-engine/apidocs/ee-image-getdownloadurl)

## 3. 完整可运行 Python 代码

```python
import ee


def tile_plan(aoi: ee.Geometry, crs: str, pixel_m: float, side_pixels: int = 512) -> ee.FeatureCollection:
    if pixel_m <= 0 or not 1 <= side_pixels <= 10000:
        raise ValueError("invalid tile dimensions")
    cells = aoi.coveringGrid(ee.Projection(crs), pixel_m * side_pixels)
    return cells.map(lambda cell: cell.set({"tile_id": cell.id(), "pixel_m": pixel_m,
                                            "crs": crs, "side_pixels": side_pixels}))
```

## 4. 结果质检标准

各块固定同一目标变换、数据类型、nodata 和波段顺序。按波段和字节数控制单次请求 32 MB 及单边 10000 像元限制。
