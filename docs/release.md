# EOStage 发布、构建与组件化说明

本文说明 **EOStage** 桌面端的跨平台构建流程、可选原生 GIS 运行时集成方式以及发布前质量与许可证门禁。

## 1. 桌面构建入口

EOStage 基于 **Tauri 2 + Rust + React** 构建。不同平台的本地构建初始化脚本如下：

- **macOS**：`./scripts/initialize_macos_native_build.sh`
- **Windows**：`.\scripts\initialize_windows_native_build.ps1`
- **Linux**：`./scripts/initialize_linux_native_build.sh`

完成前端构建后，可使用 Tauri 官方 CLI 生成对应平台安装包：

```bash
cd ui
npm ci
npm run build:tauri
npx tauri build
```

## 2. 可选原生 GIS 运行时与高程基准网

DEM 基准转换、光学裁剪与部分栅格处理由 `crates/insar-gis` 调用受控的原生 GDAL/PROJ 运行时（位于 `runtime/gis`）和垂直基准网格（`runtime/vertical-grids`）完成：

- 当本地尚未安装原生 GIS 运行时，应用仍可正常编译和启动，检索与下载功能不受影响，涉及原生 GIS 的转换操作会明确提示缺少运行组件。
- 可使用 `scripts/build_native_gis_runtime.ps1`（Windows）或 `scripts/build_unix_native_gis_runtime.py`（macOS/Linux）按清单与 SHA-256 校验装配运行时目录。

## 3. 发布前必过门禁

在创建正式版本标签或分发构建包前，应逐项通过以下检查：

1. **前端构建与测试**：`cd ui && npm run check && npm test && npm run build:tauri`
2. **单宿主构建隔离校验**：`node scripts/check_tauri_single_host_bundle.mjs`
3. **Rust 格式、静态检查与测试**：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace -- --test-threads=1`
4. **公开洁净度检查**：`python3 scripts/check_public_hygiene.py`
5. **分发依赖许可审计**：`python3 scripts/audit_tauri_host_licenses.py --mode distribution`（要求 `GPL-3.0-or-later` 零不兼容依赖、零临时豁免）
6. **版本与说明核对**：核对 `src-tauri/tauri.conf.json`、`ui/package.json`、`CITATION.cff`、`CHANGELOG.md` 与 `THIRD-PARTY-NOTICES.md` 版本及声明一致。
