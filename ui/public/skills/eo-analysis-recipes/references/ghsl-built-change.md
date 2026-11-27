关键词：建成表面面积变化、GHSL、JRC/GHSL/P2023A/GHS_BUILT_S、built surface change

## 1. 适用场景与输入要求

100 m 网格，输入 1975–2030 的五年间隔产品年份，后期晚于前期。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

built_surface 已是每像元建成表面积 m²。后期减前期，再按区求和时无需乘像元面积。注意模型插值和外推年份。 [JRC/GHSL/P2023A/GHS_BUILT_S](https://developers.google.com/earth-engine/datasets/catalog/JRC_GHSL_P2023A_GHS_BUILT_S)

## 3. 完整可运行 Python 代码

```python
import ee


def built_change(aoi: ee.Geometry, first_year: int, last_year: int) -> ee.Image:
    years = range(1975, 2031, 5)
    if first_year not in years or last_year not in years or first_year >= last_year:
        raise ValueError("expected two ordered GHSL epochs")

    def surface(year):
        return ee.Image(f"JRC/GHSL/P2023A/GHS_BUILT_S/{year}").select("built_surface")

    return (surface(last_year).toFloat().subtract(surface(first_year))
            .rename("built_change_m2").clip(aoi))
```

## 4. 结果质检标准

100 m 原生格网单元的差值绝对值通常不超过 10000 m²；负值需结合拆除、建模误差或网格错位判断。
