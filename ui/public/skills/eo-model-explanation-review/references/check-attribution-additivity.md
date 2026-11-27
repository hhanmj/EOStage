关键词：在同一输出尺度复核单样本归因、EO Model Explanation Review、model-explanation、error-analysis

## 1. 适用场景与输入要求

同一模型、同一类别、同一样本的基线、各特征贡献与模型输出；调用方指定该输出尺度上的数值容差。

## 2. 核心公式与参数避坑

计算加和重构值与输出的残差。近似解释器需结合算法误差设定容差。

## 3. 完整可运行 Python 代码

```python
def check_attribution(base: float, contributions: list[float], output: float, tolerance: float) -> tuple[float, bool]:
    from math import fsum, isfinite
    if not contributions or not all(isfinite(x) for x in [base, output, tolerance, *contributions]) or tolerance < 0:
        raise ValueError("expected finite values, contributions and a nonnegative tolerance")
    residual = fsum([base, *contributions, -output])
    return residual, abs(residual) <= tolerance


assert check_attribution(0.2, [0.3, -0.1], 0.4, 1e-8)[1]
assert not check_attribution(0.2, [0.3, -0.1], 0.8, 1e-8)[1]
```

## 4. 结果质检标准

同尺度且贡献齐全的精确解释应通过；改变输出而保留相同贡献时应检出不一致。
