# AI CLI Installer

为 Claude Code / Codex CLI 提供镜像加速下载与一键安装的桌面应用，同时也能作为本地 / 远程 Web 服务运行。Tauri + Svelte 5 + Rust。

当前版本：**v0.6.0**

> v0.6.0 起，Web 模式默认启用访问令牌与 Host / Origin 校验，安装二进制改为原子替换，远程修复定义只能修改内置白名单内的配置项。macOS 桌面版最低需要 13.3。

> CLI 安装与安装诊断在共享后端串行执行。连续安装多个工具时，后续任务会等待当前操作完成，避免 Windows 下并发争用下载文件或可执行文件导致 `os error 32`；桌面端与 Web 端行为一致。

同一用户的桌面端与 Web 服务通过 `~/.ai-cli-installer/operations.lock` 协调安装和诊断。锁等待最多 30 秒，超时提示稍后重试；刷新工具列表时优先返回已有缓存。锁文件保持存在，由操作系统在进程退出或文件句柄关闭时释放锁；取消等待不会提前释放仍在执行的阻塞部署所持有的锁。

安装任务由后端持有，刷新页面或暂时断网不会取消安装。前端每秒查询任务快照（隐藏页面每 5 秒），同一工具的活动任务会复用，状态同步失败时暂停新安装，恢复连接后继续显示结果。任务记录保留最近 20 条完成记录，仅在当前后端进程内有效；后端重启后会提示检查安装结果，不会自动重放旧任务。

> Claude Code 走「镜像加速」时，若官方 bootstrap 的自我安装（`claude install`）因为回连 `downloads.claude.ai` 失败（国内网络常见），会自动 fallback 到直接把已校验的二进制部署到 `~/.local/bin/claude.exe`——保证装得上。

## 架构

**Cargo workspace（3 crate）+ 双前端 API 层**，桌面端和 Web 端共享同一份核心逻辑。

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

- Rust 1.90+ (`rustup install stable`)，三个 crate 共享 workspace 的最低版本声明
- Node 20.19+（20.x）、22.12+（22.x）或 24+ / npm；推荐使用受支持的 LTS 版本
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

桌面窗口会保存尺寸、位置和最大化状态，并在下次启动时恢复；原显示器断开后由系统重新安排窗口位置。

### Web 模式（Axum 服务器）

```sh
npm install
npm run build:web    # 输出 dist/，installer-web 通过 rust-embed 嵌入
npm run web          # 等价于 cargo run -p installer-web
```

默认监听 `http://127.0.0.1:3210`。可通过 `--host` / `--port` 或环境变量 `INSTALLER_HOST` / `INSTALLER_PORT` 调整。

未设置 `INSTALLER_TOKEN` 时，服务启动会自动生成随机令牌，并在终端打印 `http://127.0.0.1:3210/?token=…` 链接，用该链接打开即可。服务只接受 Host 为 `127.0.0.1` / `localhost` / `[::1]` 的请求，并拒绝跨源的写操作和 WebSocket 连接，以防御 DNS rebinding。

### 远程 Web 访问

Web API 新增 `POST /api/install-tasks`（参数与原安装接口相同，立即返回带 `task_id` 的任务）和 `GET /api/install-tasks`（返回 `session_id` 与任务快照）。桌面端对应 `start_install_task` / `list_install_tasks`。旧安装接口和进度 WebSocket 保留兼容，新前端以任务快照关联进度与最终结果。

绑定非本机地址时必须设置至少 16 位访问令牌，否则服务会拒绝启动：

```powershell
$env:INSTALLER_HOST="0.0.0.0"
$env:INSTALLER_TOKEN="请替换为至少16位的随机令牌"
$env:INSTALLER_ALLOWED_HOSTS="192.168.1.10,installer.example.com"
npm run web
```

监听 `0.0.0.0` / `::` 时，浏览器地址栏里使用的 IP 或域名必须通过 `--allowed-host` 或 `INSTALLER_ALLOWED_HOSTS`（逗号分隔，可不带端口）加入白名单，否则请求会被 403 拒绝。

浏览器使用 `http://服务器地址:3210/?token=访问令牌` 首次打开。页面会把令牌保存到当前标签页的 `sessionStorage`，并立即从地址栏移除；API 使用 Bearer 鉴权，WebSocket 使用同一令牌。公网使用时仍应放在 HTTPS 反向代理或 SSH 隧道后，避免明文 HTTP 泄露令牌和 API Key。

> 注意：Web 模式改的是**运行 installer-web 的那台机器**的环境（`~/.local/bin`、PATH、`~/.claude/settings.json`）。在 Docker 中运行时，安装和配置只作用于容器内部，适合隔离验证，不会替宿主机安装 CLI。项目提供[Docker 测试环境](tests/docker/README.md)，不发布生产容器镜像。

配置修复和 Unix PATH 写入前会保留递增备份：首次为 `.bak`，后续为 `.bak.1`、`.bak.2` 等，只保留最近 5 份；内容没有变化时不写入也不备份。同名配置字段会覆盖，其他字段保持不变。

Unix PATH 按当前 shell 选择配置文件。fish 使用 `$XDG_CONFIG_HOME/fish/conf.d/ai-cli-installer.fish`，未设置或使用相对 `XDG_CONFIG_HOME` 时回退到 `~/.config/fish/conf.d/ai-cli-installer.fish`。重新添加会原位更新完整的管理块并保留其它内容；残缺管理块会提示错误。修改后新开终端生效。

下载暂存文件位于 `~/.cache/ai-cli-installer/staging/` 下的任务专属目录。安装完成、失败或取消后自动清理，正在进行的阻塞复制/解压会先完成再释放目录；历史版本遗留的缓存不自动删除。

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
npm run test:frontend    # 安装任务恢复与状态同步的轻量回归测试
cargo check --workspace  # 三 crate 全检查
cargo clippy --workspace -- -D warnings
```

## Docker Linux 验证

在 Docker Desktop 的 Linux 容器模式下，使用 PowerShell 7.2+ 从仓库根目录运行：

```powershell
./tests/docker/run.ps1
```

该命令会构建测试环境，运行 Linux core/Web 测试和前端检查，实际验证 Claude Code、Codex 的 Native/npm 安装，并使用 Chromium 检查 Web 安装、刷新恢复和故障提示。首次需要联网下载依赖和 CLI；所有安装使用容器自己的用户目录，不挂载宿主机 CLI 配置或凭据。

日志、JSON 结果和截图保存在 `target/docker-validation/<运行编号>/`。分阶段运行和验证边界见[测试说明](tests/docker/README.md)。

## 代码风格

- Rust：默认无注释，仅在非显然约束 / 易踩坑处加一行；错误统一走 `installer_core::error::AppError`
- 前端：Svelte 5 Runes 模式（`$state` / `$derived` / `$props`）+ Tailwind CSS（Cerulean Flow 主题，与 [AI Session Viewer](https://github.com/zuoliangyu/AI-Session-Viewer) 同款配色）
- 跨语言类型：手写在 `src/lib/types.ts`，与 Rust struct 对齐（不引入自动生成）
- Web 模式与 Tauri 模式的 API 形状必须保持一致，新加 command 时同时改 `tauriApi.ts` + `webApi.ts` + `api.ts` 的 bind 列表 + `installer-web/src/routes.rs`

## 贡献者

- [@zuoliangyu](https://github.com/zuoliangyu)（左岚）— 项目作者及主要维护者
- [@LS-plan](https://github.com/LS-plan)（ShuyuS）
  - v0.0.12：安装诊断 + 修复项配置状态 + 启动窗口居中（PR [#1](https://github.com/zuoliangyu/ai-cli-installer/pull/1)）
  - v0.2.4：修复列表扩到 33 条 + 远程缓存 + 标签筛选 + 分页（PR [#2](https://github.com/zuoliangyu/ai-cli-installer/pull/2)）
  - v0.4.0：开发者选项 + 日志查看器（PR [#3](https://github.com/zuoliangyu/ai-cli-installer/pull/3)）

完整变更与每版贡献者列表见 [CHANGELOG.md](CHANGELOG.md)。
