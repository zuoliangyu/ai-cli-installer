## 变更说明

<!-- 这个 PR 做了什么、为什么要做。关联 Issue 请写 `Closes #编号`。 -->

## 变更类型

- [ ] Bug 修复
- [ ] 新功能
- [ ] 重构 / 代码整理
- [ ] 构建 / CI / 发布流程
- [ ] 文档

## 影响范围

- [ ] 桌面端（Tauri）
- [ ] Web 模式（installer-web）
- [ ] 核心逻辑（installer-core）
- [ ] 前端界面（Svelte）

## 测试

<!-- 说明如何验证：运行的命令、测试的平台（Windows / macOS / Linux）、截图等。 -->

- [ ] `cargo clippy --workspace --all-targets`
- [ ] `cargo test --workspace`
- [ ] `npm run check` 与 `npm run build`
- [ ] 已在实际环境中手动验证

## 检查清单

- [ ] 新增或修改命令时，已同步更新 `tauriApi.ts`、`webApi.ts`、`api.ts` 与 `installer-web/src/routes.rs`
- [ ] 面向用户的变更已更新 `CHANGELOG.md`
- [ ] 不包含密钥、令牌等敏感信息
