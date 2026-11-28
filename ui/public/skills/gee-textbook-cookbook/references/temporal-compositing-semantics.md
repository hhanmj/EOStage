# 时间合成与时间序列语义规范

GEE 的集合归约（Collection Reducer）是**逐像元（pixel-wise）**独立运算的。一幅看似完整干净的合成影像，其相邻像元甚至同一像元的不同波段，可能分别来自完全不同的观测日期。

## 1. 合成算子语义对照表

| 操作算子 | 像元值物理来源 | 适用分析场景 | 核心科学风险 |
| --- | --- | --- | --- |
| `median()` / `mean()` | 每个波段独立取所有有效观测的中值或均值 | 稳健季节背景、减少单景残云/奇异值影响 | 生成的是**合成统计量**，同一像元不同波段可能拼自不同日期，**禁止表述为单日真实过境影像** |
| `percentile()` / `min()` / `max()` | 逐像元分布分位数或极值 | 物候极值（如年内最高植被）、干旱/热浪包络线 | 极值对漏检云影、高光饱和或未掩膜异常噪声极其敏感 |
| `mosaic()` | 按集合内**先后顺序取最后一个非掩膜像元** | 按优先级/最新日期空间拼接 | 集合排序即是选择规则，**不代表成像质量更好**；未排序则结果随入库序变化 |
| `qualityMosaic(q)` | 对每个像元选出品质波段 `q` 最高的那**整景所有波段** | 最绿像元合成、最短时滞晴空最优像元、保持同景跨波段光谱一致 | 不同像元来自不同日期；若 `q`（如 NDVI）未配合云掩膜，可能挑中云边缘伪高值 |
| 逐景制表序列 (`map` -> `FeatureCollection`) | 每个日期窗口独立统计区域均值/分位 | 趋势回归、物候曲线、突发事件精准定日 | 必须显式保留云遮挡导致的缺测时段，不可静默跳过伪装等间隔采样 |

## 2. 强制附加的两类质量诊断波段

### 任何中值/均值合成必带有效观测数 `valid_obs`
仅靠外观“看起来没云”不能证明合成质量；仅 1 次有效观测的像元与 20 次观测的像元统计可信度完全不同：

```python
masked = collection.map(mask_and_scale)
median_img = masked.select(target_bands).median()
valid_obs = masked.select(target_bands[0]).count().rename("valid_obs")
composite_with_qa = median_img.addBands(valid_obs)
```

### 使用 `qualityMosaic()` 时必保留选中日期 `source_time`
```python
def attach_source_time(img: ee.Image) -> ee.Image:
    ts = ee.Image.constant(img.date().millis()).toDouble().rename("source_time")
    return img.addBands(ts)

best_img = collection.map(attach_source_time).qualityMosaic("quality_score")
```

## 3. 周期序列（月/季/年）编制 5 条纪律

1. **左闭右开时间窗 `[start, end)`**：避免跨期边界重复统计同一景（`ee.Filter.date(start, end)` 默认左闭右开）。
2. **先掩膜定标、后降维聚合**：在任何 `median/mean/max` 之前完成 QA 位掩膜和缩放偏移。
3. **空窗口显式留空**：月度全云无有效观测时保留空值（Masked / NaN），**先显式定义缺失填充规则再做 Savitzky-Golay 或谐波平滑**，平滑曲线属于观测模型而非实测无云图像。
4. **前后期一致化对比**：变化监测的前后期须维持一致的季节窗与接近的有效观测次数。
5. **分类产物永不平均**：土地利用/覆盖编码只允许 `mode()`、类别计数或乘以 `pixelArea()` 统计面积，严禁 `mean()` / `median()`。

## 4. 最小披露清单

向用户汇报合成或时序结果时，至少写清：**时间窗口区间、采用算子或拼接排序规则、QA 掩膜方式、输出尺度与投影、`valid_obs` 有效观测分布（或缺失间隔数量）**，并准确称之为“周期合成产品”或“统计摘要”。
