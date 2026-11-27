关键词：Dynamic World、近实时 10 m 土地覆盖、类别概率合成、概率变化监测、GOOGLE/DYNAMICWORLD/V1

## 1. 适用场景与输入要求

需要近实时（随 Sentinel-2 过境更新）10 m 九类土地覆盖制图、地表扩张或毁林动态筛查时使用。输入研究区边界 `aoi`、基线期与对比期起止日期 `[t0, t1)` 和 `[t2, t3)`，以及重点变化目标类别名（9 类之一：`water`, `trees`, `grass`, `flooded_vegetation`, `crops`, `shrub_and_scrub`, `built`, `bare`, `snow_and_ice`）。运行环境为已初始化的 `earthengine-api`。

## 2. 核心公式与参数避坑

- **概率均值优于直接对逐景硬分类作差**：逐景 `label` 受云残余和物候起伏影响大；推荐对时段内 9 个连续概率波段求均值 `.mean()`（或中位数），再用各像元最大概率或 `label.mode()` 确定主地类，跨期变化优先比较 `prob_after - prob_before` 连续概率增益并叠加阈值筛选，能显著压低伪变化噪声。
- **概率总和与空像元防护**：单景 9 个类别概率波段经模型 softmax 输出、理论上和为 1；当某时段极度多云无有效覆盖时 `mean()` 会被掩膜，必须同时输出最小观测次数 `obs_count` 供下游剔除缺测区。官方目录：[GOOGLE/DYNAMICWORLD/V1](https://developers.google.com/earth-engine/datasets/catalog/GOOGLE_DYNAMICWORLD_V1)。

## 3. 完整可运行 Python 代码

```python
import ee

DW_CLASSES = (
    "water",
    "trees",
    "grass",
    "flooded_vegetation",
    "crops",
    "shrub_and_scrub",
    "built",
    "bare",
    "snow_and_ice",
)


def dynamic_world_change(
    aoi: ee.Geometry,
    pre_start: str,
    pre_end: str,
    post_start: str,
    post_end: str,
    target_class: str = "built",
    prob_delta_thresh: float = 0.25,
) -> ee.Image:
    if target_class not in DW_CLASSES:
        raise ValueError(f"target_class must be one of {DW_CLASSES}")

    base = ee.ImageCollection("GOOGLE/DYNAMICWORLD/V1").filterBounds(aoi)
    pre_col = base.filterDate(pre_start, pre_end)
    post_col = base.filterDate(post_start, post_end)

    pre_probs = pre_col.select(list(DW_CLASSES)).mean().clip(aoi)
    post_probs = post_col.select(list(DW_CLASSES)).mean().clip(aoi)
    post_label = post_col.select("label").mode().rename("post_label_mode").clip(aoi)

    delta = (
        post_probs.select(target_class)
        .subtract(pre_probs.select(target_class))
        .rename(f"{target_class}_prob_delta")
    )
    gain_mask = delta.gte(prob_delta_thresh).rename(f"{target_class}_gain_mask")
    min_obs = (
        pre_col.select("label")
        .count()
        .min(post_col.select("label").count())
        .toInt16()
        .rename("min_obs_count")
        .clip(aoi)
    )
    return post_label.addBands([delta, gain_mask, min_obs])
```

## 4. 结果质检标准

核查 `min_obs_count` 确保两期均存在足够晴空观测；季节性作物轮作/物候落叶在 `trees`/`crops`/`grass` 之间易产生季节概率波动，评估长期用地变化时前后两期须尽量对齐相同物候窗口。
