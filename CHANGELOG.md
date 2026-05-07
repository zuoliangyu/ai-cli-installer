# Changelog

本项目所有重要变更记录。遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式，版本号遵循 [SemVer](https://semver.org/lang/zh-CN/spec/v2.0.0.html)。

## [Unreleased]

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
