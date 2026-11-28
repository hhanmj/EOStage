# GEE 智能体任务执行闭环、审查清单与排错阶梯

GEE Python 代码本身并不直接运算栅格，而是在服务端拼接一张**延迟求值计算图（Computation Graph）**，直到被图瓦片渲染、`getInfo()`、`computePixels` 或 `Export` 触发求值。本卡规范 EOStage 中从需求澄清、选数校验、代码自检到排错交付的全链路纪律。

## 1. 六步标准化任务流程

1. **界定目标与消歧（Frame）**：明确 AOI（必须源自当前工作区或用户输入，缺失时主动澄清）、时间范围、目标变量、空间尺度及交付形态（在线图斑、地理栅格 GeoTIFF 还是表格 CSV）。
   - *模棱两可的提取请求先问再算*：例如“提取这里的水体”需区分**历史永久水体**（JRC GSW）、**当前光学瞬时水体**（S2/Landsat MNDWI）还是**有云期洪涝变化**（Sentinel-1 SAR）；“提取建筑”需区分已有开放建筑矢量还是当前影像目标分割。
2. **检索与验证（Discover & Verify）**：使用 `gee_catalog_search` / `gee_dataset_advisor` 进行中英检索、主题配数或对比；**关键数据集在写定量代码前必须确认 ID、波段名、缩放因子、掩膜规则与是否退役**，无证据宁可追问澄清也不编造冷门数据集 ID。
3. **可用性与导出预演（Availability & Preflight）**：调用 `gee_check_availability` 核清有效景数与覆盖时段；涉及栅格输出时调用 `gee_export_preflight` 校验像素规模、分块数与算法 `tile_safe` 属性。
4. **脚本科学审查（Static Review）**：任意自定义 `ee` Python 脚本必须先通过 `gee_script_inspect` 校验，确认无恶意系统调用、无高危反模式后再提交用户审批执行。
5. **最小探测再放大（Probe Before Scale）**：先用小尺度/短时间测试确认非空、波段正确和掩膜正常，再放量成图或导出；**禁止将“地图渲染成功”等同于“数值定标与投影对齐正确”**。
6. **血缘留痕与诚实交付（Provenance & Honest Handoff）**：附带真实数据来源、分辨率、掩膜方式和核验状态；受限于权限、配额或网络未执行的步骤必须直白说明为未验证。

## 2. `gee_script_inspect` 重点代码自检清单

提交自定义脚本供审批执行前，对照以下常见红线自查（也是 EOStage 内置科学审校器的重点审查对象）：

- **服务端控制流禁忌**：
  - 严禁在 `.map(fn)` 回调函数或 Python `for`/`while` 循环内调用 `.getInfo()`；
  - 严禁在 `.map(fn)` 内依赖 `print()` 期待逐像元调试输出或修改外部 Python 列表；
  - 避免 `collection.toList()` 与 `ee.Algorithms.If()`；使用 `filter()`、`limit()` 与条件掩膜组合替代。
- **统计与导出参数完整性**：
  - `reduceRegion()` / `reduceRegions()` 必须显式传入 `geometry`、`scale` 与 `maxPixels`（或说明 `tileScale`）；
  - `Export.image.*` 必须显式指定 `region`、`scale`（或 `crs` + `crsTransform`）与 `maxPixels`；
  - **离散分类产品**（如 `ESA/WorldCover`、`GOOGLE/DYNAMICWORLD`、`CLCD`）严禁出现 `.mean()`，应改用 `.mode()`、类别分组统计或乘以 `pixelArea()`。
- **传感器专属定标与掩膜**：
  - Sentinel-2 SR (`COPERNICUS/S2_SR_HARMONIZED`) 须具备 `Cloud Score+` 或 `SCL` 掩膜，跨越 2022–2024 不可单用 `QA60`，且须包含 `/ 10000` 或 `* 0.0001` 缩放说明（比值指数除外）；
  - Landsat C2 L2 须应用 `QA_PIXEL` 位掩膜与官方缩放/偏移 (`0.0000275 / -0.2`, `0.00341802 / +149.0`)；
  - MODIS 植被 (`MOD13Q1`) 与温度 (`MOD11A1/A2`) 须检查缩放系数 (`0.0001` / `0.02 K`) 及 QA 波段。
- **宿主授权规范**：受控脚本执行环境由宿主自动注入有效授权会话，脚本内无需强行插入会阻塞弹窗的 `ee.Authenticate()`。

## 3. 八步最小探测排错阶梯

遇到报错、全黑无像素或超时前，按由轻到重顺序定位（先用轻量元数据探针，避免盲目重写整份脚本）：

1. 查结构：用 `image.bandNames()`、`image.projection()`、`collection.size()` 验证非空与波段存在。
2. 查单景：手动调用一次映射函数 `fn(collection.first())` 验证单个元素逻辑是否成立。
3. 缩范围：临时将测试窗收窄到极小 AOI（如数公里方框）和 1–2 个月时段。
4. 查掩膜：检查 `.mask()`、`.selfMask()` 与是否全区被云掩膜筛空导致“无输出像素”。
5. 查投影与尺度：核对实际 `projection().nominalScale()` 与统计时 `scale` 是否匹配。
6. 调分块因子：轻量归约出现内存限制时，适度调大 `reduceRegion(..., tileScale=4)` 并排查是否存在大范围 `.reproject()`。
7. 转批量通道：交互计算确认超时，立即切换外部批量导出或按合规分块流程落盘。
8. 遇 HTTP 429 / 并发报错：降低并行度并采用退避重试，禁止疯狂连续重发请求。
