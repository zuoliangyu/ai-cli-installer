# Changelog

本项目所有重要变更记录。遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式，版本号遵循 [SemVer](https://semver.org/lang/zh-CN/spec/v2.0.0.html)。

## [Unreleased]

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
