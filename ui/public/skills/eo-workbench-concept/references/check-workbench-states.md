关键词：检查预览与交付是否被混为同一成功状态、EO Workbench Concept Design、concept-design、workbench

## 1. 适用场景与输入要求

传入设计评审用的状态与结果映射。键仅用于本示例，可在评审层映射到产品状态。

## 2. 核心公式与参数避坑

验收样例区分可见预览、写出错误和经检查的交付物。一次预览渲染成功不能证明导出成功。

## 3. 完整可运行 Python 代码

```python
def check_states(states: dict[str, str]) -> list[str]:
    required = {"preview_ready": "preview", "export_failed": "error", "file_verified": "delivered"}
    return [key for key, outcome in required.items() if states.get(key) != outcome]


assert check_states({"preview_ready":"preview","export_failed":"delivered","file_verified":"delivered"}) == ["export_failed"]
```

## 4. 结果质检标准

将写出失败误标为 delivered 应定位到 export_failed；三项都满足时返回空列表。
