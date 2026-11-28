# 导出前安全预演（Preflight）与通道分流规范

自然语言中的“帮我导出一份数据”往往隐含多组不可逆决策：产物形态（影像 vs 表格 vs 矢量）、存储通道、目标分辨率/投影、集合降维方法与配额预算。**严禁未经预演或参数不全就盲目提交高耗时导出任务，也严禁在静默降采样后冒充全分辨率结果。**

## 1. 导出前先走离线预演门禁

执行本地大范围导出或正式脚本导出前，优先调用 `gee_export_preflight` 完成离线规划（不直接耗费云端计算配额）：
- **缺失参数零妥协**：当研究区 `bbox`、数据集 ID 或必要参数缺失时，预演会标为 `needs_parameters` 并明确列出缺失项，须先补全再执行（不兜底套用默认测试城市）。
- **原始像元零截断估算**：预演以未经裁剪的真实原始宽高像素数 `raw_width_px × raw_height_px` 与实际分块数 `raw_total_chunks` 估算体量，让超限风险在动手前暴露出来。

## 2. 四级导出通道分流决策矩阵

| 条件判定 | 推荐通道（Route） | 执行与确认纪律 |
| --- | --- | --- |
| 缺少 `bbox` / 数据集等关键输入 | `needs_parameters`（拦截等待补全） | 向用户明确一次性问清缺项，严禁静默猜测 |
| 本机直拉分块数 `<= 9` 且算法 `tile_safe` | `local_chunked_geotiff_auto` | 自动按请求原生分辨率分块下载并本机拼接为配准 GeoTIFF |
| 本机直拉分块数 `10..=64` 且算法 `tile_safe` | `local_chunked_geotiff_confirm` | 必须弹出确认：由用户主动选择“坚持原生分辨率分块拼接”或“改用预览分辨率” |
| 分块数 `> 64`、或 `tile_unsafe` 且超过 1 块、或显式指定外部目标 | `external_batch_export` | 本机直拉存在超时与算法断裂风险，引导使用 `ee.batch.Export.*` 写入 Drive / Cloud Storage / EE Asset，或缩小研究区/明确降采样 |

## 3. 产物形态与集合降维契约

- **`ImageCollection` 禁止直接作为单一栅格导出**：导出影像前必须根据科研目标降维成单幅 `ee.Image`（如季节 `median()`、年内极端 `max()`、`qualityMosaic()` 或带序 `mosaic()`）；若目标是多期汇总曲线/清单，应先 `map()` 为 `FeatureCollection` 走表格导出。
- **分类产物对比**：需要分区面积表时走 `FeatureCollection` 导出（CSV/GeoJSON），不要误把面积表写成 GeoTIFF 下载。
- **可视化附图 vs 科研栅格**：PNG/HTML 预览仅用于交互诊断与附图沟通，不能代替带精确地理参考的 GeoTIFF/COG 交付物。

## 4. `ee.batch.Export` 核心参数规范

生成或审核批导出脚本时，须显式写明并核验以下参数：

```python
task = ee.batch.Export.image.toDrive(
    image=final_image,
    description="clear_task_name",
    folder="earthengine_exports",
    fileNamePrefix="clear_file_prefix",
    region=exact_roi,
    scale=30,
    crs="EPSG:4326",  # 需严格格网对齐时改用 crs + crsTransform
    maxPixels=1e10,
    fileFormat="GeoTIFF",
    formatOptions={"cloudOptimized": True},
)
```

1. **精确对齐**：仅传 `scale` 时 GEE 会自动推算原点，若须与已有 DEM/分类网格逐像元无缝对齐，必须同时指定 `crs` 和 `crsTransform`（或参考网格参数）。
2. **`nodata` 处理**：保持分析掩膜语义用于统计；导出 GeoTIFF 给外部 GIS 软件读取时，如像素类型与掩膜默认填充冲突，可显式 `.unmask(nodata_value)` 并保证 `nodata_value` 落在输出位深范围内（如 Float32 用 `-9999.0`，UInt8 勿用负数）。
3. **默认不静默启动高开销导出**：受控脚本审查如发现配置了 `Export.*` 但未执行 `.start()`，属安全的“参数预备待确认”模式；待用户核清目标位置、分辨率和 `maxPixels` 后再启动任务，并在结果摘要中完整回报任务名、目标去向、区域与分辨率假设。
