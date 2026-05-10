# Changelog

本项目所有重要变更记录。遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式，版本号遵循 [SemVer](https://semver.org/lang/zh-CN/spec/v2.0.0.html)。

## [Unreleased]

## [0.2.4] - 2026-05-10

### 「故障排查 / 配置补丁」扩充到 33 条 + 标签筛选 + 远程缓存

修复列表从 5 条扩到 33 条，覆盖 CC-006 / CC-005 / CC-017 / CC-WIN / CC-SHELL / CC-SAFE / CC-NET / CC-MCP / CC-TUI / CC-PRIVACY / CC-COMMON 各类。每条仍是对 `~/.claude/settings.json` 或 `~/.claude.json` 的 dotted-path 合并写入，保留所有其它字段。

### 新增

- **远程优先 + 编译时兜底**：app 启动时按 `raw.githubusercontent.com → gh-proxy / fastgit / github.chenc.dev` 顺序拉 `fixes.json`（每条 5s 超时），远程 `updated_at ≥ embedded` 才采用。新加修复改 JSON 推 main 即可对联网用户生效，不需要发版
- **进程内缓存**（`LazyLock<Mutex<Option<Vec<Fix>>>>`）：跨 tab 切换不再重复网络请求；缓存命中时仍重新读盘标注配置状态
- **多标签筛选面板**：每条 fix 在 JSON 里带 `tags` 字段（前端无硬编码分类），用户可勾选多个标签做 OR 组合，叠加三档状态过滤（全部 / 已配置 / 未配置）
- **分页**：每页 10 条，标签或状态变化时回到第 1 页

### 改动

- `FixesSection.svelte` 中 `filteredFixes` / `pageCount` / `pagedFixes` / `configuredCount` 由命令式函数改为 `$derived` 响应式派生值
- 「推荐标签」呼吸动画移除
- `App.svelte` 让 `FixesSection` 常驻挂载（`display: contents/hidden` 切换可见性），保证缓存跨页切换有效

### 修复（v0.2.4 内补丁）

- 分页按钮一点就崩：`setPage` 里把 `pageCount` 当函数调用了（`pageCount()`），但同次重构已经把它改成 `$derived` 值，应去掉括号
- `tagOptions` 在模板里被以函数调用、每次响应式更新都重算 Map，改成 `$derived.by(...)` 与同文件其他派生值保持一致

### 内部

- 新增 `crates/installer-core/src/fixes.rs` 中 `cached_fixes` / `cache_fixes` / `remote_is_fresh_enough` 等辅助
- 新增 4 个单元测试：远程更新逻辑、缓存读写

### 贡献者

- [@LS-plan](https://github.com/LS-plan) (ShuyuS) — feature PR [#2](https://github.com/zuoliangyu/ai-cli-installer/pull/2)：33 条修复扩充、远程缓存、标签筛选面板、分页、`$derived` 重构
- [@zuoliangyu](https://github.com/zuoliangyu) — 代码审查 + 合并后修正（`pageCount()` 调用、`tagOptions` 改为 `$derived`）

## [0.2.3] - 2026-05-08

### 新增：手动检查更新 + 启动自动检查

后端的 `tauri-plugin-updater` 早就配好了 endpoint 和 pubkey，但前端从来没接进去。这版接上，参考同系列 [AI Session Viewer](https://github.com/zuoliangyu/AI-Session-Viewer) 的设计：

- **启动 1.5 秒后静默检查**一次（避开 `list_tools` 的网络抢资源），发现新版本弹系统对话框「立即更新 / 暂不更新」，点更新就走 `downloadAndInstall` → `relaunch`
- **关于页 → 项目卡片**新增 `UpdateIndicator` 区块：当前版本号、手动「检查更新」按钮、release notes 预览、下载进度条、安装中、失败重试等所有状态
- **Sidebar 底部版本号**改为可点击按钮，跳到关于页；有新版本时旁边显示青色 ping 小点
- **右下角浮窗 `UpdateToast`**：发现新版且未 dismiss 时悬浮显示「v X → v Y / 更新并重启 / 忽略」
- 「忽略此版本」会写 `localStorage`（`aci_update_dismissed_version`），同版本不再二次打扰；下次有更高版本再次提示

### 内部

- 新增 `src/lib/updateStore.ts`：Svelte writable + `loadCurrentVersion` / `checkForUpdate` / `downloadAndInstall` / `openDownloadPage` / `dismiss` / `runStartupCheck` 一组 action
- 新增 `src/lib/components/UpdateIndicator.svelte`、`UpdateToast.svelte`
- 复用已有的 `@tauri-apps/plugin-updater` + `@tauri-apps/plugin-process` + `@tauri-apps/plugin-dialog`（package.json 之前就装了）

## [0.2.2] - 2026-05-08

### 修复

- **启动时不再闪 cmd 黑窗群**：Windows 下 `list_tools` 在初始化时会并发 spawn 数个 `cmd /c npm` / `cmd /c pnpm` / `where` 等 console 子进程，每个都会闪一个黑窗。给所有 spawn 加上 `CREATE_NO_WINDOW (0x08000000)` 创建标志，子进程依然能正常 IPC，但不再出现窗口。

### 内部

- `crate::proc` 新增 `silence_windows` / `silence_windows_std` 两个公共 helper，对外暴露给 `tokio::process::Command` 和 `std::process::Command` 两种调用方
- 静默覆盖：诊断/检测路径（`shell_command` / `run_executable` / `resolve_command_path`）、`env_manager::windows` 的 PowerShell（系统 PATH 用 UAC outer + 用户 PATH 用 local）、`installer::run_self_install` 的 `claude install`、`app_state::open_path_with_system` 的 `cmd /c start`

## [0.2.1] - 2026-05-08

### 安装诊断更鲁棒，覆盖 pnpm / yarn / bun / nvm

`detect_installed` 之前在 Windows 走 `Command::new("codex")` 直接 spawn，碰到 `.cmd` shim、桌面进程 PATH 不全、nvm-windows 这些场景会失败 → 顶部显示"未安装"，但实际机器上是有的。这版把整条命令调用链统一收口到 `crate::proc`，并扩展了诊断来源。

### 新增

- **pnpm / yarn / bun / nvm 检测**：诊断列表里独立标识各家全局安装，分别带颜色徽章（pnpm 琥珀 / yarn 蓝 / bun 粉 / nvm 翠绿）
  - pnpm 走 `pnpm bin -g`，yarn 走 `yarn global bin`，bun 读 `BUN_INSTALL` 环境变量或 `~/.bun/bin`
  - nvm 扫 `~/.nvm/versions/node/*/bin/<cmd>`（Mac/Linux）和 `%APPDATA%\nvm\v*\<cmd>(.cmd|.exe|.ps1)`（Windows nvm-windows）
- **PATH 状态徽章**：每条 install 行新增三态——绿色「当前 PATH」/ 黄色「被遮蔽」/ 红色「未在 PATH」
- **PATH 提示区块**：当所有安装都不在 PATH，红色提示提醒命令在终端会找不到；当有部分被遮蔽，黄色列出被同名命令拦截的来源
- **`crate::proc` 模块**：`shell_command` / `run_executable` / `resolve_command_path`，统一在 Windows 下走 `cmd /c` 来跑 `.cmd`/`.bat` shim；解决 `Command::new("npm")` 在 Tauri 桌面进程下不可靠的问题

### 改动

- `tools/codex.rs`、`tools/claude_code.rs` 的 `detect_installed`：从直接 spawn `codex` / `claude` 改为先 `where`/`command -v` 解析路径再 `cmd /c <path> --version`
- `npm_installer.rs` 全部 `Command::new("npm"|"node")` 收口到 `shell_command`（影响 `npm prefix -g` / `npm list -g` / `npm cache add` / `npm install -g`）
- `detect_npm_global` 加 fallback：`npm list -g` 失败也会扫已知 npm 全局 bin 目录（`%APPDATA%\npm`、`%APPDATA%\nvm\nodejs`、`~/.npm-global/bin`、`/usr/local/bin`、`/opt/homebrew/bin`），找到 shim 就跑 `--version` 拿版本号
- `Tool::detect_installed` 返空时，`list_tools` 从 `installations` 反推 `installed_version`（按 current_path → 任一带版本的优先级），避免顶部「未安装」与诊断「找到 v0.x.x」不一致
- `where`/`command -v` 解析失败但仅有一条 install 在 PATH 中时，自动认定其为 PATH winner（标 `current_path=true`），避免错标"被遮蔽"
- `ToolInstallation` 新增 `on_path: bool` 字段（与 `current_path` 区分：在 PATH 但未 winning）
- 隐藏 launcher_dir PATH 行（一键加入系统 PATH 按钮）当用户没有 native 安装时——npm/pnpm/yarn/bun 用户不需要这步

### 修复

- Windows 上 npm 装的 codex/claude 顶部一直显示"未安装"
- `where` 解析失败时全部 install 都被错标"被遮蔽"

## [0.2.0] - 2026-05-08

### UI 重构：与 AI Session Viewer 对齐设计语言

参考 [AI-Session-Viewer](https://github.com/zuoliangyu/AI-Session-Viewer) 的视觉，把单页布局改成 **Sidebar + 多页 + Cerulean Flow 主题**，作为同系列工具保持视觉一致。功能与 v0.1.0 等价。

### 新增

- **Tailwind CSS + Cerulean Flow 主题**：HSL 变量驱动的青绿主色调，明暗双套配色
- **三档主题切换**（亮色 / 跟随系统 / 暗色）：Sidebar 底部 ☀ ▣ ☾ 三按钮，写入 `localStorage`，自动响应系统主题变化
- **左侧 Sidebar 导航**：CLI 工具 / 中转预设 / 配置修复 / 关于，本地 page store 切换主区域
- **Sidebar 镜像状态卡**：常驻显示镜像可用数量与每条镜像延迟，点击重新探测
- **「关于」整页**：作者 / 邮箱 / QQ 群 / 哔哩哔哩 / GitHub，并加「同系列工具 → AI Session Viewer」链接
- 引入 `lucide-svelte` 图标库

### 改动

- `App.svelte` 拆为 `Sidebar` + 内容区两栏布局，主内容区域居中固定宽度
- `app.css` 由 `--bg/--accent` 等浅色橙主题改为 Cerulean Flow 的 HSL token 体系，配合 Tailwind `@apply`
- 所有组件（`ToolCard` / `PresetSection` / `FixesSection` / `About` / `ProgressBar`）改为 Tailwind class，按 Session Viewer 的卡片 / 徽章 / 模态框形态重写
- 「关于」由 modal 改为整页；旧的「v 版本号 · 关于」footer 入口移到 Sidebar 导航
- `MirrorStatus.svelte` 删除，能力合并进 Sidebar
- 新增 `src/lib/theme.ts` 与 `src/lib/page.ts` 两个 store

### 内部

- `tailwind.config.js` 把 HSL CSS 变量映射成 `bg-primary` / `text-muted-foreground` / `border-border` 等 Tailwind 颜色 token，方便后续与 Session Viewer 共享 class 体系
- `lucide-svelte` v1 已删除 `Github` 等品牌图标，About 内 GitHub 入口改用内联 SVG path

## [0.1.0] - 2026-05-08

### 重大变更：架构解耦

参考 [AI-Session-Viewer](https://github.com/zuoliangyu/AI-Session-Viewer) 的分层结构，把单 crate 改成 **Cargo workspace + 双前端 API 层**。运行行为对终端用户保持一致；面向开发者是一次结构性重构。

### 新增

- **Cargo workspace**（3 个 crate）：
  - `crates/installer-core` — 纯逻辑核心库，零 Tauri 依赖。包含 mirrors / downloader / verifier / installer / npm_installer / platform / upstream / fixes / presets / install_diagnostics / env_manager / tools。`fixes.json` 也跟着挪进核心
  - `src-tauri` — 仅留 Tauri 入口 + 薄壳 commands，每个 command 转发到 `installer_core::app_state::*`
  - `crates/installer-web` — Axum HTTP + WebSocket 服务，`rust-embed` 嵌入 `dist/`，与桌面端共享同一份核心逻辑
- **Web 模式**：可作为本地 / 远程服务器运行，浏览器访问。`/api/*` 路由覆盖 14 个原 Tauri command；`/ws/progress` 用 `tokio::broadcast` 推送下载进度，与桌面端的 `download-progress` 事件等价
  - 启动：`npm run build:web && cargo run -p installer-web`
  - 配置：`INSTALLER_HOST` / `INSTALLER_PORT`（默认 `127.0.0.1:3210`）
- **前端双 API 层**（`src/lib/`）：
  - `services/tauriApi.ts` — `invoke` + `listen`
  - `services/webApi.ts` — `fetch` + `WebSocket`，函数签名 1:1 对齐 tauriApi
  - `api.ts` — 编译时按 `__IS_TAURI__` 派发；保留所有命名导出，组件 `from "../api"` 不变
- Vite 加 `__IS_TAURI__` / `__APP_VERSION__` define，footer 版本号改用编译时变量

### 改动

- **进度回调抽象**：`AppHandle::emit("download-progress", ...)` 替换为 `ProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>`。Tauri 侧 wrap 成 emit；Web 侧 wrap 成 broadcast
- **`fixes.json` 远程 URL** 改指向新位置 `crates/installer-core/fixes.json`（旧客户端落到 embedded fallback，无功能影响）
- **构建产物路径**：从 `src-tauri/target/release/bundle/` 改为 workspace 根 `target/release/bundle/`（cargo workspace 默认行为）
- **CI workflow**：rust-cache `workspaces` 字段从 `src-tauri` 改为 `.`；删除遗留的 `src-tauri/Cargo.lock`，根 `Cargo.lock` 纳入版本控制

### 内部

- 新增 `installer_core::progress::{DownloadProgress, ProgressCallback, noop_progress}`
- 新增 `installer_core::app_state` 服务层 — `list_tools / install_tool / list_fixes / apply_fixes / check_path_status / list_claude_presets / open_path` 等 14 个统一入口，桌面端和 Web 端共用
- `Tool::install` 签名 `app: AppHandle` → `progress: ProgressCallback`
- `PathScope` 加 `Deserialize`，HTTP `POST /api/path/{add,remove}` 可直接收 `{"scope":"user"|"system"}` JSON
- 前端 vite 构建产物按模式自动二选一：Tauri 模式只打 tauriApi，Web 模式只打 webApi（动态 import + tree-shaking）

### 迁移注意

- 重构后 v0.0.12 已装的桌面用户可正常通过 updater 升级到 v0.1.0；行为没有用户可见变化
- 开发者拉取更新后，第一次 `cargo build` 会重新填充 root `target/`；旧的 `src-tauri/target/` 可手动删除
- `tauri.conf.json` 的 `frontendDist: "../dist"` 仍指仓库根 `dist/`，不变

## [0.0.12] - 2026-05-08

### 新增

- **安装诊断**：工具卡片新增「安装诊断」区块，分别识别 Native / npm 全局 / PATH 三个来源的安装，标注当前 PATH 上生效的来源、是否归本应用管理，并在检测到多重安装时给出处理建议
- **修复项配置状态**：「故障排查 / 配置补丁」区块按 `配置文件当前值 == 期望值` 判定每条 fix 是否已配置，支持 全部 / 已配置 / 未配置 三档过滤；已配置项可一键「取消配置」（仅当前值与期望值完全匹配时才会删除，避免误删用户自定义）
- **触达文件可点击**：应用 / 取消修复后的提示信息会列出受影响的 JSON 文件路径，点击用系统默认应用打开
- **启动窗口尺寸**：首次启动时自动按当前显示器 60% 尺寸居中（仅首次，后续启动尊重用户调整后的窗口位置）

### 修复 (v0.0.12 内补丁)

- `path_starts_with` 在 Windows 上改走组件级大小写无关比较，避免 `managed` 标记被路径大小写差异误判
- stable 通道镜像无独立指针时显式标记 `stable_falls_back_to_latest`，UI 显示「stable (跟随 latest v…)」防止版本号误读为 stable 自身的版本
- `open_path` 命令收紧：仅允许 `.json` 文件、需通过 `canonicalize`，Windows 改用 `cmd /c start ""` 用默认关联应用打开而不是不可靠的 `explorer.exe <file>`

### 内部

- 新增 `install_diagnostics` 模块：并发探测 native 路径 + `npm list -g --json` + `where`/`command -v` 三个来源，归一为 `ToolInstallation { source, version, path, current_path, managed }`
- `fixes.rs` 加 `annotate_config_status` / `remove_dotted` / `get_dotted` + `remove_selected` 命令，配套 `RemoveReport` 类型；新增 4 个单元测试覆盖 dotted-path get/set/remove
- `ToolDescriptor` 加 `latest_version` / `stable_version` / `stable_falls_back_to_latest` / `installations` 字段，`list_tools` 用 `tokio::join!` 并发执行 8 个探测任务（每个工具 4 项：installed / latest / stable / 诊断）
- `install_diagnostics::path_starts_with` 加 Windows / Unix 平台条件单元测试

### 贡献者

- [@LS-plan](https://github.com/LS-plan) (ShuyuS) — feature PR [#1](https://github.com/zuoliangyu/ai-cli-installer/pull/1)：安装诊断、修复项配置状态、窗口居中
- [@zuoliangyu](https://github.com/zuoliangyu) — 代码审查 + 后续修正（commit `c5bd0bc`）

## [0.0.11] - 2026-05-07

### 改动

- **npm 安装走自家镜像 (.tgz)**：选「npm」安装方式时，app 不再去 `registry.npmmirror.com` 拉，而是从我们的 mirror release 下载 `.tgz` 用 `npm install --offline` 装
  - 全程走 GH 加速代理链（gh-proxy / fastgit / yylx / chenc / ghproxy.net / ghfast / 直连），与 native 路径同一套机制
  - 每用户只下 2 个 .tgz：主包 (~13KB) + 当前平台子包 (~80MB)，总量跟 native 路径相当
  - SHA256 校验对照 mirror 同步时记录的 `npm-manifest.json`
  - **失败回退**：mirror 拉失败 / 校验失败 / npm 装失败 → 自动 fallback 到 `npm install -g <pkg> --registry npmmirror`（原 v0.0.8 行为）

### 内部

- `Mirror::asset_url(version, asset)` — 通用资产 URL 构造，取代之前只能 `{platform}-{binary}` 模板的限制
- `npm_installer` 加 `install_via_mirror_tarballs(client, mirrors, version, platform)`：抓 `npm-manifest.json` → 下两个 tgz → `npm cache add` 平台包 → `npm install -g <main> --include=optional --prefer-offline`
- 同时支持 Claude Code 的「分包式」打包（每平台独立 npm package）和 Codex 的「版本别名式」打包（单包多版本变种）—— 通过 `NpmManifestEntry::detect_platform()` 智能区分
- 暂存目录 `~/.cache/ai-cli-installer/npm/<version>/` 装完即清

### 镜像仓库改造（v0.0.11 配套）

- `claude-code-mirror` / `codex-mirror` sync workflow 都加 `sync-npm.sh`，每次同步顺手把 npm tarballs 也镜像
- 已同步：claude-code-mirror v2.1.132（18 资产）+ codex-mirror v0.128.0（16 资产）
- 同时移除两个 mirror 仓库的 `cleanup-old-releases` 任务——纯镜像仓应保留所有历史

## [0.0.10] - 2026-05-07

### 改动

- **修复列表改为远程拉取**：app 启动时从 GitHub 直接拉最新 `fixes.json`，编辑 fix 不再需要发新版
  - 候选 URL：raw.githubusercontent.com 直链 → gh-proxy / fastgit / github.chenc.dev 三个加速代理串行尝试
  - 5 秒超时，全部失败时 fallback 到编译时嵌入的版本（保证离线 / 远程全挂时 UI 仍可用）
  - 维护方式：直接编辑 [`src-tauri/fixes.json`](src-tauri/fixes.json) 推到 main，下一次用户启动应用就生效

### 内部

- `fixes::list_fixes` 改为 async + `&reqwest::Client` 参数（复用 AppState 的全局 client）
- `fixes::apply_selected` 同样改 async，应用前先尝试拿远程最新定义，离线时退到嵌入版
- 新增 `fetch_remote` / `parse_embedded` / `list_fixes_embedded` 辅助函数

## [0.0.9] - 2026-05-07

### 新增

- **故障排查 / 配置补丁**：主界面新增「故障排查 / 配置补丁」区块
  - 内置 5 条来自 [OCC 配置文档](https://docs.openclaudecode.cn) 的常见修复
    - **CC-006**：禁用实验性 Beta 参数（解决 AWS 分组 400 invalid beta flag）
    - **CC-017**：跳过 WebFetch 预检（国内网络下让 WebFetch 可用，强烈推荐）
    - **CC-005**：标记 Onboarding 已完成（绕过首次启动连通性校验）
    - 关闭遥测 / 错误上报（隐私）
    - 不附加 `Co-Authored-By` 提交标记
  - 用法：勾选后点「应用」，对应字段自动写入 `~/.claude/settings.json` 或 `~/.claude.json`
  - **保留**配置文件已有的所有其他字段，只改/加选中的键
  - 每条修复显示要写入的目标文件、JSON 路径、值，所见即所得
  - 支持点开 OCC 完整文档详细了解

### 内部

- 新增 `src-tauri/fixes.json`：JSON 数据源，结构化 fix 列表，编译时通过 `include_str!` 嵌入二进制（不依赖网络）
- 新增 `fixes.rs` 模块：`Fix` / `Patch` / `TargetFile` 类型 + dotted-path setter（支持 `env.X.Y` 这样的嵌套路径）+ 文件级合并写入（按 target 分组减少 I/O）
- 新增 2 个 Tauri 命令：`list_fixes` / `apply_fixes`
- 后续加新修复只需编辑 `fixes.json` 即可，新版本发出去就生效

## [0.0.8] - 2026-05-07

### 新增

- **npm 安装路径**：Claude Code 和 Codex 现在都能选择 npm 路径安装
  - Claude Code → `npm install -g @anthropic-ai/claude-code`（需 Node ≥ 18）
  - Codex → `npm install -g @openai/codex`（需 Node ≥ 16）
  - 默认走淘宝 npm 镜像 `https://registry.npmmirror.com`，国内速度起飞
  - **不**永久修改用户的 npm config（registry 通过 `--registry` 参数一次性传入）
  - 工具卡片新增「安装方式」单选条：镜像加速 (推荐) / npm
- **「检测已安装版本」加 PATH 回退**：之前只查 `~/.local/bin/`，现在如果该路径无文件会降级到 PATH 查找，npm 装的版本（在 `npm prefix -g`）也能被识别
- 后端新 `detect_node` Tauri 命令（前端可独立查询 Node 状态）

### 内部

- `tools/spec.rs` 加 `InstallMethod` 枚举（Native / Npm）+ `npm_package` / `npm_min_node` trait 方法
- `InstallReport` 加 `method` 字段，前端能区分本次装的是哪条路径
- `npm_installer.rs` 新模块（120 行）：Node 版本检测 + `npm install -g --registry ...` + `npm prefix -g` 解析全局 bin 目录
- `ToolDescriptor` 加 `supports_npm` / `npm_package` / `npm_min_node` 字段，UI 自动显示对应控件

### 跳过

- v0.0.7 计划过的 Gemini CLI 集成不做了——Gemini 只有 npm 路径，跟现在「专注 Claude Code + Codex」的目标重叠度低

## [0.0.7] - 2026-05-07

### 新增

- **Codex CLI 支持**：新增 OpenAI Codex CLI 工具卡，与 Claude Code 同样的镜像加速安装体验
  - 镜像源：[zuoliangyu/codex-mirror](https://github.com/zuoliangyu/codex-mirror)（每日定时同步 openai/codex 的 GitHub Releases）
  - 6 平台支持（darwin/linux/win32 × x64/arm64）
  - .zst 压缩格式（每平台 ~60 MB，比 .tar.gz 小 30%），下载后客户端 zstd 解压到 `~/.local/bin/codex(.exe)`
  - 共享 7 个 GitHub 加速代理（gh-proxy / fastgit / yylx / chenc / ghproxy.net / ghfast）
- `Tool` trait 加 `mirror_list(&self) -> MirrorList`，每个工具自带镜像源配置（Claude Code → claude-code-mirror，Codex → codex-mirror）
- `MirrorList::builtin_for(repo, with_upstream)` — 参数化镜像列表构造
- `PlatformEntry` 加 `runtime_binary` 字段，支持「下载文件名 ≠ 运行时文件名」（Codex 下载 `codex.zst`，解压后 `codex`）

### 改动

- **发版策略转单仓库**：本仓库 v0.0.7+ 由私有改为公开，源码与发版产物都在本仓库 Releases 下，不再走 `ai-cli-installer-dist` 跨仓库链路
  - `release.yml` 简化：去掉 `DIST_REPO_PAT` secret 依赖，全用内置 `GITHUB_TOKEN`
  - Tauri updater endpoint 改为 `https://github.com/zuoliangyu/ai-cli-installer/releases/latest/download/latest.json`
  - `ai-cli-installer-dist` 仓库保留作为 v0.0.1~v0.0.6 历史归档，不再维护
- v0.0.6 已装用户的 updater 端点指向旧仓库，**收不到 v0.0.7+ 自动更新**，需要手动从本仓库 Releases 重新下载一次

### 内部

- 新增 `tools/codex.rs` 模块，180 行实现完整安装流程
- 新增依赖 `zstd = "0.13"`（用于 .zst 解压）
- `commands.rs` 改造：每个工具调用自己的 `mirror_list()`，`AppState.mirrors` 字段保留作为 UI 镜像状态展示用（默认 Claude Code 的镜像列表）

## [0.0.6] - 2026-05-07

### 新增

- **「关于」弹窗**：footer 点击「v0.0.6 · 关于」按钮打开
  - 作者：左岚（链接到[哔哩哔哩主页](https://space.bilibili.com/27619688)）
  - 项目：[GitHub Releases](https://github.com/zuoliangyu/ai-cli-installer-dist)
  - 应用版本号
  - ESC / 点击外部 / 点击「关闭」按钮关闭
- 引入 `@tauri-apps/plugin-shell`，链接通过 `shell.open()` 走系统默认浏览器打开（不会卡在 WebView 内）
- **中转站快捷配置**：主界面新增「中转站快捷配置」区块
  - 内置预设：
    - **Micu (米醋)** — `https://www.micuapi.ai`（v0.0.6 起的新地址，不再用 cc-switch-web 老仓库的 `openclaudecode.cn`）
    - **E-FlowCode** — `https://e-flowcode.cc`
  - **同步 cc-switch**：自动读 `~/.cc-switch/cc-switch.db` 的 claude providers 表（只读、最佳努力，db 不存在或解析失败静默跳过），与内置预设合并去重（按 base_url 大小写无关）
  - 应用预设：仅写入 `~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL` + `env.ANTHROPIC_AUTH_TOKEN`，**保留**已有的 `effortLevel` / `enabledPlugins` / 其他 `env.*` 字段
  - 主界面显示当前激活的 BASE_URL，匹配的预设卡片标「使用中」徽章
  - v0.0.6 仅支持 Claude（Codex / Gemini 在路上）

### 内部

- 新增 `presets.rs`：Claude 预设抽象 + cc-switch DB 读取 + settings.json 合并写入
- 新增 3 个 Tauri 命令：`list_claude_presets` / `get_claude_settings` / `apply_claude_preset`
- 引入 `rusqlite` 依赖（bundled SQLite，跨平台）

### 注意

`~/.claude/settings.json` 是用户级配置，与 cc-switch 共享。如果你正在用 cc-switch 切换 provider，本应用的「应用预设」会覆盖 cc-switch 写入的 `BASE_URL` / `AUTH_TOKEN`。两者互相可见，无需手动同步——但同时只能有一个生效。

## [0.0.5] - 2026-05-07

### 修复

- **Windows updater 字段对齐 Tauri v2 协议**：v0.0.3/v0.0.4 错误地以为 Windows 需要 `.nsis.zip` 包装，实际上 Tauri 2 的 v2 updater 在 Windows 直接下载 NSIS `.exe` 安装器并用 `.exe.sig` 验签
- 删除「Upload Windows updater bundles」步骤（瞎折腾——Tauri 根本不生成那些 `.zip`，"Looking for artifacts in:" 列出的是预期路径而非实际文件）
- `latest.json` 的 `windows-x86_64` 现在指向 `*_x64-setup.exe` + `.exe.sig`，匹配 Tauri 实际产物

## [0.0.4] - 2026-05-07

### 修复

- v0.0.3 的「Upload Windows updater bundles」步骤用 `shopt -s nullglob` + bash glob，在 Windows git-bash runner 上未匹配到 `.nsis.zip` / `.msi.zip` 文件，导致这些文件没上传、`latest.json` 里 Windows signature 为空、Windows 用户实际上无法自动更新
- 改用 `find` 显式查找，匹配失败时打印 bundle 目录内容并 exit 1（之前是静默通过）

### 影响

- Windows 用户从 v0.0.1/v0.0.2/v0.0.3 任一版本启动 app，updater 应该能正确识别 v0.0.4 并提示更新（v0.0.3 latest.json 的 Windows 字段虽然空，但 GH `releases/latest/download/latest.json` 已被 v0.0.4 覆盖）

## [0.0.3] - 2026-05-07

### 修复

- **Tauri updater 链路打通**：v0.0.1/v0.0.2 都缺 `latest.json`（updater 必需的元数据文件），导致已装用户永远收不到更新提示。本版起每次发版自动生成并上传 `latest.json`
- **Windows updater 包补齐**：`tauri-action` 在 `tagName: ''` 模式下会过滤掉 `.nsis.zip` / `.msi.zip`（Windows updater 实际使用的产物），现在显式额外上传

### 内部

- `release.yml` 新增 `generate-latest` 作业：所有 build 完后下载各平台 `.sig`，用 jq 拼出 `latest.json`，跨仓库上传到 dist release
- `publish` 作业现在依赖 `generate-latest`，确保 latest.json 在 release 公开前到位
- 三平台 url 模式：
  - `windows-x86_64` → `..._x64-setup.nsis.zip`
  - `darwin-aarch64` → `....app.tar.gz`
  - `linux-x86_64`   → `..._amd64.AppImage`

## [0.0.2] - 2026-05-07

### 新增

- **PATH 管理**：工具卡片新增 PATH 状态行，区分「系统 PATH ✓ / 仅用户 PATH / 仅当前会话 / 未在 PATH」四档
  - 一键加入系统 PATH（Windows 弹一次 UAC，提权 PowerShell 写 `HKLM\System\...\Environment\Path` + 广播 `WM_SETTINGCHANGE`）
  - 一键移除（同样需要 UAC）
  - 用户取消 UAC（exit code 1223）会展示明确错误提示
- 三平台共用 `~/.local/bin` 作为 launcher 目录（Claude Code 在 Windows 上也是 Unix 风格布局）
- 新 Tauri 命令：`check_path_status` / `add_to_path` / `remove_from_path`
- Tool trait 加 `launcher_dir(&self)`，为后续多工具准备

### 修复

- **Windows 已安装版本检测**：v0.0.1 错误地查 `%LOCALAPPDATA%\Programs\claude\claude.exe`，实际路径是 `~\.local\bin\claude.exe`，导致 Claude Code 装着也显示「未安装」（不影响新装/更新）
- `release.yml` 的 prepare 步骤加 `--target main`，防止 dist 仓库在「无 main 分支」状态下 publish 失败（HTTP 422 "Repository is empty"）

### 已知限制

- macOS / Linux 的「系统 PATH」（`/etc/profile.d/`）写入需要 sudo，v0.0.2 在这两个平台 fallback 到用户 rc 文件（`~/.bashrc`/`~/.zshrc`/`~/.profile`）。系统级写入留给 v0.0.3
- Windows 用户首次 UAC 弹窗后，必须开新终端窗口才能看到 PATH 生效（已知 Windows 限制，已在 UI 提示）

## [0.0.1] - 2026-05-07

首次发版。

### 新增

- Tauri v2 + Svelte 5（Runes 模式）+ TypeScript 桌面应用骨架
- Rust 后端模块化结构：
  - `upstream` — 读取官方 / 镜像 manifest
  - `mirrors` — 镜像枚举、并发测速、故障切换
  - `downloader` — reqwest 流式下载 + 进度事件回传给前端
  - `verifier` — SHA256 校验
  - `installer` — 调用 binary 的 `install` 子命令完成自举安装
  - `platform` — OS / arch / Linux musl 检测
  - `tools` — Tool trait 抽象，预留 Codex 等扩展点
- **Claude Code 工具支持**：
  - 自动检测已安装版本（`~/.local/bin/claude --version`）
  - 走镜像加速下载二进制
  - 校验 SHA256 与官方 manifest 一致
  - 调用 binary `install latest` / `install stable` 完成自举
  - 支持 latest / stable 双通道
- **镜像加速**：
  - 上游直连 `downloads.claude.ai`
  - 自建 GitHub Release 镜像（`zuoliangyu/claude-code-mirror`，每日同步）
  - 7 个 GitHub 加速代理候选（gh-proxy / fastgit / yylx / chenc / ghproxy.net / ghfast 等）
  - 启动时并发测速、故障自动切换
  - 镜像列表从 `mirrors.json` 动态拉取，无需更新应用即可调整
- **CI/CD**：
  - tag 推送触发 `release.yml`，三平台并行构建（Windows x64 / macOS arm64 / Linux x64）
  - 跨仓库推送：私有源码 → 公开 `ai-cli-installer-dist` Release
  - Tauri 内置 updater 支持，含签名验签
- **同步基础设施**：
  - 配套 `claude-code-mirror` 仓库每日 cron 同步 8 平台二进制
  - 自动校验 SHA256，失败时开 issue
  - 通道指针 `channels/{latest,stable}.txt` 自动更新

### 已知限制

- 暂时仅支持 Claude Code，Codex 在 v0.0.2+ 加入
- 未提供代码签名（Windows SmartScreen / macOS Gatekeeper 首次运行有警告，需手动允许）
- macOS x64（Intel）平台未在 CI 矩阵中，仅 arm64
- 镜像仓库 GitHub UI 上的 "Latest" 标记可能与实际 latest 不符，应用以 `channels/latest.txt` 为准
