# EOStage 仓库结构与模块边界

本文件说明 **EOStage** 当前 Rust + Tauri 2 + React 仓库的目录结构与职责边界，便于贡献者快速定位代码与测试。

## 顶层目录

| 目录 / 文件 | 职责 |
| --- | --- |
| `Cargo.toml` / `Cargo.lock` | Rust 工作区配置与锁定依赖 |
| `crates/insar-core/` | 领域模型、资源注册表、检索筛选契约、任务状态机与通用类型 |
| `crates/insar-provider/` | 远端数据源适配器（ASF、CDSE、USGS M2M、Bhoonidhi、Planetary Computer、STAC、边界服务等） |
| `crates/insar-download/` | 断点续传与分片下载引擎、NISAR HDF5 Range 裁剪、重定向安全策略、系统钥匙串凭据管理 |
| `crates/insar-store/` | SQLite 持久化任务库、快照编解码与边界缓存 |
| `crates/insar-gis/` | 原生 GDAL/PROJ 栅格处理、高程基准转换、光学 AOI 裁剪与 MapGIS 互操作 |
| `crates/insar-application-service/` | 传输无关的应用服务编排层（目录检索、任务管理、边界服务、DEM/GACOS 规划） |
| `crates/insar-service/` | 持久化协议与存储调度服务 |
| `crates/insar-agent/`、`crates/insar-agent-model/`、`crates/insar-agent-mcp/` | 本地优先 AI 助手引擎、模型通信传输与 MCP 工具协议 |
| `crates/insar-subset-lab/` | NISAR 远端分片裁剪布局、Range 规划与元数据验证 |
| `src-tauri/` | Tauri 2 桌面宿主（`eostage-desktop`）、IPC 命令边界、CSP 安全策略、OAuth 流程、GEE 云端套件与开放地学工具 |
| `ui/` | React + TypeScript + Tailwind CSS 桌面端界面（核心入口 `ui/src/components/desktop/DesktopWorkbench.tsx`） |
| `contracts/` | 跨栈 JSON Schema 与静态契约定义（资源目录、底图清单、边界路由政策、许可证审查记录） |
| `runtime/plugins/` | 官方内置扩展插件包（`eostage-*-plugin`） |
| `packaging/` | 桌面图标与组件清单模板 |
| `scripts/` | 构建初始化、契约生成、依赖许可审计与公开仓库洁净度检查脚本 |
| `docs/` | 面向用户与贡献者的使用说明与技术文档 |

## 模块边界原则

1. **后端拥有状态真相**：下载队列、任务快照、凭据安全存储与边界缓存均由 Rust 后端与 SQLite/系统钥匙串持有，前端仅渲染投影与发送命令。
2. **凭据不进代码与日志**：所有第三方平台账号、Token 与 API Key 均通过操作系统钥匙串或受控凭据库保管，并在日志和外发请求中严格脱敏。
3. **单一桌面宿主**：桌面端统一由 `src-tauri`（`eostage-desktop`）承载，通过 Tauri IPC 与前端通信。
