//! 完整文件暂存、同卷重命名、保留备份；从不截断正在使用的 EXE。
//! 参考 self-replace 的 Windows 重命名策略和 Tauri 的失败恢复流程。
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const MAX_EXE_SIZE: u64 = 160 * 1024 * 1024;

pub fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn file_digest(path: &Path) -> io::Result<String> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_EXE_SIZE {
        return Err(io::Error::other("程序文件大小超出限制"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_EXE_SIZE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_EXE_SIZE {
        return Err(io::Error::other("程序文件大小超出限制"));
    }
    Ok(digest(&bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub dest: PathBuf,
    pub candidate: PathBuf,
    pub old_hash: String,
    pub new_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub dest: PathBuf,
    pub backup: PathBuf,
    pub old_hash: String,
    pub new_hash: String,
    pub was_readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub message: String,
    pub os_code: Option<i32>,
}

impl Failure {
    fn at(step: &str, path: &Path, error: io::Error) -> Self {
        Self {
            message: format!("{step}：{}\n{error}", path.display()),
            os_code: error.raw_os_error(),
        }
    }
    pub fn text(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            os_code: None,
        }
    }
    #[cfg(windows)]
    pub fn needs_elevation(&self) -> bool {
        self.os_code == Some(5)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn check(path: &Path, expected: &str) -> Result<(), Failure> {
    let actual = file_digest(path).map_err(|e| Failure::at("无法核对程序文件", path, e))?;
    if actual != expected {
        return Err(Failure::text(format!(
            "程序文件已被其他操作更改，已停止更新并保留现场：{}",
            path.display()
        )));
    }
    Ok(())
}

// 锁文件不删除：删除再创建会使并发进程锁住不同文件。系统在进程退出时释放锁。
fn lock(dest: &Path) -> Result<File, Failure> {
    let name = dest
        .file_name()
        .ok_or_else(|| Failure::text("更新目标缺少文件名"))?;
    let path = dest.with_file_name(format!(".{}.update-lock", name.to_string_lossy()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| Failure::at("无法准备更新目录", &path, e))?;
    file.try_lock().map_err(|e| {
        Failure::text(format!(
            "另一个更新正在处理此程序，请稍后重试：{}\n{e}",
            dest.display()
        ))
    })?;
    Ok(file)
}

fn rename_retry(from: &Path, to: &Path) -> io::Result<()> {
    let start = Instant::now();
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            // 仅等待短暂的共享/锁定冲突。无权限不能靠无限重试解决。
            Err(e)
                if cfg!(windows)
                    && matches!(e.raw_os_error(), Some(32 | 33))
                    && start.elapsed() < Duration::from_secs(20) =>
            {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    }
}

pub fn install(request: &Request) -> Result<Installed, Failure> {
    let _guard = lock(&request.dest)?;
    check(&request.dest, &request.old_hash)?;
    check(&request.candidate, &request.new_hash)?;
    let parent = request
        .dest
        .parent()
        .ok_or_else(|| Failure::text("无效的程序目录"))?;
    // 与目标同卷暂存，避免 %TEMP% 位于另一磁盘时无法重命名。
    let staging = tempfile::Builder::new()
        .prefix(".lceda-update-")
        .tempdir_in(parent)
        .map_err(|e| Failure::at("无法写入程序目录", parent, e))?;
    let prepared = staging.path().join("new.exe");
    let backup = staging.path().join("previous.exe");
    let permissions = fs::metadata(&request.dest)
        .map_err(|e| Failure::at("无法读取原程序权限", &request.dest, e))?
        .permissions();
    let was_readonly = permissions.readonly();
    fs::copy(&request.candidate, &prepared)
        .map_err(|e| Failure::at("无法暂存新程序", &prepared, e))?;
    File::options()
        .write(true)
        .open(&prepared)
        .and_then(|f| f.sync_all())
        .map_err(|e| Failure::at("无法完整保存新程序", &prepared, e))?;
    #[cfg(unix)]
    fs::set_permissions(&prepared, permissions.clone())
        .map_err(|e| Failure::at("无法保留程序执行权限", &prepared, e))?;
    check(&prepared, &request.new_hash)?;
    check(&request.dest, &request.old_hash)?;
    let installed = Installed {
        dest: request.dest.clone(),
        backup,
        old_hash: request.old_hash.clone(),
        new_hash: request.new_hash.clone(),
        was_readonly,
    };
    write_json(&staging.path().join("transaction.json"), &installed)
        .map_err(|e| Failure::at("无法保存恢复记录，尚未替换原程序", staging.path(), e))?;
    #[cfg(windows)]
    if was_readonly {
        let mut writable = permissions.clone();
        writable.set_readonly(false);
        fs::set_permissions(&request.dest, writable)
            .map_err(|e| Failure::at("无法解除原程序只读属性", &request.dest, e))?;
    }
    if let Err(e) = rename_retry(&request.dest, &installed.backup) {
        let _ = fs::set_permissions(&request.dest, permissions);
        return Err(Failure::at(
            "原程序被占用或无法移动，旧版仍可继续使用",
            &request.dest,
            e,
        ));
    }
    // 从此必须保留备份；即使发生第二次写入/回滚失败也不能由 TempDir 删除。
    let _ = staging.keep();
    if let Err(e) = rename_retry(&prepared, &request.dest) {
        let restore = rename_retry(&installed.backup, &request.dest);
        if let Err(restore_error) = restore {
            return Err(Failure::text(format!(
                "安装失败：{e}；恢复失败：{restore_error}\n完整旧版保留在：{}",
                installed.backup.display()
            )));
        }
        let _ = fs::set_permissions(&request.dest, permissions);
        let _ = fs::remove_dir_all(installed.backup.parent().unwrap());
        return Err(Failure::at("安装未完成，已恢复旧版", &request.dest, e));
    }
    if let Err(error) = check(&request.dest, &request.new_hash) {
        // 目标被外部改动时不能覆盖它，完整旧版仍在 backup。
        return Err(Failure::text(format!(
            "{error}\n完整旧版保留在：{}",
            installed.backup.display()
        )));
    }
    Ok(installed)
}

pub fn rollback(installed: &Installed) -> Result<(), Failure> {
    let _guard = lock(&installed.dest)?;
    check(&installed.dest, &installed.new_hash)?;
    check(&installed.backup, &installed.old_hash)?;
    let rejected = installed.backup.with_file_name("rejected.exe");
    if rejected.exists() {
        return Err(Failure::text("恢复目录已有文件，已保留现场"));
    }
    rename_retry(&installed.dest, &rejected)
        .map_err(|e| Failure::at("无法移开未启动的新版本", &installed.dest, e))?;
    if let Err(e) = rename_retry(&installed.backup, &installed.dest) {
        let _ = rename_retry(&rejected, &installed.dest);
        return Err(Failure::at(
            "无法恢复旧版，备份已保留",
            &installed.backup,
            e,
        ));
    }
    #[cfg(windows)]
    if installed.was_readonly {
        if let Ok(metadata) = fs::metadata(&installed.dest) {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(true);
            let _ = fs::set_permissions(&installed.dest, permissions);
        }
    }
    let _ = fs::remove_dir_all(installed.backup.parent().unwrap());
    Ok(())
}

pub fn cleanup(installed: &Installed) -> Result<(), Failure> {
    // 不清理不属于此次更新的文件，也不在新版被外部修改后删除唯一备份。
    check(&installed.dest, &installed.new_hash)?;
    if installed.backup.exists() {
        check(&installed.backup, &installed.old_hash)?;
        fs::remove_file(&installed.backup)
            .map_err(|e| Failure::at("旧版尚未退出", &installed.backup, e))?;
    }
    let parent = installed.backup.parent().unwrap();
    let _ = fs::remove_file(parent.join("transaction.json"));
    let _ = fs::remove_dir(parent);
    Ok(())
}

pub fn write_json(path: &Path, data: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(data).map_err(io::Error::other)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("无效的记录路径"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(64 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("更新记录过大".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Request) {
        let dir = tempfile::Builder::new()
            .prefix("update 中文 & space ")
            .tempdir()
            .unwrap();
        let dest = dir.path().join("工作站 old.exe");
        let candidate = dir.path().join("新版 candidate.exe");
        fs::write(&dest, b"original executable").unwrap();
        fs::write(&candidate, b"complete new executable").unwrap();
        let request = Request {
            old_hash: file_digest(&dest).unwrap(),
            new_hash: file_digest(&candidate).unwrap(),
            dest,
            candidate,
        };
        (dir, request)
    }

    #[test]
    fn install_preserves_full_backup_and_can_rollback() {
        let (_dir, request) = fixture();
        let installed = install(&request).unwrap();
        assert_eq!(file_digest(&request.dest).unwrap(), request.new_hash);
        assert_eq!(file_digest(&installed.backup).unwrap(), request.old_hash);
        let record: Installed =
            read_json(&installed.backup.parent().unwrap().join("transaction.json")).unwrap();
        assert_eq!(record.dest, installed.dest);
        rollback(&installed).unwrap();
        assert_eq!(file_digest(&request.dest).unwrap(), request.old_hash);
        assert!(!installed.backup.exists());
    }

    #[test]
    fn corrupt_download_does_not_touch_original() {
        let (_dir, request) = fixture();
        fs::write(&request.candidate, b"truncated").unwrap();
        assert!(install(&request).is_err());
        assert_eq!(file_digest(&request.dest).unwrap(), request.old_hash);
    }

    #[test]
    fn external_change_before_install_is_preserved() {
        let (_dir, request) = fixture();
        fs::write(&request.dest, b"another updater already wrote this").unwrap();
        assert!(install(&request).is_err());
        assert_eq!(
            fs::read(&request.dest).unwrap(),
            b"another updater already wrote this"
        );
    }

    #[test]
    fn rollback_does_not_overwrite_external_change_or_corrupt_backup() {
        let (_dir, request) = fixture();
        let installed = install(&request).unwrap();
        fs::write(&request.dest, b"external edit").unwrap();
        assert!(rollback(&installed).is_err());
        assert_eq!(fs::read(&request.dest).unwrap(), b"external edit");
        assert_eq!(file_digest(&installed.backup).unwrap(), request.old_hash);
        fs::copy(&request.candidate, &request.dest).unwrap();
        fs::write(&installed.backup, b"damaged backup").unwrap();
        assert!(rollback(&installed).is_err());
        assert_eq!(file_digest(&request.dest).unwrap(), request.new_hash);
    }

    #[test]
    fn concurrent_installer_is_rejected_without_changing_program() {
        let (_dir, request) = fixture();
        let _held = lock(&request.dest).unwrap();
        assert!(install(&request)
            .unwrap_err()
            .message
            .contains("另一个更新"));
        assert_eq!(file_digest(&request.dest).unwrap(), request.old_hash);
    }

    #[test]
    fn cleanup_checks_both_versions_before_deleting_backup() {
        let (_dir, request) = fixture();
        let installed = install(&request).unwrap();
        fs::write(&request.dest, b"externally modified").unwrap();
        assert!(cleanup(&installed).is_err());
        assert!(installed.backup.exists());
        fs::copy(&request.candidate, &request.dest).unwrap();
        cleanup(&installed).unwrap();
        assert!(!installed.backup.exists());
        assert_eq!(file_digest(&request.dest).unwrap(), request.new_hash);
    }

    #[cfg(unix)]
    #[test]
    fn executable_permissions_survive_install_and_rollback() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, request) = fixture();
        fs::set_permissions(&request.dest, fs::Permissions::from_mode(0o751)).unwrap();
        let installed = install(&request).unwrap();
        assert_eq!(
            fs::metadata(&request.dest).unwrap().permissions().mode() & 0o777,
            0o751
        );
        rollback(&installed).unwrap();
        assert_eq!(
            fs::metadata(&request.dest).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }

    #[cfg(unix)]
    #[test]
    fn readonly_executable_updates_without_truncating_original() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, request) = fixture();
        fs::set_permissions(&request.dest, fs::Permissions::from_mode(0o555)).unwrap();
        let installed = install(&request).unwrap();
        assert_eq!(file_digest(&request.dest).unwrap(), request.new_hash);
        assert_eq!(
            fs::metadata(&request.dest).unwrap().permissions().mode() & 0o777,
            0o555
        );
        rollback(&installed).unwrap();
        assert_eq!(file_digest(&request.dest).unwrap(), request.old_hash);
    }

    #[cfg(windows)]
    #[test]
    fn readonly_target_updates_and_rollback_restores_attribute() {
        let (_dir, request) = fixture();
        let mut permissions = fs::metadata(&request.dest).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&request.dest, permissions).unwrap();
        let installed = install(&request).unwrap();
        assert_eq!(file_digest(&request.dest).unwrap(), request.new_hash);
        assert!(!fs::metadata(&request.dest)
            .unwrap()
            .permissions()
            .readonly());
        rollback(&installed).unwrap();
        assert_eq!(file_digest(&request.dest).unwrap(), request.old_hash);
        assert!(fs::metadata(&request.dest)
            .unwrap()
            .permissions()
            .readonly());
        let mut permissions = fs::metadata(&request.dest).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&request.dest, permissions).unwrap();
    }
}
