# xDEM 三维配准、垂直基准对齐与大地测量物质平衡

## 1. 核心原理：为什么不能直接 `DEM_t2 - DEM_t1`？

陡峭山区的亚像元水平偏移（$\Delta x \approx 3\sim 15\,\text{m}$）会在坡度 $\alpha$ 上产生系统性伪高程误差 $\Delta h \approx \Delta x \cdot \tan\alpha$，呈现出**随坡向呈正弦波 $\cos(\psi - \psi_0)$ 分布**的假薄化/增厚信号。必须使用 Nuth & Kääb (2011) 方法在稳定裸露基岩上迭代求解平移矢量 $(a, b, c)$：

$$\frac{dh}{\tan\alpha} = a \cdot \cos(b - \psi) + c$$

## 2. `xDEM` 标准三维配准与 $dh/dt$ 计算模板

```python
import xdem
import geoutils as gu

ref_dem = xdem.DEM("cop30_ellipsoidal.tif")
tba_dem = xdem.DEM("historical_dem_ellipsoidal.tif").reproject(ref_dem)
glacier_outlines = gu.Vector("RGI2000-v7.0-G-13_central_asia.gpkg")

# 仅在冰川外且非水体的稳定基岩区配准
stable_mask = ~glacier_outlines.create_mask(ref_dem)

coreg = xdem.coreg.NuthKaab() + xdem.coreg.Tilt()
aligned_dem = coreg.fit_and_apply(ref_dem, tba_dem, inlier_mask=stable_mask)

dh = aligned_dem - ref_dem
stable_nmad = xdem.spatialstats.nmad(dh[stable_mask])
print(f"Stable terrain NMAD after coregistration: {stable_nmad:.3f} m")
```

可在「设置 -> 运行环境与工具 -> 本机 Conda」中一键确认安装 `xdem`。
