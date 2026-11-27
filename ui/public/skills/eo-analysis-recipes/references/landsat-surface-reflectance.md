关键词：Landsat 地表反射率合成、Landsat 8/9、LANDSAT/LC08/C02/T1_L2、LANDSAT/LC09/C02/T1_L2、QA_PIXEL、SR surface reflectance

## 1. 适用场景与输入要求

OLI 多光谱 30 m。处理 Landsat 8/9 同季多景，输入时间须覆盖相应卫星运行期。 运行环境为已初始化的 earthengine-api。

## 2. 核心公式与参数避坑

QA_PIXEL 从低到高的位 0–5 表示填充、膨胀云、卷云、云、云影、雪，任一置位即剔除。QA_RADSAT 取零。SR_B* × 0.0000275 − 0.2。偏移项在归一化差值中也不能省略。 [LANDSAT/LC08/C02/T1_L2](https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC08_C02_T1_L2)、[LANDSAT/LC09/C02/T1_L2](https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC09_C02_T1_L2)

## 3. 完整可运行 Python 代码

```python
import ee


def build_reflectance(aoi: ee.Geometry, start_date: str, end_date: str) -> ee.Image:
    scenes = (ee.ImageCollection("LANDSAT/LC08/C02/T1_L2")
              .merge(ee.ImageCollection("LANDSAT/LC09/C02/T1_L2"))
              .filterBounds(aoi).filterDate(start_date, end_date))

    def calibrate(image):
        clear = image.select("QA_PIXEL").bitwiseAnd(63).eq(0)
        valid = clear.And(image.select("QA_RADSAT").eq(0))
        values = image.select("SR_B[1-7]").multiply(0.0000275).subtract(0.2)
        return values.updateMask(valid)

    values = scenes.map(calibrate)
    count = values.select(0).count().rename("n_obs")
    return values.median().addBands(count).clip(aoi)
```

## 4. 结果质检标准

多数陆面反射率在 0–1，暗目标可为负。逐波段检查分位数和云边缘，勿把小负值直接解释为无效填充。
