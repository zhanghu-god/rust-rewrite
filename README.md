# rust-rewrite 工作空间

Rust + Tauri 便携桌面软件的 monorepo 工作空间。聚合多个独立 app 与一个公共能力 crate，
共享编译输出与依赖版本，减少重复造轮子。

## 结构

```text
rust-rewrite/
├─ Cargo.toml              # workspace：成员 + 集中依赖版本 + release profile
├─ rustfmt.toml            # 统一代码风格
├─ crates/
│  └─ portable-kit/        # 公共能力 crate（本仓库托管）
│     ├─ src/portable.rs   # exe 同级 data 数据隔离 + WebView2 缓存重定向
│     └─ src/update.rs     # 方案二自动更新（Web 重定向 + SHA256SUMS，不依赖匿名 API）
├─ shared/
│  └─ ui/                  # 共享前端层（base.css 设计令牌/基础样式 + base.js PKUI 工具库）
├─ scripts/
│  └─ sync-ui.mjs          # 同步 shared/ui 到各 app src/shared
├─ push-wp/                # 独立 git 仓库（app）
├─ ddl-desktop/            # 独立 git 仓库（app）
└─ launch-manager/         # 独立 git 仓库（app）
```

## 公共 crate：portable-kit

- `portable::portable_data_dir()` / `portable::configure_webview_data_dir()`：数据隔离。
- `update::{check_update, prepare_update, apply_update_and_restart, pending_update, cleanup_leftover}`：
  方案二自动更新，参数化 `UpdateConfig { repo, exe_name, asset_base, user_agent }`。
- 不依赖 Tauri，纯 Rust，可在任意 Tauri app 复用。

各 app 通过 `portable-kit.workspace = true`（path 依赖）引用，并在自己的 `update.rs`
里定义 `UpdateConfig` 常量做薄封装，命令层保持调用 `update::*` 不变。

前端 likewise：三个 app 共用 `shared/ui/base.css`（家族调色板/重置/滚动条/toast）与
`base.js`（`window.PKUI` 工具库）。`scripts/sync-ui.mjs` 把它们同步到各 app `src/shared`，
打包脚本自动调用；改公共前端只改 `shared/ui` 一处。

## 构建

```powershell
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"
cargo build --workspace          # 共享 target/，一次编译全部
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

单独构建某个 app：`cargo build -p push-wp`（或 `-p ddl-desktop` / `-p launch-manager`）。
打包仍用各 app 的 `scripts/package-portable.ps1`（自动定位共享 target）。

## 源码构建说明

三个 app 是独立仓库，但通过 path 依赖引用本工作空间的 `crates/portable-kit`。
**从源码构建 app 需要保持本工作空间布局**（根仓库 + 三个 app 仓库并列克隆）。
面向最终用户分发的是便携 ZIP 二进制，不依赖源码布局。

## 规则

工程约定见 `.qoder/rules/rust-desktop.md`（always_on），其中共同点已关联到
`portable-kit` 对应模块。
