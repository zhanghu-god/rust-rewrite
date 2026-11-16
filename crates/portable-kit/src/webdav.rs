//! WebDAV 云备份/恢复：把每个 app 的 `data/` 当作不透明文件镜像到远端。
//!
//! 远端布局 `<root>/<app名>/…`，另存 `manifest.json` 记录每个文件的 sha/mtime 用于增量上传。
//! 使用 blocking reqwest（Basic 认证），可在同步线程与 Tauri 同步命令中调用。
//! 本地仍是主存储，WebDAV 只做备份/恢复，不做实时多端合并。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 排除的本地子目录（缓存/更新暂存/日志/恢复备份），不参与备份。
const EXCLUDED_DIRS: [&str; 5] = ["cache", "caches", "update", "logs", "webview2"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebDavConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub root: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    pub sha: String,
    pub mtime: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupReport {
    pub app: String,
    pub uploaded: usize,
    pub skipped: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub app: String,
    pub downloaded: usize,
    pub backup_dir: String,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("创建 WebDAV 客户端失败：{e}"))
}

fn auth_header(cfg: &WebDavConfig) -> String {
    let raw = format!("{}:{}", cfg.username, cfg.password);
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw))
}

fn encode_segment(seg: &str) -> String {
    let mut out = String::new();
    for b in seg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 拼接远端 URL：base + root + 各段（百分号编码）。
fn remote_url(cfg: &WebDavConfig, parts: &[&str]) -> String {
    let mut url = cfg.url.trim_end_matches('/').to_string();
    let root = cfg.root.trim_matches('/');
    if !root.is_empty() {
        url.push('/');
        url.push_str(&encode_segment(root));
    }
    for p in parts {
        for seg in p.split('/') {
            if seg.is_empty() {
                continue;
            }
            url.push('/');
            url.push_str(&encode_segment(seg));
        }
    }
    url
}

fn is_excluded(rel: &str) -> bool {
    let first = rel.split('/').next().unwrap_or("");
    EXCLUDED_DIRS.contains(&first) || first.starts_with("restore-backup-")
}

/// 递归收集本地 data 目录下需备份的文件（rel, sha, mtime）。
pub fn collect_local_files(data_dir: &Path) -> Vec<(String, FileMeta)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, FileMeta)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if path.is_dir() {
                if !is_excluded(&rel) {
                    walk(&path, &rel, out);
                }
            } else if !is_excluded(&rel) {
                if let Ok(bytes) = fs::read(&path) {
                    let sha = format!("{:x}", Sha256::digest(&bytes));
                    let mtime = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    out.push((rel, FileMeta { sha, mtime }));
                }
            }
        }
    }
    walk(data_dir, "", &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn mkcol(client: &reqwest::blocking::Client, cfg: &WebDavConfig, url: &str) -> Result<(), String> {
    let resp = client
        .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), url)
        .header(reqwest::header::AUTHORIZATION, auth_header(cfg))
        .send()
        .map_err(|e| format!("WebDAV MKCOL 失败：{e}"))?;
    let status = resp.status();
    // 405 = 已存在，视为成功
    if status.is_success() || status.as_u16() == 405 {
        Ok(())
    } else {
        Err(format!("WebDAV MKCOL {url} 返回 {status}"))
    }
}

/// 逐级创建远端目录（root/app/…）。
pub fn ensure_dirs(cfg: &WebDavConfig, parts: &[&str]) -> Result<(), String> {
    let client = client()?;
    let root = cfg.root.trim_matches('/');
    if !root.is_empty() {
        mkcol(&client, cfg, &remote_url(cfg, &[]))?;
    }
    let mut acc: Vec<&str> = Vec::new();
    for p in parts {
        for seg in p.split('/') {
            if seg.is_empty() {
                continue;
            }
            acc.push(seg);
            mkcol(&client, cfg, &remote_url(cfg, &acc))?;
        }
    }
    Ok(())
}

fn put(cfg: &WebDavConfig, parts: &[&str], bytes: &[u8]) -> Result<(), String> {
    let client = client()?;
    let url = remote_url(cfg, parts);
    let resp = client
        .put(&url)
        .header(reqwest::header::AUTHORIZATION, auth_header(cfg))
        .body(bytes.to_vec())
        .send()
        .map_err(|e| format!("WebDAV PUT 失败：{e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("WebDAV PUT {url} 返回 {}", resp.status()))
    }
}

fn get(cfg: &WebDavConfig, parts: &[&str]) -> Result<Option<Vec<u8>>, String> {
    let client = client()?;
    let url = remote_url(cfg, parts);
    let resp = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, auth_header(cfg))
        .send()
        .map_err(|e| format!("WebDAV GET 失败：{e}"))?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("WebDAV GET {url} 返回 {}", resp.status()));
    }
    let bytes = resp.bytes().map_err(|e| format!("读取 WebDAV 响应失败：{e}"))?;
    Ok(Some(bytes.to_vec()))
}

fn delete(cfg: &WebDavConfig, parts: &[&str]) -> Result<(), String> {
    let client = client()?;
    let url = remote_url(cfg, parts);
    let resp = client
        .delete(&url)
        .header(reqwest::header::AUTHORIZATION, auth_header(cfg))
        .send()
        .map_err(|e| format!("WebDAV DELETE 失败：{e}"))?;
    let s = resp.status();
    if s.is_success() || s.as_u16() == 404 {
        Ok(())
    } else {
        Err(format!("WebDAV DELETE {url} 返回 {s}"))
    }
}

fn read_manifest(cfg: &WebDavConfig, app: &str) -> BTreeMap<String, FileMeta> {
    match get(cfg, &[app, "manifest.json"]) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => BTreeMap::new(),
    }
}

/// 测试连接：创建 root 目录并确认可以访问。
pub fn test_connection(cfg: &WebDavConfig) -> Result<(), String> {
    if cfg.url.trim().is_empty() {
        return Err("未配置 WebDAV 地址".into());
    }
    ensure_dirs(cfg, &[])
}

/// 增量备份一个 app 的 data 目录到远端 `<root>/<app>/`。
pub fn backup_app(cfg: &WebDavConfig, app: &str, data_dir: &Path) -> Result<BackupReport, String> {
    ensure_dirs(cfg, &[app])?;
    let local = collect_local_files(data_dir);
    let old = read_manifest(cfg, app);
    let mut next = BTreeMap::new();
    let mut report = BackupReport { app: app.into(), ..Default::default() };

    for (rel, meta) in &local {
        let changed = old.get(rel).map(|m| &m.sha) != Some(&meta.sha);
        if changed {
            let bytes =
                fs::read(data_dir.join(rel)).map_err(|e| format!("读取 {rel} 失败：{e}"))?;
            put(cfg, &[app, rel], &bytes)?;
            report.uploaded += 1;
        } else {
            report.skipped += 1;
        }
        next.insert(rel.clone(), meta.clone());
    }
    for rel in old.keys() {
        if !next.contains_key(rel) {
            let _ = delete(cfg, &[app, rel]);
            report.removed += 1;
        }
    }
    let manifest = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
    put(cfg, &[app, "manifest.json"], &manifest)?;
    Ok(report)
}

/// 从远端恢复一个 app 的 data：先把现有本地文件备份到 restore-backup-<ts>/，再下载覆盖。
pub fn restore_app(
    cfg: &WebDavConfig,
    app: &str,
    data_dir: &Path,
) -> Result<RestoreReport, String> {
    let manifest = read_manifest(cfg, app);
    if manifest.is_empty() {
        return Err(format!("远端没有 {app} 的备份"));
    }
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let backup_dir = data_dir.join(format!("restore-backup-{ts}"));
    fs::create_dir_all(&backup_dir).map_err(|e| format!("创建恢复备份目录失败：{e}"))?;
    for (rel, _) in collect_local_files(data_dir) {
        let src = data_dir.join(&rel);
        let dst = backup_dir.join(&rel);
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(&src, &dst);
    }

    let mut report = RestoreReport {
        app: app.into(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        ..Default::default()
    };
    for rel in manifest.keys() {
        if let Some(bytes) = get(cfg, &[app, rel])? {
            let dst = data_dir.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
            }
            fs::write(&dst, bytes).map_err(|e| format!("写入 {rel} 失败：{e}"))?;
            report.downloaded += 1;
        }
    }
    Ok(report)
}

/// 列出某 app 远端备份的文件（来自 manifest），用于展示。
pub fn list_backup(cfg: &WebDavConfig, app: &str) -> Vec<String> {
    read_manifest(cfg, app).keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_path_segments() {
        assert_eq!(encode_segment("DDL Board"), "DDL%20Board");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("ok-_.~"), "ok-_.~");
    }

    #[test]
    fn remote_url_joins_root_and_parts() {
        let cfg = WebDavConfig {
            url: "https://dav.example.com/dav/".into(),
            username: "u".into(),
            password: "p".into(),
            root: "rust apps".into(),
        };
        assert_eq!(
            remote_url(&cfg, &["DDL Board", "config.json"]),
            "https://dav.example.com/dav/rust%20apps/DDL%20Board/config.json"
        );
    }

    #[test]
    fn excludes_cache_and_logs() {
        assert!(is_excluded("cache/webview2/x"));
        assert!(is_excluded("logs/a.log"));
        assert!(is_excluded("update/staged/e.exe"));
        assert!(is_excluded("restore-backup-1/config.json"));
        assert!(!is_excluded("config.json"));
        assert!(!is_excluded("tasks.json"));
    }

    #[test]
    fn collects_local_files_with_sha() {
        let dir = std::env::temp_dir().join(format!("pk-webdav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("cache")).unwrap();
        fs::write(dir.join("config.json"), b"{}").unwrap();
        fs::write(dir.join("tasks.json"), b"[]").unwrap();
        fs::write(dir.join("cache").join("x"), b"skip").unwrap();
        let files = collect_local_files(&dir);
        let names: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(names, vec!["config.json", "tasks.json"]);
        assert!(files.iter().all(|(_, m)| !m.sha.is_empty()));
        let _ = fs::remove_dir_all(dir);
    }
}
