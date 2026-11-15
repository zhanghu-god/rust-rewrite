//! 运行日志：写入 exe 同级 `data/logs`，按天分文件，便于定位问题与改进软件。
//!
//! 每行格式：`[RFC3339 时间] [级别] [应用名] 消息`。
//! 提供 panic hook 记录崩溃位置与背栈；提供过期日志清理供常驻管理器定时调用。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, SecondsFormat, Utc};

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static APP_NAME: OnceLock<String> = OnceLock::new();

/// 初始化日志目录（exe 同级 data/logs）并记录应用名。返回日志目录。
pub fn init(app_name: &str) -> Result<PathBuf, String> {
    let dir = crate::portable::portable_data_dir()?.join("logs");
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建日志目录：{e}"))?;
    let _ = LOG_DIR.set(dir.clone());
    let _ = APP_NAME.set(app_name.to_string());
    Ok(dir)
}

/// 当前日志目录（未初始化时为 None）。
pub fn log_dir() -> Option<&'static Path> {
    LOG_DIR.get().map(|p| p.as_path())
}

fn write_line(level: &str, msg: &str) {
    let Some(dir) = LOG_DIR.get() else { return };
    let now = Utc::now();
    let day = now.format("%Y-%m-%d");
    let ts = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let app = APP_NAME.get().map(|s| s.as_str()).unwrap_or("app");
    let path = dir.join(format!("{app}-{day}.log"));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{ts}] [{level}] [{app}] {msg}");
    }
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

pub fn warn(msg: &str) {
    write_line("WARN", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}

/// 安装 panic hook：崩溃时记录消息、位置与背栈，便于定位问题。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("unknown panic");
        let backtrace = std::backtrace::Backtrace::force_capture();
        write_line("PANIC", &format!("{msg} at {loc}\n{backtrace}"));
    }));
}

/// 删除日志目录中修改时间早于 retention_days 的 *.log，返回删除个数。
pub fn cleanup_old_logs(dir: &Path, retention_days: i64) -> Result<usize, String> {
    if !dir.exists() {
        return Ok(0);
    }
    let cutoff = Utc::now() - chrono::Duration::days(retention_days);
    let mut removed = 0usize;
    let entries = fs::read_dir(dir).map_err(|e| format!("读取日志目录失败：{e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        let modified: DateTime<Utc> = modified.into();
        if modified < cutoff && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
