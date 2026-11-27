# Sentinel-1 SLC-BURST 检索与下载

`SLC-BURST` 是 ASF 官方提供的 Sentinel-1 Burst 级目录和资产，不是第二个完整场景 `SLC`。它留在原 ASF SAR 影像来源卡片、原结果工作台和原下载入口中；Dataset 切换只改变能力驱动字段，不创建第二套界面状态。

## 运行前提

- 打包和开发环境使用经过审查的 `asf-search>=12.1,<13`。
- 运行时必须同时具备官方 `SLC-BURST`/`BURST` 常量、三个 Burst ID 参数、结构化资产和 Cookie 支持；任一能力缺失都会禁用该路径并返回明确错误。
- 普通 Sentinel-1 检索/下载可以使用 Earthdata Token。
- Burst TIFF/XML 下载需要 Earthdata 账号密码或安全解析的 `.netrc` 建立内存中的 `asf-urs` Cookie。仅有 Token 时会在传输前返回 `ASF_BURST_COOKIE_REQUIRED`，不会创建伪任务或回退成完整 SLC。

Cookie 只保存在 Rust 下载进程内存中，不写入 SQLite、任务快照或日志。账号、密码、Token、Cookie 和带凭据 URL 均不得出现在错误详情中。

## 检索条件

选择 Dataset `SLC-BURST` 后，远程 Burst 条件仅包括：

- Absolute Burst ID；
- Relative Burst ID；
- Full Burst ID。

Subswath 和 Burst Index 来自结果结构化元数据，只用于结果表和详情展示，不作为伪造的远程筛选。检索使用官方 `asf_search.search_generator`，空结果是成功的空集合；认证失败、运行时能力缺失或资产不完整是类型化错误，不回退到父 SLC。

产品身份使用 `ASF::SLC-BURST::fileID`。父粒度只从 UMM `InputGranules` 读取，不能根据文件名猜测。原始 properties、UMM、几何、Burst ID、Subswath、Burst Index 和父粒度会保留在 `native_metadata` 中。

## 下载资产

每个勾选的 Burst 产品可以选择：

- TIFF + XML（默认）；
- 仅 TIFF；
- 仅 XML。

下载任务会把父产品展开为稳定的资产身份：

```text
<product_key>::asset::primary
<product_key>::asset::metadata
```

任务只冻结已勾选产品及所选资产，保留官方文件名、真实扩展名、可信 HTTPS URL、大小、checksum、输出目录和目录策略。不会从 `scene_id` 构造 `.zip`，也不会把未勾选的完整候选集复制进任务。

## HTTP 与恢复规则

- 只接受 `https://sentinel1-burst.asf.alaska.edu/...` 的 `.tiff` 和 `.xml` 资产。
- OAuth/Cookie 建立只允许受信 Earthdata/ASF HTTPS 跳转；HTTPS 下载禁止降级到 HTTP。
- 跨来源跳转不会携带 Authorization。
- HTTP 202 按有上限且可取消的 `Retry-After` 等待，不会提前创建 `.part`。
- HTTP 307 只在受信规则下继续；403 映射为 Burst Cookie 认证错误。
- 最终资产响应才进入 Range、大小校验、`.part`、重试和原子完成流程。

## 手动验证

1. 在原 ASF 卡片切换 `SENTINEL-1` 与 `SLC-BURST`，确认只有一个工作台和一个下载按钮。
2. 确认 `SLC` 与 `SLC-BURST` 含义清楚，Burst 只显示三个官方 ID 字段。
3. 检索固定 Burst ID，检查结果中的 Subswath、Burst Index、TIFF/XML 文件名和原始元数据。
4. 仅配置 Token 时启动下载，确认收到 Cookie 所需提示且没有残留任务或文件。
5. 配置账号或 `.netrc` 后分别测试“TIFF + XML”“仅 TIFF”“仅 XML”，确认扩展名、任务行、暂停/继续和输出目录正确。
