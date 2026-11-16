---
trigger: always_on
description: Rust 桌面软件项目的通用工程约定
---

# Rust 桌面软件工程约定

适用于 `c:\Dev\rust-rewrite` 工作区下的 Rust 桌面软件项目。本规则只约束工程方式，不约束具体业务功能。

## 项目组织

- 每个软件使用以项目命名的独立子目录，不把项目文件铺在工作区根目录。
- 默认桌面方案：Rust + Tauri 2 + 原生 HTML/CSS/JavaScript 零构建前端。
- 默认面向 Windows 便携分发。
- 工作区为 Cargo workspace：根 `Cargo.toml` 聚合成员、集中依赖版本（`[workspace.dependencies]`）、共享 `target/` 与 release profile。
- 跨项目公共能力放入 `crates/portable-kit`，各 app 以 `portable-kit.workspace = true`（path 依赖）引用，避免重复造轮子；改公共功能只改 crate 一处。
- 统一代码风格：根 `rustfmt.toml`（edition 2021、max_width 100），提交前 `cargo fmt --all`。

## Rust 工具链

- 固定使用 `stable-x86_64-pc-windows-msvc`。
- 通过 `rustup run stable-x86_64-pc-windows-msvc cargo ...` 或当前会话 `RUSTUP_TOOLCHAIN` 指定。
- 不修改全局 `rustup default`。
- 遇到 `dlltool.exe` 缺失时，优先检查是否误用了 GNU 工具链。

## 便携数据隔离

- 任务数据、配置、缓存、日志均放在 exe 同级 `data` 子目录。
- 不回退到 `%APPDATA%`、`%LOCALAPPDATA%` 等系统应用目录。
- Tauri 配置必须使用 `app.appDirectoriesOverride` 覆盖 `config`、`data`、`localData`、`cache`、`log`。
- WebView2 的 `localData` 指向 `data/cache/webview2`。
- 仅设置 `WEBVIEW2_USER_DATA_FOLDER` 不足以阻止 Tauri 创建默认系统目录。
- debug 与 release 因 exe 位置不同而天然隔离。
- 数据隔离实现统一复用 `portable-kit::portable`（`portable_data_dir` / `configure_webview_data_dir`），不在各 app 重复实现。

## 运行日志

- 每个 app 统一复用 `portable-kit::logging`：启动时 `init(app_name)` + `install_panic_hook()`，日志写入 exe 同级 `data/logs/<app>-<YYYY-MM-DD>.log`。
- 日志行格式 `[RFC3339 时间] [级别] [应用名] 消息`；关键操作（启动/更新检查/启动程序/扫描/设置/增删改/发布）与错误均需记录，panic 记录位置与背栈，便于定位问题。
- 常驻管理器（launch-manager）每小时调用 `logging::cleanup_old_logs` 清理自身与被管理应用超过 7 天的日志。

## 前端统一与复用

- 共享前端层 `shared/ui/{base.css,base.js}`：统一设计令牌（家族调色板+旧名别名）、重置、滚动条、toast，以及 `window.PKUI` 工具库（invoke/listen/$/errorText/toast/applyTheme/systemDark/requireInvoke）。
- 各 app `index.html` 先引 `shared/base.css` 与 `shared/base.js`；`style.css` 仅保留自身布局与语义令牌（如 --todo-bg/--code-bg）；`main.js` 复用 PKUI，不重复实现工具函数。
- `scripts/sync-ui.mjs` 将 `shared/ui` 同步到各 app `src/shared`（零构建前端需物理存在）；打包脚本自动调用。改公共前端只改 `shared/ui` 一处。

## 数据备份与迁移

- WebDAV 云备份统一由常驻管理器调度，引擎复用 `portable-kit::webdav`（blocking reqwest + Basic 认证）；凭据只存管理器配置，各 app 不各自接 WebDAV。
- 远端布局 `<root>/<app名>/…` + `manifest.json`（sha/mtime）做增量上传；把每个 app 的 `data/` 当不透明文件镜像，排除 cache/update/logs；**管理器自身配置也纳入备份/导入导出**（扫描器排除自身，需单独加入）。
- 触发：每小时定时 + 管理器退出时 + 设置页“立即备份”；恢复前先把本地现有文件备份到 `data/restore-backup-<ts>/`。
- 导入/导出复用 `portable-kit::dataio`（JSON 信封 `{format,version,app,exportedAt,files}`）；导入校验 app 名防跨 app 误导入，写入前自动备份。
- 离线优先：本地为主存储，WebDAV/导入导出只做备份/恢复，不做实时多端合并。

## 构建与发布

- 默认只分发便携 ZIP，不生成 NSIS 安装包；`tauri.conf.json` 中 `bundle.active=false`。
- 版本号同步维护在 `Cargo.toml` 与 `tauri.conf.json`。
- ZIP 包含 exe、README 和干净初始数据，不包含开发测试数据。
- ZIP 内为**单一以 productName 命名的顶层目录**（如 `push-wp/`、`DDL Board/`、`Launch Manager/`），三个 app 打包格式统一；打包脚本用 `$Config.productName` 作 staging 目录名。
- 主窗口类 app 默认窗口统一 1100×700、min 720×520、居中、可调整（启动器小面板除外），保证非全屏不超出屏幕。
- 发布流程：release 编译 → 打包 → Git 提交并推送 `main` 与 `vX.Y.Z` tag → GitHub Release 上传 ZIP → 回读远端大小与 digest。
- 统一用 `cargo build/test/clippy --workspace` 与 `cargo fmt --all`；各 app 打包脚本自动定位共享 `target/`。

## 部署边界

- 不主动部署到 `C:\Users\zhang\launch` 或其他用户运行目录。
- 发布产物只上传到 GitHub Release，由用户自行下载、解压和部署。
- 除非用户明确要求，不向用户目录复制、解压或启动发布产物。
- 运行验证只在项目自身的 `release` 临时目录进行。

## 自动更新

- 所有便携应用必须内置自动更新：启动时检查更新，并在“关于”或设置面板提供手动检查更新。
- 检查更新统一走方案二，**不得依赖 api.github.com 匿名 API**（共享出口 IP 易触发 60/小时限流）：
  - 请求 `https://github.com/<owner>/<repo>/releases/latest`，不跟随重定向，从 `Location` 解析 `/releases/tag/<tag>` 得到最新版本。
  - 资产 URL 直接拼 `releases/download/<tag>/<name>_<ver>_x64-portable.zip`，下载直连 release CDN（不限流）。
  - 完整性靠同 Release 内的 `SHA256SUMS.txt` 资产校验 SHA-256，不依赖 API 的 digest。
- 发布时必须随 ZIP 一并上传 `SHA256SUMS.txt`（格式 `<hex>  <zip文件名>`），供更新校验。
- 更新逻辑统一复用 `portable-kit::update`（方案二），各 app 仅保留 `UpdateConfig` 薄封装；修改更新行为只改 crate 一处。
- 更新包暂存在 exe 同级 `data/update`，只替换 exe、README 等程序文件，不覆盖用户数据。
- 替换前提示是否立即重启；立即重启由后台脚本等待进程退出后替换并重新启动，选择稍后则保留暂存更新。
- 更新失败不影响本地功能；启动时清理没有待更新元数据的遗留更新目录。

## 窗口与界面适配

- 默认（非全屏/非最大化）窗口必须完整落在屏幕工作区内，不得超出屏幕或被任务栏裁切。
- 默认窗口尺寸取保守值（参考不超过 1100×700），并设置合理的 minWidth/minHeight。
- 界面内容必须随窗口尺寸自适应；滚动只发生在面板/列表内部，任何视口下底部与关键控件不得被裁切或超出窗口。
- 提供最大化/全屏开关时，关闭后默认窗口仍需完整可见、内容不溢出。
- 启动器类工具启动外部程序可选最大化，但被启动程序自身的非全屏默认也必须完整显示。

## 图标与展示图片

- 软件图标和软件内部展示图片优先手写 SVG，并交由用户选择判断。
- 需要模型生成时，优先使用 `ustc-imagegen` 技能。
- `ustc-imagegen` 不可用时，再使用 Qoder 官方文生图接口。

## 交付前验证

- 执行 `cargo test`。
- 执行 `cargo clippy --all-targets -- -D warnings`。
- 检查前端 JavaScript 语法、元素 ID 与 Tauri 命令引用。
- 从最终 ZIP 在项目 `release` 临时目录全新解压启动，确认窗口正常、数据写入 exe 同级 `data`，且系统应用目录未创建。
- 若项目包含自动更新，验证检查、下载校验、暂存和重启替换流程，且更新不覆盖用户数据。
- 验证非全屏默认窗口完整可见、内容不裁切；如有最大化开关，两种状态都需验证。
