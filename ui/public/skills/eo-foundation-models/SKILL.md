---
name: eo-foundation-models
description: "Earth observation foundation models & geospatial deep learning with TorchGeo: multi-band GeoTIFF lazy tile sampling, CRS-aware IntersectionDataset, pretrained backbones (ResNet/ViT/DOFA/Prithvi), and spatial-block validation."
---

# 对地观测基础模型与 TorchGeo 深度学习规程 (EO Foundation Models)

整合 `TorchGeo`（支持在设置中心 -> 运行环境与工具一键确认安装到本机 Conda）与对地观测深度学习生态：

## 三大核心约束（严防经典“假高精”陷阱）

1. **严禁使用仅支持 8-bit 3波段 RGB 的普通视觉数据集加载器**：卫星影像（Sentinel-2 10–12波段、Landsat C2 L2 uint16、SAR双极化 float32）必须使用 `torchgeo.datasets.RasterDataset` 按真实 CRS 与物理波段对齐裁剪切片（支持实时多源 `IntersectionDataset` 联合如 `Sentinel-2 & ESA WorldCover`）。
2. **禁止在像素尺度随机切分**：严格按空间网格（如 $5\,\text{km} \times 5\,\text{km}$）或整景区域划分 Train/Val/Test（遵循 `/remote-sensing-analysis` `spatial-cv-protocol.md`）。
3. **统一传感器辐射定标与通道顺序**：迁移微调 Prithvi、DOFA、SatMAE 或 TorchGeo 预训练权重时，严格核对预训练时使用的 Sentinel-2 L2A / HLS 波长顺序与均值方差归一化系数。

可运行 `TorchGeo` `IntersectionDataset` + `GridGeoSampler` 推理与微调代码见 [references/torchgeo-foundation-inference.md](references/torchgeo-foundation-inference.md)。
