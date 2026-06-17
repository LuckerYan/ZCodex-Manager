# ZCode Manager

ZCode Manager 是一个基于 **Tauri 2 + React + Rust + SQLite** 的桌面端 ZCode 账号管理器，用来集中管理多个 ZCode / Z.ai Coding Plan 账号、额度信息和本机凭证，并尽量在不重启 ZCode 主窗口的情况下完成当前账号热切换。

> 项目地址：<https://github.com/LuckerYan/ZCodex-Manager>  
> 如果这个工具帮你省下了反复登录、查额度、切账号的时间，欢迎在 GitHub 点一个 ⭐ Star 支持一下。

## 适用场景

- 你有多个 ZCode 账号，需要统一查看套餐、到期时间和模型剩余额度。
- 你希望通过浏览器 OAuth 添加账号，而不是手动复制 token。
- 你希望把当前本机 ZCode 正在使用的账号一键导入到管理器。
- 你希望切换账号时尽量不关闭 ZCode 主窗口，只重启必要的 agent 子进程。
- 你希望账号 token 在本机落盘时以加密形式保存，而不是直接明文堆在配置文件里。

## 已实现能力

- **SQLite 账号池**：账号数据保存在用户数据目录下的 `zcode-manager/zcode-manager.db`。
- **本机凭证加密**：核心 token 字段写入 SQLite 前使用 AES-256-GCM `enc:v1` 格式加密。
- **浏览器 OAuth 添加账号**：调用 ZCode CLI OAuth 流程，打开系统浏览器授权，轮询 `ready` 后保存账号 token。
- **导入当前 ZCode 账号**：读取 `%USERPROFILE%\.zcode\v2\credentials.json`，解密 `oauth:zai:access_token`、`zcodejwttoken`、`oauth:zai:user_info`，并同步 `config.json` provider 与 `coding-plan-cache.json` 作为热切换快照。
- **套餐 / 模型额度查询**：使用 `zcode_jwt_token` 请求 `/zcode-plan/billing/current` 与 `/zcode-plan/billing/balance`，在账号列表中展示套餐、到期时间、GLM 模型额度和刷新状态。
- **账号热切换**：写入 ZCode credentials/config/cache，精准查找命令行包含 `zcode.cjs app-server --stdio` 的 agent 子进程并结束，保留 ZCode 主窗口。
- **运行状态监控**：展示本机 ZCode 进程、凭证文件、配置文件、缓存文件、CDP 调试端口等状态。
- **批量操作**：支持账号勾选、批量刷新、批量导出、批量删除与 JSON 导入。

## 页面说明

- **仪表盘**：查看账号总数、可热切换账号、额度概览和近期用量概况。
- **账号管理**：管理账号列表、查看账号明细、刷新额度、导入/导出账号、执行热切换。
- **设置**：配置自动切换策略、额度阈值、候选账号过滤与自动检测间隔。
- **关于**：查看项目简介、技术栈、架构说明和 GitHub 仓库入口。

## 开发环境

建议使用 Windows + PowerShell 环境：

- Node.js / pnpm
- Rust stable toolchain
- Tauri 2 所需系统依赖

首次安装依赖：

```powershell
pnpm install
```

## 开发命令

启动 Vite + Tauri 热更新开发窗口：

```powershell
pnpm tauri dev
```

只构建前端：

```powershell
pnpm build
```

构建桌面安装包：

```powershell
pnpm tauri build
```

## 构建产物

常见产物路径如下：

```text
src-tauri/target/release/zcode-manager.exe
src-tauri/target/release/bundle/msi/zcode-manager_0.1.0_x64_en-US.msi
src-tauri/target/release/bundle/nsis/zcode-manager_0.1.0_x64-setup.exe
```

## 使用提示

- 「打开浏览器 Auth」保存的账号默认只有 token，可以查询额度，但如果缺少 provider/cache 快照，可能无法直接热切换。
- 「导入当前 ZCode 账号」会同步当前 ZCode 的 provider/cache，通常会标记为可热切换。
- 热切换不会关闭 ZCode 主窗口，只会重启匹配到的 agent 子进程；如果 ZCode UI 有缓存，界面上的用户名或套餐可能需要刷新或发起下一次请求后才更新。
- 批量导出的 JSON 可能包含明文 token，仅建议在可信环境中临时迁移使用，不要公开分享。
- 如果热切换失败，优先查看「ZCode 状态」或「账号管理」中的凭证文件、配置文件、CDP 端口和 agent 进程状态。

## 数据与安全边界

- SQLite 数据库默认位于当前用户数据目录，不会上传到远端。
- Token 字段会加密后落盘，但导出账号时为了跨机迁移可能生成明文 JSON，请自行妥善保存。
- 项目会读写本机 `%USERPROFILE%\.zcode\v2` 下的 ZCode 凭证、配置和缓存文件；热切换前建议确保当前 ZCode 没有重要未保存状态。

## 技术栈

- **桌面框架**：Tauri 2
- **前端**：React 19、TypeScript、Vite
- **后端**：Rust
- **数据库**：SQLite / rusqlite
- **加密**：AES-256-GCM、SHA-256、base64url
- **HTTP / JSON**：reqwest、serde、serde_json
- **运行时控制**：Windows 进程查询、CDP WebSocket

## 贡献与支持

欢迎提交 Issue、建议和 PR：

- GitHub：<https://github.com/LuckerYan/ZCodex-Manager>
- 如果你觉得这个项目有用，欢迎给仓库点一个 ⭐ Star，让更多 ZCode 用户能找到它。
