关键词：晴空地表温度合成、Landsat 8/9、LANDSAT/LC08/C02/T1_L2、LANDSAT/LC09/C02/T1_L2、QA_PIXEL、LST surface temperature

## 1. 适用场景与输入要求

选择 L2SP 场景。TIRS 原始热分辨率约 100 m，产品网格 30 m。日期窗口内需有可用热观测。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

QA_PIXEL 从低到高的位 0–5 表示填充、膨胀云、卷云、云、云影、雪，任一置位即剔除。QA_RADSAT 取零。ST_B10 × 0.00341802 + 149.0 得 K，再减 273.15 得 °C。 [LANDSAT/LC08/C02/T1_L2](https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC08_C02_T1_L2)、[LANDSAT/LC09/C02/T1_L2](https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC09_C02_T1_L2)

## 3. 完整可运行 Python 代码

```python
import ee


def build_lst(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("LANDSAT/LC08/C02/T1_L2")
              .merge(ee.ImageCollection("LANDSAT/LC09/C02/T1_L2"))
              .filterBounds(aoi).filterDate(start_date, end_date))
    scenes = scenes.filter(ee.Filter.eq("PROCESSING_LEVEL", "L2SP"))

    def calibrate(image):
        clear = image.select("QA_PIXEL").bitwiseAnd(63).eq(0)
        valid = clear.And(image.select("QA_RADSAT").eq(0))
        values = (image.select("ST_B10").multiply(0.00341802).add(149.0)
                  .subtract(273.15).rename("lst_c"))
        return values.updateMask(valid)

    values = scenes.map(calibrate)
    count = values.select(0).count().rename("n_obs")
    return values.median().addBands(count).clip(aoi)
```

## 4. 结果质检标准

检查云边冷斑和长期固定空洞。发射率辅助数据缺失可导致跨期缺测。抽查与同过境时刻地表测温的一致性。
