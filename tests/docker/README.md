# Docker Linux 验证

这个环境在独立的 Debian Linux 容器内，以普通用户 `tester`（UID 1000）运行前端检查、测试和 Web 构建，以及 `installer-core` / `installer-web` 的 Rust 测试和 Web 后端编译。它不验证 Windows UAC、注册表、Job Object 或 macOS、Tauri 桌面窗口行为。

已完成的验证与原始产物索引见[2026-11-15 测试报告](RESULTS-2026-11-15.md)。

从仓库根目录构建：

```powershell
docker build --progress=plain -f tests/docker/Dockerfile -t ai-cli-installer-validation:local .
```

构建镜像需要联网下载依赖，并使用 Docker BuildKit 缓存复用 npm、Cargo 包和编译产物。Dockerfile 只复制列出的项目输入，不复制宿主机的 `node_modules`、根目录 `target`、`.git` 或用户配置。构建过程不运行真实工具安装。

镜像的工作目录是 `/app`，Web 可执行文件位于 `/app/target/debug/installer-web`，配套前端位于 `/app/dist`。镜像包含 Node/npm、bash、fish、zsh 和 curl。默认入口是 `bash`，方便通过只读挂载的测试脚本在一次性容器中运行后续安装和 Web 验证。真实安装测试必须使用容器内的独立 HOME/npm prefix，不挂载宿主机凭据或 Docker socket。

```powershell
docker run --rm ai-cli-installer-validation:local /app/target/debug/installer-web --help
```

构建步骤会运行：

- `npm ci --no-audit --no-fund`
- `npm run check`
- `npm run test:frontend`
- `npm run build:web`
- `cargo test -p installer-core -p installer-web --locked`
- `cargo build -p installer-web --locked`

这里的 `--no-audit` 仅避免在依赖安装步骤重复联网审计，不修改依赖或漏洞报告；它不能替代单独的安全审计。

## 一键运行

使用 PowerShell 7.2+，在仓库根目录执行（Windows PowerShell 5.1 对原生命令 stderr 的处理不同，不支持此入口）：

```powershell
./tests/docker/run.ps1
```

首次需要网络下载依赖和真实 CLI，预计 15–30 分钟及数 GB 缓存。运行结束或失败后只移除本次创建的容器；测试镜像和 BuildKit 缓存保留以便复用，不清理其他项目的资源。测试服务不映射宿主机端口，浏览器通过共享测试服务的容器网络访问它。

支持分阶段运行：

```powershell
./tests/docker/run.ps1 -Stage Build        # 编译和自动化测试
./tests/docker/run.ps1 -Stage Integration  # 使用已构建镜像，运行 PATH 与真实安装
./tests/docker/run.ps1 -Stage Browser      # 使用已构建服务镜像，构建浏览器镜像并运行 Web 场景
./tests/docker/run.ps1 -Stage Integration -InstallCase codex-cli-native  # 只重跑指定安装路线
```

`Integration` / `Browser` 复用现有镜像，并校验构建输入的 SHA-256 指纹。源码变化或镜像由手动命令构建、没有指纹时，入口会拒绝运行；先执行 `Build` 或完整流程即可。日志同时记录源码指纹和实际镜像 ID，避免把旧程序的结果记到新代码上。浏览器使用配套的 `playwright@1.58.2` 和官方 Chromium 镜像，不向项目的前端依赖添加测试包。

产物在 `target/docker-validation/<运行编号>/`，包括运行提交信息、构建日志、`integration.json`、各场景服务日志、浏览器 `results.json` 和截图。任何场景失败，入口脚本会以失败退出；网络下载失败也如实记录，不用模拟结果冒充真实安装成功。

## 覆盖范围

- `integration.mjs` 以非 root 用户执行，每场景独立 `HOME`、npm prefix 和缓存：Claude Code / Codex × Native / npm 四条真实安装路线，校验实际 `--version`、安装诊断、任务去重、失败后再次安装、暂存目录清理，以及 Codex Native 相同版本重复安装。
- bash、zsh、fish：用含空格、引号和 `$` 的用户目录，验证 PATH 添加/移除、重复添加不写文件、备份与原权限保留、新 shell 实际找到命令、残缺配置不被覆盖。
- `browser.mjs` 的 `real-backend-real-install` 场景：真实点击 Codex Native 安装，双击只提交一次，刷新仍对应同一任务，安装结束显示结果，等待版本及安装来源诊断更新，并与 `/api/tools` 核对；页面切换后保留正确结果。
- `simulated-task-api` 场景：仅模拟任务 API 和页面数据，验证下载失败提示、自动模式重试、轮询断线时禁用安装、恢复连接、刷新恢复活动任务和后端会话重启提示。这些场景不代表真实网络故障注入，也不会发起真实安装。

这套测试验证 Linux x64 用户态逻辑和 Web 页面。未覆盖 Windows UAC/注册表/Job Object、macOS、Tauri 原生窗口、Linux ARM64/musl、账号登录与模型调用。镜像使用 Rust 1.96.1；最低 Rust 1.90 由已有 CI 独立检查。
