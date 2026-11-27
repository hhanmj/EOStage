# ASF SAR 检索与下载说明

> ASF 目录按 Provider/Dataset 能力驱动，支持 Sentinel-1、
> SLC-BURST、ALOS PALSAR、ERS、JERS-1、RADARSAT-1 和 SEASAT。Sentinel-1 GRD 会保留
> `GRD_HD/HS/MD/MS/FD` 原生级别；默认直接写入用户选择目录，可选按
> 产品/处理级别建子目录。完整设计见
> [SAR Provider architecture](sar-provider-architecture.md)。

EOStage 使用一个统一 ASF 工作台，不再为多任务目录另建检索卡片。Dataset、平台、原生产品/处理级别、模式和极化在原表单中级联；结果共用原结果表、地图、勾选状态和下载按钮。

内部身份使用 `provider::dataset::provider_product_id` 组成的 `product_key`。`scene_id` 只用于显示和旧项目兼容，不能再作为跨数据集去重、选择、地图或任务主键。

## 当前数据集与处理级别

| Dataset | 当前入口 | 处理级别说明 |
| --- | --- | --- |
| Sentinel-1 | ASF | `SLC`、`RAW`、`OCN` 和 `GRD_HD/HS/MD/MS/FD`；旧值 `GRD` 表示全部 GRD，不会静默变为 `GRD_HD`。 |
| SLC-BURST | ASF 官方 `asf_search` | `BURST`；按官方 Burst ID 检索并保留 TIFF 与 XML 两个真实资产。它不是第二个完整场景 `SLC`。 |
| ALOS PALSAR | ASF | 由本机合同中的 ASF Capabilities 提供 L1.x、RTC 等合法值。 |
| ERS | ASF | 使用 ASF 原生处理级别和 ASF Frame 能力。 |
| JERS-1 | ASF | 使用 ASF 原生处理级别和 ASF Frame 能力。 |
| RADARSAT-1 | ASF | 使用 ASF 原生处理级别和 ASF Frame 能力。 |
| SEASAT | ASF | ASF Frame 按字符串处理，保留前导零。 |

ENVISAT ASAR 当前不在 ASF 数据集列表中。本版只保留未来 ESA Provider 的扩展边界，不提供虚假的 ENVISAT 检索或下载。

## 检索逻辑

检索通常由以下条件共同决定：

- AOI 几何范围。
- 日期范围。
- 产品族与原生处理级别，例如 `GRD` + `GRD_HD`。
- 轨道方向：升轨、降轨。
- Path / Frame 起止范围。
- 极化：例如 VV、VH、VV+VH。

AOI 使用检索时冻结的真实几何，不用 bbox 代替任务拓扑。Frame 控件的标签、参数名和值类型由 Dataset Capabilities 决定：Sentinel-1 使用 ESA Frame，历史任务按能力使用 ASF/JAXA Frame；SEASAT 的前导零不会被转成数字后丢失。

影像数量留空时会先读取 ASF 匹配总量，并以 10,000 景作为单次工作台安全上限；
匹配总量与最终唯一产品数是两个指标。大结果会按时间窗口或官方分页继续检索，
中断后的不完整窗口不会作为成功结果导入。

平台、方向、成像模式和极化均为多选。显示“全部”表示不限制该字段，而不是只提交第一项。Dataset 切换时会清除不再合法的旧选项；Path/Frame 必须同时给出起止值，不能把半个范围发送给 ASF。

SLC-BURST 只显示 ASF 官方支持的 `Absolute Burst ID`、`Relative Burst ID` 和 `Full Burst ID` 三类远程筛选。Subswath 与 Burst Index 是结果元数据，不会伪装成远程筛选条件。详见 [SLC-BURST 说明](sar-burst.md)。

## SAR 工作台

检索结果进入 SAR 工作台后，可以：

- 搜索日期、轨道、文件名等字段。
- 勾选单景或全选当前列表。
- 点击影像后在地图上高亮。
- 查看完整元数据。
- 对已检索或导入的影像启动下载。

“全选”表示选择当前可见列表：没有搜索条件时选择全部，有搜索条件时选择筛选后的列表。

## 下载队列

下载中心负责真实下载任务：

- 并发下载。
- 暂停和继续。
- 结束任务。
- 失败重试。
- 断点续传。
- 查看详细日志。

每个任务应绑定创建时的输出目录，避免用户后续修改目录后影响重试和断点续传。

下载不假定文件为 ZIP。通用任务冻结 Provider 返回的真实文件名、可信 URL、远端大小和 checksum；默认直接写入用户选择目录，可选按产品/处理级别建安全子目录。当前 Dataset 已冻结在任务快照中，不作为用户重复选择项。Sentinel-1 兼容路径仍保留原成熟下载行为。

SearchBatch 保存完整候选、查询、AOI、勾选和页码。创建下载时会建立一个不激活工作台的独立 DownloadSnapshot 批次，只冻结实际选择的产品/资产、原查询、AOI、输出目录和源批次 ID；未勾选候选不会复制进下载任务。10,000 候选压力测试用于阻止任务快照随完整候选集膨胀。旧版内嵌完整任务快照仍可只读兼容恢复。

## 网络与代理

如果用户开启代理，下载和检索应使用用户设置的代理；如果未开启代理，应使用系统默认网络。

常见失败原因：

- Earthdata / ASF 凭据未配置或已过期。
- SLC-BURST 仅配置了 Token，未提供可建立 `asf-urs` Cookie 的账号密码或 `.netrc`。
- 网络无法访问 ASF 或 Earthdata。
- 代理未正确工作。
- 目标目录无写入权限。
- 下载链接过期，需要重新获取。

## 手动导入

用户也可以通过 ASF 官方文件、URL、脚本或本地文件名导入候选影像。导入后应进入同一个 SAR 工作台，方便继续筛选、查看元数据和下载。
