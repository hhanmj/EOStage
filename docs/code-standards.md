# EOStage 代码规范

本规范适用于 **EOStage** 的 Rust 后端、Tauri 宿主与 TypeScript/React 前端代码。

## 1. 结构与职责

- 保持模块职责单一：业务契约与算法放 `crates/`，桌面命令胶水放 `src-tauri/src/`，纯状态逻辑放 `ui/src/lib/` 或 `ui/src/shell/api/`，组件渲染放 `ui/src/components/desktop/`。
- 不要在前端组件中直接绕过 `ui/src/lib/bridge.ts` 或 `ui/src/lib/desktopShellBridgeTauri.ts` 散落调用底层 IPC。
- 不要把未验证的元数据编造成默认值；未知、不可用、已暂停、已取消与失败必须严格区分。

## 2. 安全与隐私

- 严禁在代码中提交个人账号、密钥、本机绝对路径或未获开源许可的第三方代码。
- 外链、重定向、文件路径与子进程参数必须经过白名单与边界校验。
- 新增或改编第三方开源代码时，必须保留原版权声明并在根目录 [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md) 中登记。

## 3. 代码风格与检查

- **Rust**：提交前确保 `cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 全绿通过。
- **TypeScript / React**：保持严格类型检查（`cd ui && npm run check`），前端测试断言避免直接把已挂载的 DOM 节点传入 `assert.equal`（使用布尔断言如 `assert.ok(el === null)`）。
- **多语言与文案**：面向用户的文案同时维护 `ui/src/i18n/locales/{zh-CN,en,ja}/` 三语条目，避免残留内部工程代号。
- **公开洁净度与许可门禁**：提交前运行 `python3 scripts/check_public_hygiene.py` 与 `python3 scripts/audit_tauri_host_licenses.py --mode distribution`。
