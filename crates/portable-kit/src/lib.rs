//! portable-kit —— Rust + Tauri 便携桌面应用的公共能力库。
//!
//! 提供跨项目复用的两块能力：
//! - [`portable`]：exe 同级 `data` 数据隔离与 WebView2 缓存重定向。
//! - [`update`]：方案二自动更新（github.com releases/latest 重定向 + SHA256SUMS 校验），
//!   不依赖 api.github.com 匿名 API，避免共享出口 IP 限流。
//!
//! 本 crate 不依赖 Tauri，纯 Rust 实现，便于在任意 Tauri app 中复用。

pub mod logging;
pub mod portable;
pub mod update;
