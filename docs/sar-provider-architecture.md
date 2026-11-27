# SAR Provider 架构说明

本文说明 **EOStage** 中能力驱动（capability-driven）的多数据源 SAR 检索与交付架构。详细仓库目录结构见 [仓库结构与模块边界](architecture/repository-map.md)，当前交付能力概览见 [CHANGELOG](../CHANGELOG.md)。

## 1. 核心设计原则

1. **能力声明驱动界面与校验**：各数据集支持的产品类型、极化、轨道方向、时间范围和资产交付路径由 `contracts/v3/asf-capabilities.v1.json`、`contracts/v3/resource-registry.v1.json` 与 `contracts/v3/search-filter-schemas.v1.json` 统一声明，前端筛选栏与后端校验共享同一事实源。
2. **逻辑产品、服务记录与物理资产分离**：检索结果只保存标准化产品与元数据标识；下载任务创建时冻结当次选择快照，与后续新检索完全隔离。
3. **科学语义不混用**：
   - Sentinel-1 SLC / GRD / SLC-BURST、ALOS PALSAR、ALOS-2、ERS、JERS-1、RADARSAT-1、SEASAT 与 NISAR L-band / S-band 各自保留独立的身份、极化与覆盖语义。
   - 完整归档（如 `.zip` / `.h5`）与客户端派生的 AOI 裁剪文件（如 `_SUBSET.h5`）在任务记录与命名上严格区分。

## 2. 主要数据源与模块位置

| 数据源 / 能力 | 负责模块 |
| --- | --- |
| 资源目录与筛选契约 | `crates/insar-core/src/resource_registry.rs`、`crates/insar-core/src/search_filter_schema.rs` |
| ASF CMR / Search（Sentinel-1、SLC-BURST、NISAR L-band、ALOS、ERS 等） | `crates/insar-provider/src/asf/` |
| Bhoonidhi（NISAR S-band 等） | `crates/insar-provider/src/bhoonidhi/` |
| Sentinel-1 精密轨道匹配（POEORB/RESORB） | `crates/insar-provider/src/orbit_download.rs` |
| 检索冻结、任务创建与状态投影 | `crates/insar-application-service/src/` |
| 分片传输、断点续传与 HDF5 Range 裁剪 | `crates/insar-download/src/`、`crates/insar-subset-lab/` |

## 3. 检索与交付数据流

1. 用户在地图工作台或 AI 助手中绑定 AOI 并提交筛选条件。
2. `insar-application-service` 按契约校验参数并调用对应 `insar-provider` 执行远端检索，返回标准化候选列表与空间足迹。
3. 用户确认选中景并创建任务后，`insar-store` 持久化任务与子项，`insar-download` 按凭据会话与重定向白名单执行可暂停/可恢复的并发传输与完整性校验。
