关键词：逐步核对工具能力和上游产物、EO Agent Capability Routing、agent-routing、tool-selection

## 1. 适用场景与输入要求

输入按依赖排序的候选步骤，每步给出名称、能力集合、所需产物集合和产物名。

## 2. 核心公式与参数避坑

仅在该步骤的能力与上游输入都可满足时，才将其产物加入规划集合。结果只检查计划可行性。

## 3. 完整可运行 Python 代码

```python
def assess_steps(steps: list[tuple[str, set, set, str]], capabilities: set, inputs: set) -> list[tuple]:
    available = set(inputs)
    result = []
    for name, needs, consumes, produces in steps:
        missing = sorted(needs - capabilities)
        absent = sorted(consumes - available)
        result.append((name, missing, absent))
        if not missing and not absent:
            available.add(produces)
    return result


assert assess_steps([("index", {"compute"}, {"scene"}, "ndvi"), ("write", {"write"}, {"ndvi"}, "file")], {"write"}, {"scene"}) == [("index", ["compute"], []), ("write", [], ["ndvi"])]
```

## 4. 结果质检标准

删除计算能力后，指数步骤应报告缺能力，下游写出步骤应报告缺指数产物。
