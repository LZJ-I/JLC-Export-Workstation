//! Validate a complete AD undo journal before replacing any library file.
use crate::{native_library::content_hash, Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::{Read, Write}, path::Path};

#[derive(Serialize, Deserialize)]
struct State { old: Option<u64>, new: Option<u64> }
#[derive(Serialize, Deserialize)]
struct Manifest { version: u32, files: BTreeMap<String, State> }

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 * 1024 { return Err(Error::msg("AD 事务文件超过 256 MiB")); }
    Ok(Some(bytes))
}

pub(crate) fn record(journal: &Path, files: &[(&str, Option<&[u8]>, Option<&[u8]>)]) -> Result<()> {
    let files = files.iter().map(|(name, old, new)| ((*name).into(), State {
        old: old.map(content_hash), new: new.map(content_hash),
    })).collect();
    let mut file = fs::File::create(journal.join("recovery.json"))?;
    file.write_all(&serde_json::to_vec(&Manifest { version: 1, files })?)?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn restore(journal: &Path, files: &[(&Path, &str)]) -> Result<()> {
    let manifest: Option<Manifest> = read_optional(&journal.join("recovery.json"))?
        .map(|bytes| serde_json::from_slice(&bytes)).transpose()?;
    if manifest.as_ref().is_some_and(|m| m.version != 1 || m.files.len() != files.len()) {
        return Err(Error::msg("AD 事务恢复记录无效；原件和备份已保留"));
    }
    let mut ready = Vec::new();
    for (target, backup) in files {
        let old = read_optional(&journal.join(backup))?;
        let current = read_optional(target)?;
        let old_hash = old.as_deref().map(content_hash);
        let current_hash = current.as_deref().map(content_hash);
        if let Some(manifest) = &manifest {
            let state = manifest.files.get(*backup).ok_or_else(|| Error::msg("AD 事务恢复记录缺少文件"))?;
            if old_hash != state.old { return Err(Error::msg("AD 事务备份不完整或已损坏；原件和备份已保留")); }
            if current_hash != state.old && current_hash != state.new {
                return Err(Error::msg(format!("中断后 {} 已被其它程序修改或删除；未恢复任何文件，备份保留在 {}", target.display(), journal.display())));
            }
        } else if current != old {
            // Older releases recorded no new-file digest. A changed file may be
            // a user edit, so legacy journals must never blindly roll it back.
            return Err(Error::msg(format!("旧版 AD 事务无法确认 {} 的当前内容；未恢复任何文件，请核对原件及备份 {} 后手动恢复", target.display(), journal.display())));
        }
        ready.push((*target, old, current));
    }
    for (index, (target, old, current)) in ready.into_iter().enumerate() {
        if current == old { continue; }
        if let Some(bytes) = old {
            let staged = journal.join(format!("{index}.restore"));
            let mut file = fs::File::create(&staged)?;
            file.write_all(&bytes)?; file.sync_all()?; drop(file);
            fs::rename(staged, target)?;
        } else { fs::remove_file(target)?; }
    }
    fs::remove_dir_all(journal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("ad-undo-{}", crate::util::unique_id()));
            fs::create_dir(&path).unwrap(); Self(path)
        }
    }
    impl Drop for Directory { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn verifies_all_current_files_and_backups_before_restoring_anything() {
        for fault in ["external_edit", "deleted", "corrupt_backup", "missing_backup", "legacy"] {
            let dir = Directory::new(); let journal = dir.0.join("journal"); fs::create_dir(&journal).unwrap();
            let a = dir.0.join("a"); let b = dir.0.join("b");
            for name in ["a", "b"] { fs::write(journal.join(name), b"old").unwrap(); fs::write(dir.0.join(name), b"new").unwrap(); }
            record(&journal, &[("a", Some(b"old"), Some(b"new")), ("b", Some(b"old"), Some(b"new"))]).unwrap();
            match fault {
                "external_edit" => fs::write(&b, b"external").unwrap(),
                "deleted" => fs::remove_file(&b).unwrap(),
                "corrupt_backup" => fs::write(journal.join("b"), b"corrupt").unwrap(),
                "missing_backup" => fs::remove_file(journal.join("b")).unwrap(),
                _ => fs::remove_file(journal.join("recovery.json")).unwrap(),
            }
            let before = read_optional(&b).unwrap();
            assert!(restore(&journal, &[(&a, "a"), (&b, "b")]).is_err(), "{fault}");
            assert_eq!(fs::read(&a).unwrap(), b"new", "must validate b before restoring a: {fault}");
            assert_eq!(read_optional(&b).unwrap(), before);
            assert!(journal.is_dir());
        }
    }

    #[test]
    fn removes_only_known_new_files_and_resumes_partial_rollback() {
        let dir = Directory::new(); let journal = dir.0.join("journal"); fs::create_dir(&journal).unwrap();
        let a = dir.0.join("a"); let b = dir.0.join("b");
        fs::write(journal.join("a"), b"old").unwrap(); fs::write(&a, b"old").unwrap(); fs::write(&b, b"new").unwrap();
        record(&journal, &[("a", Some(b"old"), Some(b"new")), ("b", None, Some(b"new"))]).unwrap();
        fs::write(&b, b"user edited the newly created file").unwrap();
        assert!(restore(&journal, &[(&a, "a"), (&b, "b")]).is_err());
        assert_eq!(fs::read(&b).unwrap(), b"user edited the newly created file");
        fs::write(&b, b"new").unwrap();
        restore(&journal, &[(&a, "a"), (&b, "b")]).unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"old"); assert!(!b.exists()); assert!(!journal.exists());
    }

    #[test]
    fn unchanged_legacy_journal_is_safe_to_complete() {
        let dir = Directory::new(); let journal = dir.0.join("journal"); fs::create_dir(&journal).unwrap();
        let a = dir.0.join("a"); fs::write(&a, b"old").unwrap(); fs::write(journal.join("a"), b"old").unwrap();
        restore(&journal, &[(&a, "a")]).unwrap();
        assert_eq!(fs::read(a).unwrap(), b"old"); assert!(!journal.exists());
    }
}
