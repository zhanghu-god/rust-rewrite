//! portable-kit —— Rust + Tauri 便携桌面应用的公共能力库。
//!
//! 提供跨项目复用的能力：
//! - [`portable`]：exe 同级 `data` 数据隔离与 WebView2 缓存重定向。
//! - [`update`]：方案二自动更新（github.com releases/latest 重定向 + SHA256SUMS 校验）。
//! - [`logging`]：运行日志与定时清理。
//! - [`webdav`]：WebDAV 云备份/恢复（增量同步）。
//! - [`dataio`]：数据导入/导出（JSON 信封）。
//!
//! 本 crate 不依赖 Tauri，纯 Rust 实现，便于在任意 Tauri app 中复用。

pub mod dataio;
pub mod logging;
pub mod portable;
pub mod update;
pub mod webdav;
