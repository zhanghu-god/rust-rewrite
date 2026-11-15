//! 方案二自动更新：不依赖 api.github.com 匿名 API。
//!
//! 检查更新走 `github.com/<repo>/releases/latest` 的 302 重定向解析 tag（Web 通道，
//! 不吃 API 匿名配额，共享出口 IP 也不受限）；完整性靠 Release 内的 `SHA256SUMS.txt`
//! 资产校验；ZIP 下载直连 release CDN（同样不限流）。
//! 更新只替换 exe 与 README，不覆盖用户 data 中的配置与数据。

use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PORTABLE_SUFFIX: &str = "_x64-portable.zip";
const CHECKSUMS_NAME: &str = "SHA256SUMS.txt";

/// 每个 app 的更新配置（仓库、exe 名、资产名前缀、User-Agent）。
#[derive(Debug, Clone, Copy)]
pub struct UpdateConfig {
    pub repo: &'static str,
    pub exe_name: &'static str,
    pub asset_base: &'static str,
    pub user_agent: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_url: String,
    pub asset_name: String,
    pub asset_url: String,
    pub checksums_url: String,
    pub asset_size: u64,
    pub asset_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedUpdate {
    pub version: String,
    pub asset_name: String,
    pub already_staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagedMeta {
    version: String,
    asset_name: String,
    asset_digest: String,
    staged_exe: String,
    staged_readme: Option<String>,
}

/// 检查最新 Release：解析重定向 tag + 读取 SHA256SUMS.txt，全程不碰 api.github.com。
pub async fn check_update(cfg: &UpdateConfig, current: &str) -> Result<UpdateInfo, String> {
    let latest_url = format!("https://github.com/{}/releases/latest", cfg.repo);
    let download_base = format!("https://github.com/{}/releases/download", cfg.repo);

    let probe = probe_client(cfg, Duration::from_secs(30))?;
    let resp = probe.get(&latest_url).send().await.map_err(|e| format!("检查更新失败：{e}"))?;
    let status = resp.status();
    if !status.is_redirection() {
        return Err(format!("检查更新失败：HTTP {status}"));
    }
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| "检查更新失败：缺少重定向地址".to_string())?
        .to_str()
        .map_err(|e| format!("检查更新失败：{e}"))?;
    let tag = parse_tag_from_location(location)?;
    let latest = parse_version(&tag)?;
    let current = parse_version(current)?;

    let asset_name = format!("{}_{}{PORTABLE_SUFFIX}", cfg.asset_base, latest);
    let asset_url = format!("{download_base}/{tag}/{asset_name}");
    let checksums_url = format!("{download_base}/{tag}/{CHECKSUMS_NAME}");

    let dl = http_client(cfg, Duration::from_secs(30))?;
    let sums = dl
        .get(&checksums_url)
        .send()
        .await
        .map_err(|e| format!("读取校验文件失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("读取校验文件失败：{e}"))?
        .text()
        .await
        .map_err(|e| format!("读取校验文件失败：{e}"))?;
    let digest = parse_checksums(&sums, &asset_name)?;

    Ok(UpdateInfo {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        has_update: latest > current,
        release_url: format!("https://github.com/{}/releases/tag/{tag}", cfg.repo),
        asset_name,
        asset_url,
        checksums_url,
        asset_size: 0,
        asset_digest: digest,
    })
}

/// 下载更新包，校验 SHA-256，解压暂存 exe 与 README，并写入元数据。
pub async fn prepare_update(
    cfg: &UpdateConfig,
    data_dir: &Path,
    info: &UpdateInfo,
) -> Result<PreparedUpdate, String> {
    if !info.has_update {
        return Err("当前已是最新版本".into());
    }

    let update_dir = data_dir.join("update");
    let staged_dir = update_dir.join("staged");
    let meta_path = update_dir.join("update.json");
    fs::create_dir_all(&staged_dir).map_err(|e| format!("创建更新目录失败：{e}"))?;

    if let Ok(text) = fs::read_to_string(&meta_path) {
        if let Ok(meta) = serde_json::from_str::<StagedMeta>(&text) {
            let staged_exe = PathBuf::from(&meta.staged_exe);
            if meta.version == info.latest_version
                && meta.asset_digest == info.asset_digest
                && staged_exe.exists()
            {
                return Ok(PreparedUpdate {
                    version: meta.version,
                    asset_name: meta.asset_name,
                    already_staged: true,
                });
            }
        }
    }

    let client = http_client(cfg, Duration::from_secs(300))?;
    let bytes = client
        .get(&info.asset_url)
        .send()
        .await
        .map_err(|e| format!("下载更新失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("下载更新失败：{e}"))?
        .bytes()
        .await
        .map_err(|e| format!("下载更新失败：{e}"))?;

    if info.asset_size > 0 && bytes.len() as u64 != info.asset_size {
        return Err("更新包大小与 Release 不一致".into());
    }
    verify_digest(&bytes, &info.asset_digest)?;

    if staged_dir.exists() {
        fs::remove_dir_all(&staged_dir).map_err(|e| format!("清理旧更新文件失败：{e}"))?;
    }
    fs::create_dir_all(&staged_dir).map_err(|e| format!("创建更新目录失败：{e}"))?;

    let staged_readme = extract_update_files(cfg, &bytes, &staged_dir)?;
    let staged_exe = staged_dir.join(cfg.exe_name);
    if !staged_exe.exists() {
        return Err(format!("更新包中未找到 {}", cfg.exe_name));
    }

    let meta = StagedMeta {
        version: info.latest_version.clone(),
        asset_name: info.asset_name.clone(),
        asset_digest: info.asset_digest.clone(),
        staged_exe: staged_exe.to_string_lossy().into_owned(),
        staged_readme: staged_readme.map(|p| p.to_string_lossy().into_owned()),
    };
    let text = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(&meta_path, text).map_err(|e| format!("写入更新元数据失败：{e}"))?;

    Ok(PreparedUpdate { version: meta.version, asset_name: meta.asset_name, already_staged: false })
}

/// 生成并启动 PowerShell 更新脚本：等待本进程退出后替换 exe/README 并重新启动。
pub fn apply_update_and_restart(_cfg: &UpdateConfig, data_dir: &Path) -> Result<(), String> {
    let update_dir = data_dir.join("update");
    let meta_path = update_dir.join("update.json");
    let text = fs::read_to_string(&meta_path).map_err(|_| "没有已准备好的更新".to_string())?;
    let meta: StagedMeta =
        serde_json::from_str(&text).map_err(|e| format!("更新元数据损坏：{e}"))?;
    let staged_exe = PathBuf::from(&meta.staged_exe);
    if !staged_exe.exists() {
        return Err("暂存的新程序不存在，请重新下载更新".into());
    }

    let target_exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序：{e}"))?;
    let target_dir = target_exe.parent().ok_or_else(|| "程序路径没有父目录".to_string())?;
    let target_readme = target_dir.join("README.md");
    let staged_readme = meta.staged_readme.clone().unwrap_or_default();
    let script_path = update_dir.join("apply-update.ps1");
    fs::write(&script_path, UPDATE_HELPER_SCRIPT).map_err(|e| format!("写入更新脚本失败：{e}"))?;

    spawn_helper(
        &script_path,
        std::process::id(),
        &staged_exe,
        &target_exe,
        &staged_readme,
        &target_readme,
        &update_dir,
    )?;
    Ok(())
}

/// 查询已暂存待安装的更新（用户之前选择了“稍后”）。
pub fn pending_update(data_dir: &Path) -> Option<PreparedUpdate> {
    let text = fs::read_to_string(data_dir.join("update").join("update.json")).ok()?;
    let meta: StagedMeta = serde_json::from_str(&text).ok()?;
    if PathBuf::from(&meta.staged_exe).exists() {
        Some(PreparedUpdate {
            version: meta.version,
            asset_name: meta.asset_name,
            already_staged: true,
        })
    } else {
        None
    }
}

/// 启动时清理上次更新遗留的脚本目录；仍有 update.json 时表示用户选择了稍后更新。
pub fn cleanup_leftover(data_dir: &Path) {
    let update_dir = data_dir.join("update");
    if update_dir.exists() && !update_dir.join("update.json").exists() {
        let _ = fs::remove_dir_all(update_dir);
    }
}

/// 探测重定向的客户端：不跟随 redirect，以便读取 Location。
fn probe_client(cfg: &UpdateConfig, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(cfg.user_agent)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("创建网络客户端失败：{e}"))
}

/// 下载客户端：跟随 redirect 到 release CDN。
fn http_client(cfg: &UpdateConfig, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(cfg.user_agent)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("创建网络客户端失败：{e}"))
}

fn parse_version(tag: &str) -> Result<Version, String> {
    let normalized = tag.trim().trim_start_matches('v');
    Version::parse(normalized).map_err(|e| format!("无法解析版本号 {tag}：{e}"))
}

/// 从 releases/latest 的 Location 中解析 tag，如 /owner/repo/releases/tag/v0.3.2。
fn parse_tag_from_location(location: &str) -> Result<String, String> {
    const MARKER: &str = "/releases/tag/";
    let idx = location.find(MARKER).ok_or_else(|| "重定向地址中未找到 release tag".to_string())?;
    let rest = &location[idx + MARKER.len()..];
    let tag = rest.split(['?', '#']).next().unwrap_or(rest).trim_matches('/');
    if tag.is_empty() {
        return Err("重定向地址中 tag 为空".into());
    }
    Ok(tag.to_string())
}

/// 解析 SHA256SUMS.txt（sha256sum 风格：`<hex>  <filename>`）。
fn parse_checksums(text: &str, asset_name: &str) -> Result<String, String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hex = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("").trim_start_matches('*').trim_start_matches("./");
        if name == asset_name {
            return Ok(hex.to_ascii_lowercase());
        }
    }
    Err(format!("{CHECKSUMS_NAME} 中未找到 {asset_name} 的校验值"))
}

fn verify_digest(bytes: &[u8], digest: &str) -> Result<(), String> {
    let expected =
        digest.trim().strip_prefix("sha256:").unwrap_or(digest.trim()).to_ascii_lowercase();
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err("更新包 SHA-256 校验失败".into());
    }
    Ok(())
}

fn extract_update_files(
    cfg: &UpdateConfig,
    bytes: &[u8],
    staged_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("打开更新包失败：{e}"))?;
    let mut exe_index = None;
    let mut readme_index = None;
    let exe_suffix = format!("/{}", cfg.exe_name);

    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|e| format!("读取更新包失败：{e}"))?;
        let name = file.name().replace('\\', "/");
        if name.ends_with(&exe_suffix) || name == cfg.exe_name {
            exe_index = Some(index);
        } else if name.ends_with("/README.md") || name == "README.md" {
            readme_index = Some(index);
        }
    }

    let exe_index = exe_index.ok_or_else(|| format!("更新包中未找到 {}", cfg.exe_name))?;
    extract_one(&mut archive, exe_index, &staged_dir.join(cfg.exe_name))?;

    let readme_path = match readme_index {
        Some(index) => {
            let path = staged_dir.join("README.md");
            extract_one(&mut archive, index, &path)?;
            Some(path)
        }
        None => None,
    };
    Ok(readme_path)
}

fn extract_one(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    index: usize,
    target: &Path,
) -> Result<(), String> {
    let mut file = archive.by_index(index).map_err(|e| format!("读取更新包失败：{e}"))?;
    let mut out = fs::File::create(target).map_err(|e| format!("创建暂存文件失败：{e}"))?;
    io::copy(&mut file, &mut out).map_err(|e| format!("解压更新文件失败：{e}"))?;
    out.flush().map_err(|e| format!("写入暂存文件失败：{e}"))?;
    Ok(())
}

#[cfg(windows)]
fn spawn_helper(
    script: &Path,
    pid: u32,
    staged_exe: &Path,
    target_exe: &Path,
    staged_readme: &str,
    target_readme: &Path,
    update_dir: &Path,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
        .arg(script)
        .args(["-ProcessId", &pid.to_string()])
        .arg("-StagedExe")
        .arg(staged_exe)
        .arg("-TargetExe")
        .arg(target_exe)
        .arg("-StagedReadme")
        .arg(staged_readme)
        .arg("-TargetReadme")
        .arg(target_readme)
        .arg("-UpdateDir")
        .arg(update_dir)
        .creation_flags(CREATE_NO_WINDOW);
    command.spawn().map_err(|e| format!("启动更新脚本失败：{e}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_helper(
    _script: &Path,
    _pid: u32,
    _staged_exe: &Path,
    _target_exe: &Path,
    _staged_readme: &str,
    _target_readme: &Path,
    _update_dir: &Path,
) -> Result<(), String> {
    Err("自动更新目前仅支持 Windows".into())
}

const UPDATE_HELPER_SCRIPT: &str = r#"param(
  [int]$ProcessId,
  [string]$StagedExe,
  [string]$TargetExe,
  [string]$StagedReadme = '',
  [string]$TargetReadme,
  [string]$UpdateDir
)
$ErrorActionPreference = 'Stop'
while (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue) {
  Start-Sleep -Milliseconds 200
}
Copy-Item -LiteralPath $StagedExe -Destination $TargetExe -Force
if ($StagedReadme -and (Test-Path -LiteralPath $StagedReadme)) {
  Copy-Item -LiteralPath $StagedReadme -Destination $TargetReadme -Force
}
Start-Process -FilePath $TargetExe -WorkingDirectory (Split-Path -Parent $TargetExe)
$staged = Join-Path $UpdateDir 'staged'
if (Test-Path -LiteralPath $staged) {
  Remove-Item -LiteralPath $staged -Recurse -Force -ErrorAction SilentlyContinue
}
$meta = Join-Path $UpdateDir 'update.json'
if (Test-Path -LiteralPath $meta) {
  Remove-Item -LiteralPath $meta -Force -ErrorAction SilentlyContinue
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: UpdateConfig = UpdateConfig {
        repo: "zhanghu-god/example",
        exe_name: "example.exe",
        asset_base: "example",
        user_agent: "example-updater",
    };

    #[test]
    fn update_info_serializes_camel_case_for_frontends() {
        let info = UpdateInfo {
            current_version: "0.3.4".into(),
            latest_version: "0.3.6".into(),
            has_update: true,
            release_url: String::new(),
            asset_name: String::new(),
            asset_url: String::new(),
            checksums_url: String::new(),
            asset_size: 0,
            asset_digest: String::new(),
        };
        let v = serde_json::to_value(&info).unwrap();
        // 前端统一读 camelCase，锁定契约防止字段名漂移
        assert!(v.get("hasUpdate").is_some());
        assert!(v.get("currentVersion").is_some());
        assert!(v.get("latestVersion").is_some());
        assert!(v.get("has_update").is_none());
    }

    #[test]
    fn parses_v_prefixed_versions() {
        assert_eq!(parse_version("v0.2.0").unwrap(), Version::new(0, 2, 0));
        assert!(parse_version("v0.2.0").unwrap() > parse_version("0.1.0").unwrap());
        assert!(parse_version("not-a-version").is_err());
    }

    #[test]
    fn parses_tag_from_redirect_location() {
        assert_eq!(
            parse_tag_from_location("https://github.com/zhanghu-god/example/releases/tag/v0.3.2")
                .unwrap(),
            "v0.3.2"
        );
        assert_eq!(
            parse_tag_from_location("/zhanghu-god/example/releases/tag/v0.3.2?foo=1").unwrap(),
            "v0.3.2"
        );
        assert!(parse_tag_from_location("https://github.com/zhanghu-god/example").is_err());
    }

    #[test]
    fn parses_checksums_for_asset() {
        let sums = "aaaa  example_0.3.2_x64-portable.zip\nbbbb  README.md\n";
        assert_eq!(parse_checksums(sums, "example_0.3.2_x64-portable.zip").unwrap(), "aaaa");
        assert!(parse_checksums(sums, "missing.zip").is_err());
    }

    #[test]
    fn verifies_sha256_digest() {
        let bytes = b"example update";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        assert!(verify_digest(bytes, &digest).is_ok());
        assert!(verify_digest(bytes, "sha256:0000").is_err());
    }

    #[test]
    fn extracts_exe_and_readme_from_zip() {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("Example/example.exe", options).unwrap();
            writer.write_all(b"exe").unwrap();
            writer.start_file("Example/README.md", options).unwrap();
            writer.write_all(b"readme").unwrap();
            writer.start_file("Example/data/config.json", options).unwrap();
            writer.write_all(b"{}").unwrap();
            writer.finish().unwrap();
        }

        let dir = std::env::temp_dir().join(format!("portable-kit-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let readme = extract_update_files(&CFG, &buffer.into_inner(), &dir).unwrap();
        assert!(dir.join("example.exe").exists());
        assert!(readme.unwrap().exists());
        assert!(!dir.join("config.json").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
