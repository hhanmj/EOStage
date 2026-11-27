# GEE 服务端性能优化、集合 Join 与排错清单 (Performance, Joins & Debugging)

## 1. 客户端 / 服务端严格隔离与初始化规范

- **显式绑定 Cloud Project**：现代 Earth Engine Python API 初始化推荐传入 GCP 项目标识 `ee.Initialize(project="...")`，避免无项目归属导致配额与计费路由失败。
- **严禁客户端循环调用服务端求值**：禁止写 Python `for i in range(col.size().getInfo())` 或在 `.map()` 内部调用 `.getInfo()` / `print()`；全部批量操作必须以 `ee.ImageCollection.map()`、`ee.List.map()`、`ee.Reducer` 表达为服务端计算图（DAG）。

## 2. 四条最立竿见影的性能避坑红线

| 高频反模式 | 为什么会导致崩溃或超时 | 正确做法 |
| --- | --- | --- |
| 在集合每个元素上早早 `.clip(复杂边界)` | 大尺度矢量裁剪会在早期节点把计算图切碎且迫使服务端过早评估复杂几何边界 | 仅用 `.filterBounds(aoi)` 做空间初筛，仅在最终导出、显示或缩减时统一在末端裁剪一次 |
| 中途随意调用 `.reproject(crs, scale)` | 打破 GEE 金字塔懒计算多尺度机制，迫使服务端按指定细分辨率强制物化超大范围网格，触发内存爆炸 | 让算子按导出 `scale` 或视图级别自动按需重投影；仅在物理要求明确固定网格的算子末端或导出前显式设置 |
| 先 `.map()` 重算全波段再筛选时间/云量 | 对最终根本用不到的超多云景或范围外瓦片白白构建了重型计算图 | **Filter Early**：先按时间 `filterDate`、范围 `filterBounds`、云量元数据滤除无用景，再 `.select(needed_bands)`，最后再 `.map()` |
| 大区域 `reduceRegion` / `sampleRegions` 爆内存 | 默认单计算瓦片承载过重多波段高分样本（报 `User memory limit exceeded`） | 增大 `tileScale=4..16` 将每个计算瓦片拆细；仅做估算可视化统计时设 `bestEffort=True` 或增大 `maxPixels=1e10` |

## 3. 集合配对与属性关联选型（`ee.Join` 家族）

详见可执行示例 `eo-analysis-recipes/references/ee-temporal-collection-join.md`：

- **`primary.linkCollection(secondary, bands)`**：两个集合具有一一对应的系统 ID（如 `COPERNICUS/S2_SR_HARMONIZED` 配对 `GOOGLE/CLOUD_SCORE_PLUS/V1/S2_HARMONIZED`）时的最轻量直连方式。
- **`ee.Join.saveBest(matchKey, measureKey)`**：为每个主影像查找时间最接近且空间相交的单个最佳辅影像（如 S2 配对最邻近过境 S1）。
- **`ee.Join.saveAll(...)`**：保留时间窗口内的全部合规辅景列表（例如构建前后平滑窗或相邻月参考堆叠）。
- **`ee.Join.inner(...)`**：只保留主辅双集合同时命中的配对要素集合（直接过滤掉空匹配，再还原为 `ImageCollection` 或特性表）。

## 4. 高频报错速查与降载决策表

```python
import ee

def safe_region_mean(img: ee.Image, aoi: ee.Geometry, scale: float = 30.0) -> ee.Dictionary:
    # 规范示范：先选所需波段，调大 tileScale 与 maxPixels 防止内存溢出
    return img.select(["NDVI"]).reduceRegion(
        reducer=ee.Reducer.mean(),
        geometry=aoi,
        scale=scale,
        maxPixels=1e10,
        tileScale=8,
        bestEffort=False,
    )
```

- **`User memory limit exceeded`**：依次检查 ① 去掉中间环节的 `.reproject()` 与 `.clip()`；② 提前 `.select()` 剔除无关波段；③ 在 `reduceRegion`/`sampleRegions` 中把 `tileScale` 提高到 `4`、`8` 或 `16`；④ 超大省域导出改按 `coveringGrid` 分块计算或调用 EOStage `gee_chunked_download`。
- **`Computation timed out`**：检查是否混用了 `toList(size)` 客户端遍历或未提前 `filterBounds`/`filterDate`；复杂多年度回归或 CCDC/LandTrendr 请缩小每次导出 AOI 分片范围并改用批处理分块导出。
- **`Array: Slice bounds out of bounds`**：对 `LandTrendr` / `CCDC` 等变长数组像元输出执行 `arraySlice` 前，必须先检查 `.arrayLength(axis).gt(...)` 并用 `.updateMask(...)` 屏蔽空断点或单顶点缺测像元。
