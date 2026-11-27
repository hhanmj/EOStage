# 双通 D-InSAR 干涉相位分解与二维解缠规程 (D-InSAR & Phase Unwrapping)

## 1. 干涉相位五分量物理模型

两景共配准 SLC 复共轭相乘后的缠绕相位 $\phi \in (-\pi, \pi]$ 满足：
$$\phi = W\big\{\phi_{\text{def}} + \phi_{\text{topo}} + \phi_{\text{flat}} + \phi_{\text{atmo}} + \phi_{\text{noise}}\big\}$$
其中去平地效应与外部 DEM 模拟地形相位后，残余地形误差相位与高度模糊数 $h_a$ 为：
$$h_a = \frac{\lambda R \sin\theta_i}{2 B_\perp}, \qquad \phi_{\text{topo,res}} = -\frac{2\pi}{h_a} \Delta h$$
- **基线选对规则**：C 波段 Sentinel-1（$\lambda \approx 5.55\,\text{cm}$）地形失相干阈值通常取 $|B_\perp| < 150\,\text{m}$；L 波段 NISAR（$\lambda \approx 24\,\text{cm}$）植被穿透更强，可放宽至 $|B_\perp| < 400\,\text{m}$；短时间基线优先取重访倍数（6/12/24 天）。

## 2. 相干性估计与 Goldstein 滤波

复相干系数 $\gamma = \frac{|E[s_1 s_2^*]|}{\sqrt{E[|s_1|^2] E[|s_2|^2]}} \in [0, 1]$。在低相干植被区建议多视窗口（Sentinel-1 IW 推荐 Range×Azimuth = `10×2` 或 `20×4` 获得近似正方形地面像元 `~28m`），随后应用 Goldstein 自适应频域滤波（$\alpha \in [0.4, 0.7]$，过高会导致强形变梯度处相位过度平滑）。

## 3. SNAPHU / MCF 最小费用流二维解缠与跳周检核

- 解缠前**必须对低相干水体与 layover/shadow 掩膜**，防止低相干噪声节点在解缠网络中引发整片 $2\pi$ 整数周跳（Phase Unwrapping Jump）。
- 城镇断裂带或矿区大梯度形变优先启用 SNAPHU `DEFO` 模式 + `STATCOSTMODE`，并在解缠后利用闭合环相干性或连接分量图（`connComp > 0`）验证。
