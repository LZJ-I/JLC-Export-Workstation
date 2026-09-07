use serde_json::Value;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REPO: &str = "LZJ-I/JLC-Export";
pub const REPO_URL: &str = "https://github.com/LZJ-I/JLC-Export";
pub const RELEASES_URL: &str = "https://github.com/LZJ-I/JLC-Export/releases/latest";
const PROXY: &str = "https://gh-proxy.com/";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub zip_url: Option<String>,
    pub page_url: String,
    /// 当前版本到最新版本之间各 Release 的 markdown（新→旧）。
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Downloading,
    Extracting,
    Installing,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateProgress {
    pub phase: UpdatePhase,
    /// 0.0–1.0 overall progress.
    pub fraction: f32,
    /// When download size is unknown.
    pub indeterminate: bool,
}

pub type ProgressHandle = Arc<Mutex<UpdateProgress>>;

#[derive(Debug, Clone)]
pub enum CheckResult {
    Available(UpdateInfo),
    UpToDate,
    Failed,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn via_proxy(url: &str) -> String {
    if url.starts_with(PROXY) {
        url.to_string()
    } else {
        format!("{PROXY}{url}")
    }
}

pub fn check_for_update() -> CheckResult {
    let Some(local) = parse_version(current_version()) else {
        return CheckResult::Failed;
    };
    let list_api = format!("https://api.github.com/repos/{REPO}/releases?per_page=50");
    if let Some(json) = fetch_json(&list_api) {
        if let Some(result) = result_from_list(&json, local) {
            return result;
        }
    }
    let latest_api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    result_from_latest(&latest_api, local).unwrap_or(CheckResult::Failed)
}

fn fetch_json(url: &str) -> Option<Value> {
    let body = fetch_bytes(&via_proxy(url)).or_else(|| fetch_bytes(url))?;
    serde_json::from_slice(&body).ok()
}

#[derive(Debug, Clone)]
struct Release {
    version: String,
    parsed: (u64, u64, u64),
    notes: String,
    zip_url: Option<String>,
}

fn parse_release(v: &Value) -> Option<Release> {
    if v.get("draft").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    if v.get("prerelease").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let tag = v.get("tag_name").and_then(Value::as_str)?.trim();
    let parsed = parse_version(tag)?;
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        parsed,
        notes: release_notes(v),
        zip_url: v
            .get("assets")
            .and_then(Value::as_array)
            .and_then(|assets| pick_asset(assets)),
    })
}

fn result_from_list(json: &Value, local: (u64, u64, u64)) -> Option<CheckResult> {
    let arr = json.as_array()?;
    let mut rels: Vec<Release> = arr.iter().filter_map(parse_release).collect();
    if rels.is_empty() {
        return None;
    }
    rels.sort_by(|a, b| b.parsed.cmp(&a.parsed));
    let latest = rels[0].parsed;
    if latest <= local {
        return Some(CheckResult::UpToDate);
    }
    Some(CheckResult::Available(UpdateInfo {
        version: rels[0].version.clone(),
        zip_url: rels[0].zip_url.clone(),
        page_url: via_proxy(RELEASES_URL),
        notes: compose_notes(&rels, local, latest),
    }))
}

fn result_from_latest(url: &str, local: (u64, u64, u64)) -> Option<CheckResult> {
    let json = fetch_json(url)?;
    let rel = parse_release(&json)?;
    if rel.parsed <= local {
        return Some(CheckResult::UpToDate);
    }
    Some(CheckResult::Available(UpdateInfo {
        version: rel.version.clone(),
        zip_url: rel.zip_url.clone(),
        page_url: via_proxy(RELEASES_URL),
        notes: compose_notes(std::slice::from_ref(&rel), local, rel.parsed),
    }))
}

fn compose_notes(rels: &[Release], local: (u64, u64, u64), latest: (u64, u64, u64)) -> String {
    let mut chunks = Vec::new();
    for r in rels {
        if r.parsed > local && r.parsed <= latest {
            let mut block = format!("# {}", r.version);
            if !r.notes.is_empty() {
                block.push_str("\n\n");
                block.push_str(&r.notes);
            }
            chunks.push((r.parsed, block));
        }
    }
    chunks.sort_by(|a, b| b.0.cmp(&a.0));
    chunks
        .into_iter()
        .map(|(_, s)| s)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn release_notes(json: &Value) -> String {
    json.get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .replace('\r', "")
        .trim()
        .to_string()
}

pub fn download_and_apply(info: &UpdateInfo, progress: Option<ProgressHandle>) -> Result<(), String> {
    let url = info
        .zip_url
        .as_deref()
        .ok_or_else(|| "没有找到可下载的安装包".to_string())?;
    set_progress(
        &progress,
        UpdatePhase::Downloading,
        0.0,
        true,
    );
    let bytes = fetch_bytes_with_progress(url, &progress)
        .ok_or_else(|| "下载更新失败".to_string())?;
    if bytes.len() < 64 {
        return Err("下载内容太小，不是安装包".into());
    }
    set_progress(&progress, UpdatePhase::Extracting, 0.88, false);
    let exe = extract_exe(&bytes)?;
    set_progress(&progress, UpdatePhase::Installing, 0.96, false);
    replace_self_and_restart(&exe)?;
    set_progress(&progress, UpdatePhase::Idle, 1.0, false);
    Ok(())
}

pub fn cleanup_old_binary() {
    if let Ok(cur) = std::env::current_exe() {
        let _ = fs::remove_file(old_path(&cur));
    }
}

fn set_progress(
    progress: &Option<ProgressHandle>,
    phase: UpdatePhase,
    fraction: f32,
    indeterminate: bool,
) {
    if let Some(p) = progress {
        if let Ok(mut g) = p.lock() {
            g.phase = phase;
            g.fraction = fraction.clamp(0.0, 1.0);
            g.indeterminate = indeterminate;
        }
    }
}

fn pick_asset(assets: &[Value]) -> Option<String> {
    let hint = asset_hint();
    for a in assets {
        let name = a.get("name")?.as_str()?;
        let url = a.get("browser_download_url")?.as_str()?;
        if name.ends_with(".zip") && name.contains(hint) {
            return Some(via_proxy(url));
        }
    }
    None
}

fn asset_hint() -> &'static str {
    if cfg!(windows) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn extract_exe(zip_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("解压失败: {e}"))?;
    let want = if cfg!(windows) { "lceda.exe" } else { "lceda" };
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("解压失败: {e}"))?;
        let name = file.name().replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or(&name);
        if base.eq_ignore_ascii_case(want) {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            if buf.len() > 64 {
                return Ok(buf);
            }
        }
    }
    Err("压缩包里没有程序文件".into())
}

fn replace_self_and_restart(new_bytes: &[u8]) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = current.with_file_name(format!(
        "{}.new",
        current.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::write(&tmp, new_bytes).map_err(|e| format!("无法写入更新文件: {e}"))?;
    let old = old_path(&current);
    let _ = fs::remove_file(&old);
    fs::rename(&current, &old).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("无法替换正在运行的程序: {e}")
    })?;
    if let Err(e) = fs::rename(&tmp, &current) {
        let _ = fs::rename(&old, &current);
        return Err(format!("无法安装新版本: {e}"));
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cmd = Command::new(&current);
    cmd.args(&args);
    cmd.spawn().map_err(|e| {
        let _ = fs::rename(&old, &current);
        format!("无法重新启动: {e}")
    })?;
    std::process::exit(0);
}

fn old_path(current: &Path) -> PathBuf {
    let name = current.file_name().unwrap_or_default().to_string_lossy();
    current.with_file_name(format!("{name}.old"))
}

fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    fetch_bytes_with_progress(url, &None)
}

fn fetch_bytes_with_progress(url: &str, progress: &Option<ProgressHandle>) -> Option<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .user_agent("JLC-Export")
        .build();
    let resp = agent
        .get(url)
        .set("Accept", "application/vnd.github+json,*/*")
        .call()
        .ok()?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0);
    let mut reader = resp.into_reader();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 80 * 1024 * 1024 {
            return None;
        }
        if let Some(total) = total {
            let frac = (buf.len() as f64 / total as f64).min(1.0) as f32;
            set_progress(progress, UpdatePhase::Downloading, frac * 0.85, false);
        } else {
            set_progress(progress, UpdatePhase::Downloading, 0.0, true);
        }
    }
    Some(buf)
}

pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_order() {
        assert!(parse_version("v0.1.2").unwrap() > parse_version("0.1.1").unwrap());
        assert_eq!(parse_version("v0.1.1"), parse_version("0.1.1"));
        assert!(parse_version("0.2.0").unwrap() > parse_version("0.1.9").unwrap());
        assert!(parse_version("0.2.1").unwrap() > parse_version("0.2.0").unwrap());
        assert!(parse_version("0.3.0").unwrap() > parse_version("0.2.1").unwrap());
        assert!(parse_version("0.3.1").unwrap() > parse_version("0.3.0").unwrap());
        assert!(parse_version("0.3.2").unwrap() > parse_version("0.3.1").unwrap());
        assert!(parse_version("0.3.3").unwrap() > parse_version("0.3.2").unwrap());
        assert!(parse_version("0.5.0").unwrap() > parse_version("0.3.3").unwrap());
        assert!(parse_version("0.5.1").unwrap() > parse_version("0.5.0").unwrap());
        assert!(parse_version("0.5.2").unwrap() > parse_version("0.5.1").unwrap());
        assert!(parse_version("0.5.3").unwrap() > parse_version("0.5.2").unwrap());
    }

    #[test]
    fn pick_asset_matches_current_os_only() {
        let assets = serde_json::json!([
            {
                "name": "lceda-v0.5.1-x86_64-pc-windows-msvc.zip",
                "browser_download_url": "https://example.com/win.zip"
            },
            {
                "name": "lceda-v0.5.1-x86_64-unknown-linux-gnu.zip",
                "browser_download_url": "https://example.com/linux.zip"
            },
            {
                "name": "lceda-v0.5.1-aarch64-apple-darwin.zip",
                "browser_download_url": "https://example.com/mac-arm.zip"
            },
            {
                "name": "lceda-v0.5.1-x86_64-apple-darwin.zip",
                "browser_download_url": "https://example.com/mac-intel.zip"
            }
        ]);
        let url = pick_asset(assets.as_array().unwrap()).expect("matching zip");
        let expect = if cfg!(windows) {
            "win.zip"
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "mac-arm.zip"
        } else if cfg!(target_os = "macos") {
            "mac-intel.zip"
        } else {
            "linux.zip"
        };
        assert!(url.ends_with(expect), "{url} should end with {expect}");
    }

    #[test]
    fn proxy_wraps_once() {
        let u = "https://github.com/LZJ-I/JLC-Export/releases/latest";
        let p = via_proxy(u);
        assert!(p.starts_with(PROXY));
        assert_eq!(via_proxy(&p), p);
    }

    #[test]
    fn release_notes_trim_and_normalize() {
        let json = serde_json::json!({ "body": "\r\n- 修复导出\r\n- 更新文档\n  " });
        assert_eq!(release_notes(&json), "- 修复导出\n- 更新文档");
        let empty = serde_json::json!({ "body": null });
        assert!(release_notes(&empty).is_empty());
    }

    #[test]
    fn changelog_lists_versions_after_local() {
        let json = serde_json::json!([
            { "tag_name": "v0.3.3", "body": "## 改进\n- 放大", "draft": false, "prerelease": false },
            { "tag_name": "v0.3.2", "body": "## 改进\n- 旋转", "draft": false, "prerelease": false },
            { "tag_name": "v0.3.1", "body": "## 修复\n- PcbLib", "draft": false, "prerelease": false },
            { "tag_name": "v0.2.1", "body": "## 改进\n- 进度条", "draft": false, "prerelease": false },
        ]);
        let CheckResult::Available(info) =
            result_from_list(&json, parse_version("0.3.1").unwrap()).unwrap()
        else {
            panic!("expected available");
        };
        assert_eq!(info.version, "0.3.3");
        assert!(info.notes.starts_with("# 0.3.3"));
        assert!(info.notes.contains("# 0.3.2"));
        assert!(info.notes.contains("放大"));
        assert!(info.notes.contains("旋转"));
        assert!(!info.notes.contains("# 0.3.1"));
        assert!(!info.notes.contains("PcbLib"));
        assert!(!info.notes.contains("0.2.1"));
    }

    #[test]
    fn changelog_skips_draft_and_prerelease() {
        let json = serde_json::json!([
            { "tag_name": "v0.3.3", "body": "stable", "draft": false, "prerelease": false },
            { "tag_name": "v0.3.4-rc", "body": "rc", "draft": false, "prerelease": true },
            { "tag_name": "v0.3.2", "body": "drafted", "draft": true, "prerelease": false },
        ]);
        let CheckResult::Available(info) =
            result_from_list(&json, parse_version("0.3.0").unwrap()).unwrap()
        else {
            panic!("expected available");
        };
        assert_eq!(info.version, "0.3.3");
        assert!(info.notes.contains("stable"));
        assert!(!info.notes.contains("rc"));
        assert!(!info.notes.contains("drafted"));
    }
}
