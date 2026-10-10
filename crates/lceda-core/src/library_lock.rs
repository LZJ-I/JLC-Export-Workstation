//! Hold the library's sidecar file for the full update transaction.
//! Windows exclusive sharing also works on WSL's 9P shares, where byte-range
//! LockFileEx is unsupported. The OS releases the handle after a process exits.
use crate::{Error, Result};
use std::{fs::{File, OpenOptions}, io, path::Path};

fn busy(path: &Path) -> Error {
    Error::msg(format!("库正在被其它同步任务占用，请稍后重试：{}", path.display()))
}

fn open_error(path: &Path, error: io::Error) -> Error {
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33)) { return busy(path); }
    Error::msg(format!("无法取得库同步锁：{}；系统错误：{error}。请检查该路径及目录写入权限。", path.display()))
}

pub(crate) fn acquire(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Reject every other read/write handle, including legacy byte locks.
        // Do not remove the sidecar: deleting a lock file can split ownership.
        options.share_mode(0);
    }
    let file = options.open(path).map_err(|error| open_error(path, error))?;
    #[cfg(not(windows))]
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => busy(path),
        std::fs::TryLockError::Error(error) => open_error(path, error),
    })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn excludes_another_writer_and_releases_without_removing_sidecar() {
        let path = std::env::temp_dir().join(format!("lceda-library-lock-{}", crate::util::unique_id()));
        fs::write(&path, b"retained lock contents").unwrap();
        {
            let _first = acquire(&path).unwrap();
            let error = acquire(&path).unwrap_err().to_string();
            assert!(error.contains("占用"), "{error}");
        }
        assert_eq!(fs::read(&path).unwrap(), b"retained lock contents");
        let second = acquire(&path).unwrap();
        drop(second);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_path_errors_instead_of_claiming_another_sync_is_running() {
        let path = std::env::temp_dir().join(format!("lceda-library-lock-dir-{}", crate::util::unique_id()));
        fs::create_dir(&path).unwrap();
        let error = acquire(&path).unwrap_err().to_string();
        assert!(error.contains("系统错误") && error.contains(&*path.to_string_lossy()), "{error}");
        assert!(!error.contains("稍后重试"));
        fs::remove_dir(path).unwrap();
    }
}
