关键词：多年平均坡面侵蚀估算、USLE、universal soil loss equation、土壤流失

## 1. 适用场景与输入要求

输入同网格 R、K、LS、C、P 五个单波段因子。R 单位 MJ·mm/(ha·h·yr)，K 单位 t·ha·h/(ha·MJ·mm)。尺度由最粗因子限定。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

A = R×K×LS×C×P。LS 是包含坡长和坡度的无量纲因子；C、P 的参考情景须与标定一致。

## 3. 完整可运行 Python 代码

```python
import ee


def soil_loss(aoi: ee.Geometry, r: ee.Image, k: ee.Image, ls: ee.Image,
              c: ee.Image, p: ee.Image) -> ee.Image:
    valid = r.gte(0).And(k.gte(0)).And(ls.gte(0)).And(c.gte(0)).And(p.gte(0))
    loss = r.multiply(k).multiply(ls).multiply(c).multiply(p)
    return loss.updateMask(valid).rename("soil_loss_t_ha_yr").clip(aoi)
```

## 4. 结果质检标准

结果非负。与坡面小区侵蚀模数比对；流域出口输沙量还受泥沙输移和沟蚀影响。
