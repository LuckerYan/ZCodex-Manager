# ZCode Manager

Rust + Tauri 桌面账号管理器，用于管理 ZCode 账号池、浏览器 OAuth、当前 ZCode 凭据导入、套餐/模型额度查询，以及不重启 ZCode 主窗口的 agent 热切换。

## 已实现能力

- SQLite 账号池：数据库位于用户数据目录 `zcode-manager/zcode-manager.db`。
- 从当前 ZCode 导入：读取 `%USERPROFILE%\.zcode\v2\credentials.json`，解密 `oauth:zai:access_token` / `zcodejwttoken` / `oauth:zai:user_info`，并抓取 `config.json` provider 和 `coding-plan-cache.json` 作为热切换快照。
- 浏览器 OAuth：调用 ZCode CLI OAuth 设备流，打开系统浏览器授权，轮询 `ready` 后保存 token。
- 套餐和模型额度：通过 `zcode_jwt_token` 调用 `/zcode-plan/billing/current` 与 `/zcode-plan/billing/balance`，账号列表中展示套餐、到期时间、GLM 模型额度。
- 热切换：写入 ZCode credentials/config/cache，精确查找命令行包含 `zcode.cjs app-server --stdio` 的 agent 子进程并结束，保留 ZCode 主窗口。

## 开发命令

```powershell
pnpm install
pnpm tauri dev
pnpm build
pnpm tauri build
```

## 构建产物

```text
src-tauri/target/release/zcode-manager.exe
src-tauri/target/release/bundle/msi/zcode-manager_0.1.0_x64_en-US.msi
src-tauri/target/release/bundle/nsis/zcode-manager_0.1.0_x64-setup.exe
```

## 使用提示

- 「打开浏览器 Auth」保存的账号默认只有 token，可查额度，但缺少 provider/cache，因此不一定能热切换。
- 「导入当前 ZCode 账号」会同时抓取 provider/cache，通常会标记为可热切换。
- 热切换不会关闭 ZCode 主窗口，只重启 agent 子进程；若 ZCode UI 有缓存，界面上的用户名/套餐可能需要手动刷新或发起下一次请求后更新。
- Token 存入 SQLite 前使用与 ZCode credentials 兼容的 AES-256-GCM `enc:v1` 加密格式保存。
