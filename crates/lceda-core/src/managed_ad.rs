//! A stable pair of AD libraries owned by this application. Existing user libraries
//! are never parsed or overwritten. The source catalogue allows offline rebuilds.
use crate::altium::{self, SchColors};
use crate::ir::{FootprintIr, SymbolIr};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const FILES: [&str; 3] = ["JLC.SchLib", "JLC.PcbLib", "JLC.catalog.json"];

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub symbol: Option<SymbolIr>,
    pub footprint: Option<FootprintIr>,
    pub step: Option<Vec<u8>>,
}

#[derive(Default, Serialize, Deserialize)]
struct Catalogue {
    version: u32,
    entries: Vec<Entry>,
    // Detect edits made outside this application instead of silently discarding them.
    sch_hash: Option<u64>,
    pcb_hash: Option<u64>,
}

pub fn update(dir: &Path, entry: Entry, colors: SchColors) -> Result<(PathBuf, PathBuf)> {
    if entry.id.is_empty() || (entry.symbol.is_none() && entry.footprint.is_none()) {
        return Err(Error::msg("固定 AD 库：器件没有可写入的符号或封装"));
    }
    fs::create_dir_all(dir)?;
    let _lock = crate::library_lock::acquire(&dir.join("JLC.lock"))?;
    // A transaction journal is retained through process crashes. Restore the last
    // complete generation before reading its catalogue.
    let pending = dir.join(".JLC-transaction");
    if pending.exists() {
        recover(dir, &pending)?;
    }
    let catalog_path = dir.join(FILES[2]);
    let mut catalog: Catalogue = if catalog_path.exists() {
        let c: Catalogue = serde_json::from_slice(&fs::read(&catalog_path)?)?;
        if c.version != 1 { return Err(Error::msg("不支持的固定 AD 库目录版本")); }
        verify(dir, FILES[0], c.sch_hash)?;
        verify(dir, FILES[1], c.pcb_hash)?;
        c
    } else {
        if dir.join(FILES[0]).exists() || dir.join(FILES[1]).exists() {
            return Err(Error::msg("目标目录已有非本软件管理的 JLC 库，请选择新的专用目录"));
        }
        Catalogue { version: 1, ..Default::default() }
    };
    let original_hashes = (catalog.sch_hash, catalog.pcb_hash);
    if let Some(old) = catalog.entries.iter_mut().find(|e| e.id == entry.id) {
        *old = entry;
    } else {
        catalog.entries.push(entry);
    }
    catalog.entries.sort_by(|a, b| a.id.cmp(&b.id));
    // Stable supplier IDs keep identically named generic footprints from colliding.
    for e in &mut catalog.entries {
        if let Some(fp) = &mut e.footprint {
            fp.name = e.id.clone();
        }
        if let Some(sym) = &mut e.symbol {
            sym.name = e.id.clone();
            if e.footprint.is_some() { sym.meta.footprint_lib = e.id.clone(); }
        }
    }
    let staged = dir.join(".JLC-staged");
    if staged.exists() { fs::remove_dir_all(&staged)?; }
    fs::create_dir(&staged)?;
    let result = (|| -> Result<()> {
        let symbols: Vec<_> = catalog.entries.iter().filter_map(|e| e.symbol.as_ref()).collect();
        let footprints: Vec<_> = catalog.entries.iter().filter_map(|e| e.footprint.as_ref().map(|fp|
            altium::pcblib::PcbLibPart { fp, step: e.step.as_deref() })).collect();
        if !symbols.is_empty() {
            altium::write_schlib_many_with_colors(&staged.join(FILES[0]), &symbols, colors)?;
        }
        if !footprints.is_empty() {
            altium::write_pcblib_library(&staged.join(FILES[1]), &footprints)?;
        }
        for name in &FILES[..2] {
            if staged.join(name).exists() {
                // Windows FlushFileBuffers requires a writable handle.
                OpenOptions::new().read(true).write(true).open(staged.join(name))?.sync_all()?;
            }
        }
        verify(dir, FILES[0], original_hashes.0)?;
        verify(dir, FILES[1], original_hashes.1)?;
        catalog.sch_hash = file_hash(&staged.join(FILES[0]))?;
        catalog.pcb_hash = file_hash(&staged.join(FILES[1]))?;
        let mut f = fs::File::create(staged.join(FILES[2]))?;
        f.write_all(&serde_json::to_vec(&catalog)?)?;
        f.sync_all()?;
        // Make the backup complete before exposing a recovery journal.
        let backup = dir.join(".JLC-backup-next");
        if backup.exists() { fs::remove_dir_all(&backup)?; }
        fs::create_dir(&backup)?;
        for name in FILES {
            if dir.join(name).exists() { fs::copy(dir.join(name), backup.join(name))?; }
        }
        let generations = FILES.iter().map(|name| Ok((*name,
            crate::ad_recovery::read_optional(&backup.join(name))?,
            crate::ad_recovery::read_optional(&staged.join(name))?,
        ))).collect::<Result<Vec<_>>>()?;
        let states: Vec<_> = generations.iter().map(|(name, old, new)| (*name, old.as_deref(), new.as_deref())).collect();
        crate::ad_recovery::record(&backup, &states)?;
        fs::rename(&backup, &pending)?;
        for name in FILES {
            let dest = dir.join(name);
            if staged.join(name).exists() { fs::rename(staged.join(name), dest)?; }
            else if dest.exists() { fs::remove_file(&dest)?; }
        }
        // Removing the journal is the commit. The prior generation remains available.
        let previous = dir.join(".JLC-previous");
        if previous.exists() { fs::remove_dir_all(&previous)?; }
        fs::rename(&pending, &previous)?;
        Ok(())
    })();
    if result.is_err() && pending.exists() {
        if let Err(e) = recover(dir, &pending) {
            return Err(Error::msg(format!("更新失败且旧库恢复失败：{e}。请关闭 AD 后重试；备份保存在 {}", pending.display())));
        }
    }
    let _ = fs::remove_dir_all(&staged);
    result?;
    Ok((dir.join(FILES[0]), dir.join(FILES[1])))
}

fn recover(dir: &Path, backup: &Path) -> Result<()> {
    let paths: Vec<_> = FILES.iter().map(|name| dir.join(name)).collect();
    let files: Vec<_> = paths.iter().zip(FILES).map(|(path, name)| (path.as_path(), name)).collect();
    crate::ad_recovery::restore(backup, &files)
}

fn file_hash(path: &Path) -> Result<Option<u64>> {
    if !path.exists() { return Ok(None); }
    // Stable FNV-1a change detector, not a security signature.
    let mut hash = 0xcbf29ce484222325u64;
    for b in fs::read(path)? { hash = (hash ^ b as u64).wrapping_mul(0x100000001b3); }
    Ok(Some(hash))
}

fn verify(dir: &Path, name: &str, expected: Option<u64>) -> Result<()> {
    if file_hash(&dir.join(name))? != expected {
        return Err(Error::msg(format!("{name} 已被外部修改或删除；为保留修改，请选择新的固定库目录")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("jlc-managed-{}-{}", std::process::id(), crate::util::unique_id()));
        fs::create_dir_all(&p).unwrap(); p
    }
    fn entry(id: &str) -> Entry {
        Entry { id: id.into(), symbol: Some(SymbolIr {
            name: "same-name".into(), description: "part".into(), designator: "R?".into(),
            meta: Default::default(), pins: vec![], rects: vec![], polys: vec![], ellipses: vec![],
        }), footprint: None, step: None }
    }
    #[test]
    fn adds_updates_and_protects_external_edits() {
        let p = dir();
        update(&p, entry("C1"), SchColors::default()).unwrap();
        update(&p, entry("C2"), SchColors::default()).unwrap();
        update(&p, entry("C1"), SchColors::default()).unwrap();
        let c: Catalogue = serde_json::from_slice(&fs::read(p.join(FILES[2])).unwrap()).unwrap();
        assert_eq!(c.entries.len(), 2);
        let mut cfb = cfb::open(p.join(FILES[0])).unwrap();
        assert!(cfb.open_stream("/C1/Data").is_ok());
        assert!(cfb.open_stream("/C2/Data").is_ok());
        drop(cfb);
        fs::write(p.join(FILES[0]), b"user-edit").unwrap();
        assert!(update(&p, entry("C3"), SchColors::default()).is_err());
        assert_eq!(fs::read(p.join(FILES[0])).unwrap(), b"user-edit");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn refuses_unmanaged_library_and_recovers_interrupted_transaction() {
        let p = dir();
        fs::write(p.join(FILES[0]), b"existing").unwrap();
        assert!(update(&p, entry("C1"), SchColors::default()).is_err());
        fs::remove_file(p.join(FILES[0])).unwrap();
        update(&p, entry("C1"), SchColors::default()).unwrap();
        update(&p, entry("C3"), SchColors::default()).unwrap();
        let journal = p.join(".JLC-transaction");
        fs::rename(p.join(".JLC-previous"), &journal).unwrap();
        fs::copy(journal.join(FILES[2]), p.join(FILES[2])).unwrap();
        update(&p, entry("C2"), SchColors::default()).unwrap();
        let c: Catalogue = serde_json::from_slice(&fs::read(p.join(FILES[2])).unwrap()).unwrap();
        assert_eq!(c.entries.len(), 2);
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn recovery_does_not_replace_edits_saved_after_interruption() {
        let p = dir();
        update(&p, entry("C1"), SchColors::default()).unwrap();
        update(&p, entry("C2"), SchColors::default()).unwrap();
        let journal = p.join(".JLC-transaction");
        fs::rename(p.join(".JLC-previous"), &journal).unwrap();
        fs::write(p.join(FILES[0]), b"manual edit after interruption").unwrap();
        let catalogue = fs::read(p.join(FILES[2])).unwrap();
        assert!(update(&p, entry("C3"), SchColors::default()).is_err());
        assert_eq!(fs::read(p.join(FILES[0])).unwrap(), b"manual edit after interruption");
        assert_eq!(fs::read(p.join(FILES[2])).unwrap(), catalogue);
        assert!(journal.is_dir());
        fs::remove_dir_all(p).unwrap();
    }
}
