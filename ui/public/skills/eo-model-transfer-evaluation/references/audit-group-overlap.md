关键词：检查训练与测试是否共享独立采样单元、EO Model Transfer Evaluation、model-selection、spatial-validation

## 1. 适用场景与输入要求

传入两侧每条样本的稳定地块或空间块 ID。重叠影像块应在上游使用同一分组策略。

## 2. 核心公式与参数避坑

训练组集合与测试组集合取交集。无交集仅证明 ID 分离，邻域重叠、时间穿越和预处理泄漏仍要单独核查。

## 3. 完整可运行 Python 代码

```python
def overlap(train: list[str], test: list[str]) -> list[str]:
    if not train or not test or any(not x.strip() for x in train + test):
        raise ValueError("both splits need nonempty group identifiers")
    return sorted(set(train) & set(test))


assert overlap(["field-a", "field-b"], ["field-b", "field-c"]) == ["field-b"]
```

## 4. 结果质检标准

重复地块须被检出。缺失的组 ID 或空划分应报错。
