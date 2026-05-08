# AI CLI Installer

为 Claude Code / Codex CLI 提供镜像加速下载与一键安装的桌面应用，同时也能作为本地 / 远程 Web 服务运行。Tauri + Svelte 5 + Rust。

## 架构

v0.1.0 起拆为 **Cargo workspace（3 crate）+ 双前端 API 层**，桌面端和 Web 端共享同一份核心逻辑。

```
官方 downloads.claude.ai
       │ (claude-code-mirror / codex-mirror 仓库每日定时同步)
       ▼
GH Release 镜像 + 7 个加速代理
       │
       ▼
┌──────────────────────────────────────────┐
│ installer-core（Cargo crate · 纯逻辑）     │
│   mirrors / downloader / verifier        │
│   tools / npm_installer / fixes / ...     │
└──────────┬───────────────────────────────┘
           │
   ┌───────┴───────┐
   ▼               ▼
src-tauri        installer-web
（桌面壳）         （Axum + WS + rust-embed）
   │               │
   │ invoke IPC    │ HTTP + WebSocket
   ▼               ▼
┌──────────────────────────────────────────┐
│ 前端（Svelte + TS）                        │
│   src/lib/api.ts ← 编译时 __IS_TAURI__ 派发 │
│        ├─ services/tauriApi.ts            │
│        └─ services/webApi.ts              │
└──────────────────────────────────────────┘
```

应用源码与发版产物都在本仓库（v0.0.7+ 起改为单仓库公开模式；v0.0.1~v0.0.6 历史归档在 [`ai-cli-installer-dist`](https://github.com/zuoliangyu/ai-cli-installer-dist)）。

## 目录

```
src/                          Svelte + TypeScript 前端
  lib/
    api.ts                    统一入口（按 __IS_TAURI__ 动态 import）
    stores.ts                 Svelte stores
    types.ts                  前后端共享类型定义
    services/
      tauriApi.ts             Tauri 模式：invoke + listen
      webApi.ts               Web 模式：fetch + WebSocket
    components/               UI 组件

crates/
  installer-core/             共享核心库（无 Tauri 依赖）
    src/
      app_state.rs            高层服务 API（list_tools / install_tool / ...）
      progress.rs             ProgressCallback 抽象
      mirrors.rs              镜像枚举 + 并发测速 + 故障切换
      downloader.rs           reqwest 流式下载 + 进度回调
      verifier.rs             SHA256 校验
      installer.rs            调用 binary 的 install 子命令
      npm_installer.rs        npm 路径安装（mirror tarball + 在线 fallback）
      platform.rs             OS / arch / Linux musl 检测
      upstream.rs             /latest /stable /manifest.json 抓取
      fixes.rs                配置补丁（远程 fixes.json + 本地 embedded）
      presets.rs              Claude 中转站预设 + cc-switch 同步
      install_diagnostics.rs  多源安装诊断
      env_manager/            跨平台 PATH 管理
      tools/                  Tool trait + Claude Code / Codex 实现
    fixes.json                故障排查补丁数据源（编译时嵌入 + 远程同源）

  installer-web/              Axum HTTP + WebSocket 服务
    src/
      main.rs                 路由组装
      routes.rs               /api/* 处理函数
      ws.rs                   /ws/progress（DownloadProgress 广播）
      static_files.rs         rust-embed 嵌入 dist/
      config.rs               --host / --port / 环境变量

src-tauri/                    Tauri 桌面壳
  src/
    lib.rs                    Tauri builder + 插件注册 + 窗口初始化
    commands.rs               #[tauri::command] 薄壳，转发到 installer-core
  tauri.conf.json             桌面包配置 + updater endpoint
  capabilities/               Tauri v2 能力声明

.github/workflows/
  release.yml                 tag 推送时多平台构建并发版
```

## 开发环境

需要：

- Rust 1.77+ (`rustup install stable`)
- Node 20+ / npm
- Tauri CLI: `cargo install tauri-cli --locked --version "^2.0"`
- 各平台原生依赖：参考 https://v2.tauri.app/start/prerequisites/

Windows 还需要：VS Build Tools 2022（C++ 桌面开发） + WebView2（Win11 自带）。

Linux 需要：`libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`。

## 本地运行

### 桌面模式（Tauri）

```sh
npm install
npm run tauri dev
```

首次启动会编译 Rust 依赖（几分钟），之后增量编译很快。前端改动热更新。

### Web 模式（Axum 服务器）

```sh
npm install
npm run build:web    # 输出 dist/，installer-web 通过 rust-embed 嵌入
npm run web          # 等价于 cargo run -p installer-web
```

默认监听 `http://127.0.0.1:3210`。可通过 `--host` / `--port` 或环境变量 `INSTALLER_HOST` / `INSTALLER_PORT` 调整。

> 注意：Web 模式改的是**运行 installer-web 的那台机器**的环境（`~/.local/bin`、PATH、`~/.claude/settings.json`）。容器化对该应用没有意义，因此不提供 Docker 镜像。

## 构建

### 桌面包

```sh
npm run tauri build
```

产物在 `target/release/bundle/`（Cargo workspace 输出在仓库根 `target/`）。

### Web 服务器二进制

```sh
npm run build:web && cargo build -p installer-web --release
```

产物：`target/release/installer-web` (Linux/macOS) 或 `target/release/installer-web.exe` (Windows)。整套是单文件，前端通过 `rust-embed` 编译期嵌入。

## 命令速查

```sh
# 桌面端开发 / 构建
npm run tauri dev
npm run tauri build

# Web 端开发 / 运行
npm run dev              # vite dev（独立前端，用于调样式）
npm run build:web        # 产出 dist/ 给 installer-web 嵌入
npm run web              # cargo run -p installer-web

# 类型 / 编译检查
npm run check            # svelte-check
cargo check --workspace  # 三 crate 全检查
cargo clippy --workspace -- -D warnings
```

## 图标

`src-tauri/icons/` 当前是占位，构建会失败。准备一张 1024×1024 的 PNG 后跑：

```sh
cargo tauri icon path/to/logo.png
```

## 发版（CI）

推 `v*` 标签触发 `release.yml`：

```sh
git tag v0.2.3
git push --tags
```

CI 流程：
1. `prepare`: 在本仓库创建 draft release（如果不存在），从 CHANGELOG 抽对应版本段落作为 release notes
2. `build`: 三平台并行构建（Linux x64 / Windows x64 / macOS arm64），上传 bundle 到 release
3. `generate-latest`: 下载各平台 `.sig`，用 jq 拼出 `latest.json` 上传
4. `publish`: 取消 draft 标志、设为 latest

需要的 GitHub Secrets：

| Secret | 用途 |
|--|--|
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater 签名私钥（base64） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码（生成时设置的） |

生成 updater 签名密钥：

```sh
cargo tauri signer generate -w ~/.tauri/myapp.key
```

公钥写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，私钥内容（含 BEGIN/END 包裹的整段）作为 secret。

## 镜像列表

应用启动时优先从 `claude-code-mirror` / `codex-mirror` 仓库的 `mirrors.json` 拉取镜像列表，拉不到时回退到 `installer-core/src/mirrors.rs` 内置兜底。

修改镜像策略不需要发版，编辑对应 mirror 仓库的 `mirrors.json` 推 main 即可生效。

## 代码风格

- Rust：默认无注释，仅在非显然约束 / 易踩坑处加一行；错误统一走 `installer_core::error::AppError`
- 前端：Svelte 5 Runes 模式（`$state` / `$derived` / `$props`）+ Tailwind CSS（Cerulean Flow 主题，与 [AI Session Viewer](https://github.com/zuoliangyu/AI-Session-Viewer) 同款配色）
- 跨语言类型：手写在 `src/lib/types.ts`，与 Rust struct 对齐（不引入自动生成）
- Web 模式与 Tauri 模式的 API 形状必须保持一致，新加 command 时同时改 `tauriApi.ts` + `webApi.ts` + `api.ts` 的 bind 列表 + `installer-web/src/routes.rs`

## 贡献者

- [@zuoliangyu](https://github.com/zuoliangyu)（左岚）— 项目作者及主要维护者
- [@LS-plan](https://github.com/LS-plan)（ShuyuS）— v0.0.12：安装诊断 + 修复项配置状态 + 启动窗口居中（PR [#1](https://github.com/zuoliangyu/ai-cli-installer/pull/1)）

完整变更与每版贡献者列表见 [CHANGELOG.md](CHANGELOG.md)。
