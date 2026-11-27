# SAR 成像几何、地形畸变与辐射地形校正 (SAR Geometry & Radiometry)

## 1. 斜距成像几何与地形畸变判别准则

设雷达侧视局部入射角为 $\theta_i$，沿距离向（Range Look Vector 方向）地形坡角为 $\alpha_r$（朝向传感器为正，背向传感器为负）：
- **透视收缩（Foreshortening）**：$0 < \alpha_r < \theta_i$，迎坡面积在斜距像面被压缩，后向散射异常偏亮。
- **叠掩（Layover）**：$\alpha_r \ge \theta_i$，山顶比山脚更靠近传感器，多处地物散射叠加在同一分辨单元，干涉相位彻底混淆，**应通过 `shadow_layover_mask` 掩膜剔除**。
- **雷达阴影（Shadow）**：$-\alpha_r \ge 90^\circ - \theta_i$，背坡无法被微波照射，回波仅为接收机热噪声（$\gamma \approx 0$）。

## 2. 轨道方向（Ascending vs Descending）与形变观测灵敏度

星载 SAR 单位视线向量（卫星指向地面目标）：
$$\hat{u}_{\text{LOS}} = [u_E,\, u_N,\, u_U]^T = [\mp \sin\theta_i \cos\alpha_h,\; -\sin\theta_i \sin\alpha_h,\; -\cos\theta_i]^T$$
其中近极轨卫星升轨（Ascending）航向角约 $\alpha_h \approx -10^\circ$（传感器从南向北飞行、右侧东视），降轨（Descending）约 $\alpha_h \approx -170^\circ$（从北向南、右侧西视）：
- 东西向（E-W）形变主导或陡坡滑坡监测：**务必同时检索升轨与降轨数据**（2D 二维形变分解：$d_U, d_E$）。
- 南北向（N-S）高梯度滑坡：$u_N$ 分量较小（约 $0.1\sim 0.15$），应辅以像素偏移量追踪（Pixel Offset Tracking，NISAR `GOFF` / `ROFF` 产品）或光学亚像元相关性配准。

## 3. 辐射定标与相干斑滤波顺序

正确顺序（切勿颠倒）：
`精密轨道定轨 (POEORB) -> 热噪声去除 -> Radiometric Calibration (sigma0) -> Terrain Flattening (gamma0 RTC) -> Multi-look / Refined Lee Filter -> 地形校正 Geocoding (Range-Doppler) -> 对数变换 dB`

注意：**时间中值/均值统计与多视抑制应在物理线性功率单位（Linear power）完成，可视化或阈值统计再转为 $10 \log_{10}(\gamma^0)$**。
