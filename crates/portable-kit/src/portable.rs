//! 便携数据隔离：所有数据/配置/缓存落在 exe 同级 `data`，不写系统目录。

use std::fs;
use std::path::{Path, PathBuf};

/// 返回 exe 同级的 `data` 目录（不创建）。
pub fn portable_data_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取程序路径：{e}"))?;
    let exe_dir = exe.parent().ok_or_else(|| "程序路径没有父目录".to_string())?;
    Ok(exe_dir.join("data"))
}

/// 在创建窗口前把 WebView2 缓存重定向到 `data/cache/webview2`。
///
/// 仅设置该环境变量不足以阻止 Tauri 宿主创建默认系统目录，
/// 还需配合 `appDirectoriesOverride`（见各 app 的 lib.rs / tauri.conf.json）。
pub fn configure_webview_data_dir(data_dir: &Path) -> Result<(), String> {
    let cache_dir = data_dir.join("cache").join("webview2");
    fs::create_dir_all(&cache_dir).map_err(|e| format!("无法创建 WebView2 缓存目录：{e}"))?;
    std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", cache_dir);
    Ok(())
}
