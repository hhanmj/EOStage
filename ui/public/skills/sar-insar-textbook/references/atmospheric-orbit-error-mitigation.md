# 轨道误差、对流层延迟与电离层改正规程 (Orbit & Atmospheric Mitigation)

## 1. 精密轨道与残余基线趋势面扣除

- Sentinel-1 SLC 必配 `AUX_POEORB`（径向精度 $< 5\,\text{cm}$）；实时应急若在采样后 20 天内尚未发布 POEORB，临时采用 `AUX_RESORB`，并在事后归档复算时升级替换。
- 对残留线性或二次轨道相位坡面（Long-wavelength Ramp），在排除区域构造构造形变与高程相关性后，利用远场非形变掩膜像元拟合双线性平面扣除（`MintPy deramp` 需显式说明是否会误削大尺度沉降漏斗）。

## 2. 对流层水汽延迟两路改正（GACOS ZTD + ERA5）

总对流层天顶延迟 $\text{ZTD} = \text{ZHD} + \text{ZWD}$ 投影到视线向 $\Delta \phi_{\text{tropo}} = \frac{4\pi}{\lambda \cos\theta_i} (\text{ZTD}_2 - \text{ZTD}_1)$：
1. **GACOS 迭代通用大气改正**：分辨率约 $90\,\text{m}$，结合 ECMWF 高分气压水汽场与 GNSS 实测 ZTD，对地形强相关的垂直分层延迟与局部湍流延迟均有良好抑制；在 EOStage 中下载 Sentinel-1 时支持按任务时相自动打包提交 GACOS 申请。
2. **ERA5 / PyAPS 模型改正 + 经验相位-高程线性回归 ($\Delta\phi = K \cdot h + c$)**：无外部 GACOS 时作为基准方案；对大尺度山地地震同震图尤其关键，需排除高程与同震断层共线性区域再回归。

## 3. L 波段 / S 波段电离层相位分离

L/S 波段波长长、法拉第旋转与电离层色散效应显著（与雷达载频成反比 $\Delta\phi_{\text{iono}} \propto 1/f_0$）：
- 对 Sentinel-1（C 波段）低纬地区强磁暴期间或 **ALOS-2 / NISAR** 干涉图，采用**距离向分谱法（Range Split-Spectrum）**配合 GIM/IONEX TEC 先验分离电离层非色散形变项，避免把数百公里电离层条纹误判为板块构造形变。
