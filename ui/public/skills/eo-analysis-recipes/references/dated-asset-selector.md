关键词：按期读取专题资产、canopy height、rice extent、AGB、annual composites、GEE Assets

## 1. 适用场景与输入要求

适用外部水稻、冠层高度、AGB 或年合成产品。提供经核实授权的 ee.Image、波段、比例/偏移、有效编码范围和物理单位。静态图片按其实际观测年份解释。

## 2. 核心公式与参数避坑

保留源栅格的原始掩膜；先在 DN 域去填充值，再乘比例并加偏移。分类码用比例 1 和偏移 0；类别间禁用双线性插值。

## 3. 完整可运行 Python 代码

```python
import ee


def asset_band(aoi: ee.Geometry, image: ee.Image, band: str, output: str, unit: str,
               valid_min: float, valid_max: float, factor: float = 1, offset: float = 0) -> ee.Image:
    if valid_max < valid_min or factor == 0:
        raise ValueError("invalid encoding")
    raw = image.select(band)
    values = raw.updateMask(raw.gte(valid_min).And(raw.lte(valid_max)))
    return values.multiply(factor).add(offset).rename(output).clip(aoi).set(
        "units",unit,"source_band",band,"source_time",image.get("system:time_start"))
```

## 4. 结果质检标准

核对类别码本、有效范围、时间与空间范围。不同年份需显式选择对应波段或影像；专题冠层模型与 GEDI 足迹高度不可互换。
