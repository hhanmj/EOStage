# TorchGeo 地理空间采样、多源交集对齐与基础模型推理

## 1. 按真实物理坐标实时对齐图像与掩膜（零手工重裁剪）

```python
from torch.utils.data import DataLoader
from torchgeo.datasets import RasterDataset, stack_samples
from torchgeo.samplers import GridGeoSampler

class Sentinel2PatchDataset(RasterDataset):
    filename_glob = "*S2*.tif"
    is_image = True

class LandCoverMaskDataset(RasterDataset):
    filename_glob = "*WorldCover*.tif"
    is_image = False

img_ds = Sentinel2PatchDataset(paths="./s2_folder")
lbl_ds = LandCoverMaskDataset(paths="./wc_folder")
# TorchGeo 自动取时间/空间交集并按相同 CRS/分辨率重采样
joint_ds = img_ds & lbl_ds
sampler = GridGeoSampler(joint_ds, size=256, stride=192)
loader = DataLoader(joint_ds, sampler=sampler, batch_size=8, collate_fn=stack_samples)
```

## 2. 常见预训练骨干与 TorchGeo Weights

- `torchgeo.models.ResNet50_Weights.SENTINEL2_ALL_MOCO`（13波段 Sentinel-2 无监督表征）
- `torchgeo.models.ViTSmall16_Weights.SENTINEL2_ALL_DINO`（ViT 多光谱自监督权重）
- `DOFA` / `Prithvi-EO`（多光谱+SAR 动态波长自适应编码器，推理时须传入每通道中心波长 `wavelengths` 列表）。
