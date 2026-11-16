//! 数据导入/导出：把 app 的 `data/` 文本文件打包为单个 JSON 信封，便于本地迁移与分享。
//!
//! 信封格式 `{format, version, app, exportedAt, files:{rel: 文本内容}}`。
//! 导入前自动把现有本地文件备份到 `restore-backup-<ts>/`，再写入信封内容，避免误覆盖丢数据。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::webdav::collect_local_files;

pub const FORMAT: &str = "portable-kit-data";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub app: String,
    pub written: usize,
    pub backup_dir: String,
}

/// 导出 app 数据为 JSON 信封（仅文本文件，排除缓存/日志/更新暂存）。
pub fn export_app(app: &str, data_dir: &Path) -> Result<Value, String> {
    let mut files = BTreeMap::new();
    for (rel, _) in collect_local_files(data_dir) {
        let bytes = fs::read(data_dir.join(&rel)).map_err(|e| format!("读取 {rel} 失败：{e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{rel} 不是文本文件，无法导出"))?;
        files.insert(rel, text);
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(serde_json::json!({
        "format": FORMAT,
        "version": VERSION,
        "app": app,
        "exportedAt": now,
        "files": files,
    }))
}

/// 读取并校验导入文件，返回信封。
pub fn read_envelope(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取导入文件失败：{e}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("导入文件不是有效 JSON：{e}"))?;
    validate_envelope(&value)?;
    Ok(value)
}

pub fn validate_envelope(value: &Value) -> Result<(), String> {
    if value.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
        return Err("导入文件格式不符（缺少 portable-kit-data 标识）".into());
    }
    if value.get("files").and_then(|v| v.as_object()).is_none() {
        return Err("导入文件缺少 files 字段".into());
    }
    Ok(())
}

/// 导入信封到 app 数据目录：先备份现有文件，再写入。expected_app 用于防止跨 app 误导入。
pub fn import_app(
    value: &Value,
    expected_app: &str,
    data_dir: &Path,
) -> Result<ImportReport, String> {
    validate_envelope(value)?;
    let app = value.get("app").and_then(|v| v.as_str()).unwrap_or("");
    if !app.is_empty() && app != expected_app {
        return Err(format!("该备份属于 {app}，与当前 {expected_app} 不符"));
    }
    let files = value.get("files").and_then(|v| v.as_object()).ok_or("缺少 files")?;

    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let backup_dir = data_dir.join(format!("restore-backup-{ts}"));
    fs::create_dir_all(&backup_dir).map_err(|e| format!("创建导入备份目录失败：{e}"))?;
    for (rel, _) in collect_local_files(data_dir) {
        let dst = backup_dir.join(&rel);
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(data_dir.join(&rel), &dst);
    }

    let mut report = ImportReport {
        app: expected_app.into(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        ..Default::default()
    };
    for (rel, content) in files {
        let text = content.as_str().ok_or_else(|| format!("{rel} 内容不是文本"))?;
        let dst = data_dir.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
        fs::write(&dst, text).map_err(|e| format!("写入 {rel} 失败：{e}"))?;
        report.written += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pk-dataio-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn export_then_import_roundtrip() {
        let src = temp_dir("src");
        fs::write(src.join("config.json"), "{\"a\":1}").unwrap();
        fs::write(src.join("tasks.json"), "[]").unwrap();
        let envelope = export_app("ddl-desktop", &src).unwrap();
        assert_eq!(envelope["app"], "ddl-desktop");
        assert_eq!(envelope["files"]["config.json"], "{\"a\":1}");

        let dst = temp_dir("dst");
        fs::write(dst.join("config.json"), "{\"old\":true}").unwrap();
        let report = import_app(&envelope, "ddl-desktop", &dst).unwrap();
        assert_eq!(report.written, 2);
        assert_eq!(fs::read_to_string(dst.join("config.json")).unwrap(), "{\"a\":1}");
        // 旧文件被备份
        assert!(Path::new(&report.backup_dir).join("config.json").exists());
        let _ = fs::remove_dir_all(src);
        let _ = fs::remove_dir_all(dst);
    }

    #[test]
    fn rejects_wrong_app_and_bad_format() {
        let dir = temp_dir("bad");
        let envelope = export_app("push-wp", &dir).unwrap();
        assert!(import_app(&envelope, "ddl-desktop", &dir).is_err());
        assert!(validate_envelope(&serde_json::json!({"files":{}})).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
