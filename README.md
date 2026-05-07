# AI CLI Installer

Tauri + Svelte 桌面应用，为 Claude Code 提供镜像加速下载与一键安装。

## 架构

```
官方 downloads.claude.ai
       │
       │ (claude-code-mirror 仓库每日定时同步)
       ▼
GH Release (zuoliangyu/claude-code-mirror)
       │
       │ (本应用启动时拉 manifest，多镜像并发下载)
       ▼
用户机器 → Claude Code 已装
```

应用本体由 **私有源码仓库 (本仓库)** 编译，产物推送到公开的 `ai-cli-installer-dist` 仓库 Release，供终端用户下载。

## 目录

```
src/                    Svelte + TypeScript 前端
  lib/
    api.ts              Tauri invoke 封装
    stores.ts           Svelte stores
    types.ts            前后端共享类型定义
    components/         UI 组件

src-tauri/              Rust 后端
  src/
    lib.rs              Tauri builder + 插件注册
    commands.rs         #[tauri::command] 暴露给前端
    upstream.rs         /latest /stable /manifest.json 抓取
    mirrors.rs          镜像枚举 + 并发测速 + 故障切换
    downloader.rs       reqwest 流式下载 + 进度事件
    verifier.rs         SHA256 校验
    installer.rs        调用 binary 的 install 子命令
    platform.rs         OS/arch + Linux musl 检测
    tools/              每个被管工具一模块
      spec.rs           Tool trait
      claude_code.rs    Claude Code 实现

.github/workflows/
  release.yml           tag 推送时多平台构建并跨仓库发版
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

```sh
npm install
npm run tauri dev
```

首次启动会编译 Rust 依赖（几分钟），之后增量编译很快。前端改动热更新。

## 构建

```sh
npm run tauri build
```

产物在 `src-tauri/target/release/bundle/`。

## 图标

`src-tauri/icons/` 当前是占位，构建会失败。准备一张 1024×1024 的 PNG 后跑：

```sh
cargo tauri icon path/to/logo.png
```

## 发版（CI）

推 `v*` 标签触发 `release.yml`：

```sh
git tag v0.2.0
git push --tags
```

CI 流程：
1. `prepare`: 在 `ai-cli-installer-dist` 创建 draft release
2. `build`: 三平台并行构建（Linux/Windows/macOS arm64），用 `gh release upload --clobber` 跨仓库推送产物
3. `publish`: 取消 draft 标志、设为 latest

需要的 GitHub Secrets（在本仓库 Settings → Secrets）：

| Secret | 用途 |
|--|--|
| `DIST_REPO_PAT` | fine-grained PAT，授权范围限 `ai-cli-installer-dist` 仓库的 Contents=Write |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater 签名私钥（base64） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码（生成时设置的） |

生成 updater 签名密钥：

```sh
cargo tauri signer generate -w ~/.tauri/myapp.key
```

公钥写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，私钥内容（含 BEGIN/END 包裹的整段）作为 secret。

## 镜像列表

应用启动时优先从 `claude-code-mirror` 仓库的 `mirrors.json` 拉取镜像列表，拉不到时回退到代码内置的兜底列表（见 `mirrors.rs::MirrorList::builtin`）。

修改镜像策略不需要发版，编辑 `claude-code-mirror/mirrors.json` 推 main 即可生效。

## 代码风格

- Rust：默认无注释，仅在非显然约束 / 易踩坑处加一行
- 错误：所有 `Result` 走 `error::AppError`，`#[tauri::command]` 直接返回 `Result<T, AppError>`，序列化为字符串给前端
- 前端：Svelte 5 Runes 模式（`$state` / `$derived` / `$props`）
- 跨语言类型：手写在 `src/lib/types.ts`，与 Rust struct 对齐（v0.2 不引入自动生成）
