//! Incremental KiCad libraries. Retain unowned S-expressions verbatim and replace
//! only entries whose previous bytes match our ownership manifest. A durable undo
//! journal covers the symbol, footprint, model and ownership file together.
use crate::{kicad, managed_ad::Entry, models::looks_like_lcsc_token, Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashSet}, fs::{self, File, OpenOptions}, io::{Read, Write}, ops::Range, path::{Component, Path, PathBuf}};

const MAX_FILE: u64 = 64 * 1024 * 1024;
const VERSION: u64 = 20241209;
fn invalid(message: impl std::fmt::Display) -> Error {
    Error::msg(format!("KiCad 绑定库同步失败：{message}。请检查库文件或关闭占用它的程序后重试。"))
}

#[derive(Debug)]
struct Node { span: Range<usize>, atom: Option<String>, children: Vec<Node> }
impl Node {
    fn text(&self) -> Option<&str> { self.atom.as_deref() }
    fn head(&self) -> Option<&str> { self.children.first()?.text() }
    fn arg(&self) -> Option<&str> { self.children.get(1)?.text() }
}
struct Parser<'a> { bytes: &'a [u8], pos: usize, nodes: usize }
impl Parser<'_> {
    fn space(&mut self) {
        loop {
            while self.bytes.get(self.pos).is_some_and(u8::is_ascii_whitespace) { self.pos += 1; }
            if self.bytes.get(self.pos) == Some(&b';') {
                while self.bytes.get(self.pos).is_some_and(|b| *b != b'\n') { self.pos += 1; }
            } else { break; }
        }
    }
    fn node(&mut self, depth: usize) -> Result<Node> {
        self.space();
        self.nodes += 1;
        if depth > 128 || self.nodes > 1_000_000 { return Err(invalid("S 表达式嵌套或节点过多")); }
        let start = self.pos;
        let first = *self.bytes.get(self.pos).ok_or_else(|| invalid("S 表达式意外结束"))?;
        self.pos += 1;
        if first == b'(' {
            let mut children = Vec::new();
            loop {
                self.space();
                if self.bytes.get(self.pos) == Some(&b')') { self.pos += 1; break; }
                children.push(self.node(depth + 1)?);
            }
            if children.is_empty() { return Err(invalid("空 S 表达式")); }
            return Ok(Node { span: start..self.pos, atom: None, children });
        }
        if first == b')' { return Err(invalid("多余的右括号")); }
        let value = if first == b'"' {
            let mut value = Vec::new();
            loop {
                let byte = *self.bytes.get(self.pos).ok_or_else(|| invalid("字符串缺少结束引号"))?;
                self.pos += 1;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        let escaped = *self.bytes.get(self.pos).ok_or_else(|| invalid("字符串转义不完整"))?;
                        self.pos += 1;
                        value.push(match escaped { b'n' => b'\n', b'r' => b'\r', b't' => b'\t', other => other });
                    }
                    0 => return Err(invalid("字符串含 NUL")),
                    byte => value.push(byte),
                }
            }
            String::from_utf8(value).map_err(invalid)?
        } else {
            while self.bytes.get(self.pos).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(*b, b'(' | b')' | b';')) { self.pos += 1; }
            std::str::from_utf8(&self.bytes[start..self.pos]).map_err(invalid)?.to_string()
        };
        Ok(Node { span: start..self.pos, atom: Some(value), children: Vec::new() })
    }
}
fn parse(text: &str) -> Result<Node> {
    if text.len() as u64 > MAX_FILE || text.as_bytes().contains(&0) { return Err(invalid("文件过大或含 NUL")); }
    let mut parser = Parser { bytes: text.as_bytes(), pos: 0, nodes: 0 };
    // Accept UTF-8 BOM without changing the existing file's bytes.
    if parser.bytes.starts_with(&[0xef, 0xbb, 0xbf]) { parser.pos = 3; }
    let root = parser.node(0)?;
    parser.space();
    if parser.pos != text.len() { return Err(invalid("根节点后有多余数据")); }
    Ok(root)
}
fn library(text: &str) -> Result<Node> {
    let root = parse(text)?;
    if root.head() != Some("kicad_symbol_lib") { return Err(invalid("请选择 .kicad_sym 符号库")); }
    let versions: Vec<_> = root.children.iter().filter(|node| node.head() == Some("version")).collect();
    if versions.len() != 1 || versions[0].arg().and_then(|s| s.parse::<u64>().ok()).is_none_or(|v| v < 20211014) {
        return Err(invalid("不支持的符号库版本，至少需要 KiCad 6 格式"));
    }
    let mut names = HashSet::new();
    for node in root.children.iter().filter(|node| node.head() == Some("symbol")) {
        let name = node.arg().filter(|name| !name.is_empty()).ok_or_else(|| invalid("符号缺少名称"))?;
        if !names.insert(name.to_lowercase()) { return Err(invalid(format!("符号名称重复：{name}"))); }
    }
    Ok(root)
}

#[derive(Clone)]
pub struct Targets { pub symbol: PathBuf, pub pretty: PathBuf, pub shapes: PathBuf }
pub fn targets(path: &Path) -> Result<Targets> {
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("kicad_sym")) { return Err(invalid("请选择 .kicad_sym 文件")); }
    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty() && !s.contains(':')).ok_or_else(|| invalid("库文件名称无效"))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(Targets { symbol: path.to_path_buf(), pretty: parent.join(format!("{stem}.pretty")), shapes: parent.join(format!("{stem}.3dshapes")) })
}
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!(".{}.jlc-{suffix}", path.file_name().unwrap().to_string_lossy()))
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_FILE { return Err(invalid(format!("文件不是普通文件或超过 64 MiB：{}", path.display()))); }
            Ok(Some(fs::read(path)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(invalid(e)),
    }
}
pub fn validate_binding(path: &Path) -> Result<()> {
    let paths = targets(path)?;
    if let Some(bytes) = read_optional(path)? { library(std::str::from_utf8(&bytes).map_err(invalid)?)?; }
    for directory in [&paths.pretty, &paths.shapes] {
        if let Ok(meta) = fs::symlink_metadata(directory) {
            if !meta.is_dir() || meta.file_type().is_symlink() { return Err(invalid(format!("配套路径不是普通目录：{}", directory.display()))); }
        }
    }
    if let Some(bytes) = read_optional(&sidecar(path, "meta.json"))? {
        let meta: Ownership = serde_json::from_slice(&bytes).map_err(invalid)?;
        if meta.version != 1 { return Err(invalid("不支持的器件归属记录版本")); }
    }
    Ok(())
}
fn hash(bytes: &[u8]) -> u64 { bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3)) }
#[derive(Default, Serialize, Deserialize)]
struct Part { symbol: Option<u64>, footprint: Option<u64>, step: Option<u64> }
#[derive(Serialize, Deserialize)]
struct Ownership { version: u32, parts: BTreeMap<String, Part> }
impl Default for Ownership { fn default() -> Self { Self { version: 1, parts: BTreeMap::new() } } }

fn merge_symbol(text: &str, id: &str, fresh: &str, expected: Option<u64>) -> Result<String> {
    let root = library(text)?;
    let old = root.children.iter().find(|node| node.head() == Some("symbol") && node.arg().is_some_and(|s| s.eq_ignore_ascii_case(id)));
    if let Some(old) = old {
        if expected != Some(hash(text[old.span.clone()].as_bytes())) { return Err(invalid(format!("符号 {id} 已存在且不是本工具管理的版本，或已被手工修改；不会覆盖"))); }
    } else if expected.is_some() { return Err(invalid(format!("符号 {id} 已从库中移除；请恢复器件后重试"))); }
    let generated = parse(fresh.trim())?;
    let mut edits = Vec::new();
    if let Some(old) = old { edits.push((old.span.clone(), fresh.trim().to_string())); }
    else { edits.push((root.span.end - 1..root.span.end - 1, format!("  {}\n", fresh.trim()))); }
    let version = root.children.iter().find(|node| node.head() == Some("version")).unwrap();
    if version.arg().unwrap().parse::<u64>().unwrap() < VERSION {
        edits.push((version.children[1].span.clone(), VERSION.to_string()));
    }
    if generated.head() != Some("symbol") { return Err(invalid("生成的符号格式无效")); }
    edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut result = text.to_string();
    for (span, replacement) in edits { result.replace_range(span, &replacement); }
    library(&result)?;
    Ok(result)
}

struct Change { path: PathBuf, old: Option<Vec<u8>>, new: Vec<u8>, _handle: Option<File> }
fn change(path: PathBuf, new: Vec<u8>, expected: Option<u64>) -> Result<Change> {
    let old = read_optional(&path)?;
    if old.as_deref().map(hash) != expected { return Err(invalid(format!("{} 已存在但归属不符，或已被手工修改；不会覆盖", path.display()))); }
    let handle = if old.is_some() {
        if fs::metadata(&path)?.permissions().readonly() { return Err(invalid(format!("文件只读：{}", path.display()))); }
        let mut options = OpenOptions::new(); options.read(true).write(true);
        #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.share_mode(1 | 4); }
        let mut file = options.open(&path).map_err(invalid)?;
        let mut locked = Vec::new(); file.read_to_end(&mut locked)?;
        if Some(&locked) != old.as_ref() { return Err(invalid("文件在准备同步时被其它程序修改")); }
        Some(file)
    } else { None };
    Ok(Change { path, old, new, _handle: handle })
}
fn persist(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(path)?; file.write_all(bytes)?; file.sync_all()?; Ok(())
}
#[derive(Serialize, Deserialize)]
struct Undo { relative: PathBuf, old: Option<u64>, new: u64 }
fn allowed_relative(path: &Path, symbol: &Path) -> bool {
    if path.is_absolute() || path.components().any(|c| !matches!(c, Component::Normal(_))) { return false; }
    let paths = targets(symbol).unwrap();
    if path == Path::new(symbol.file_name().unwrap()) || path == Path::new(sidecar(symbol, "meta.json").file_name().unwrap()) { return true; }
    let Some(parent) = path.parent() else { return false; };
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { return false; };
    looks_like_lcsc_token(stem) && ((parent == Path::new(paths.pretty.file_name().unwrap()) && path.extension().is_some_and(|s| s == "kicad_mod")) ||
        (parent == Path::new(paths.shapes.file_name().unwrap()) && path.extension().is_some_and(|s| s == "step")))
}
fn recover(journal: &Path, symbol: &Path) -> Result<()> {
    let parent = symbol.parent().unwrap();
    let undo: Vec<Undo> = serde_json::from_slice(&fs::read(journal.join("manifest.json"))?).map_err(invalid)?;
    if undo.is_empty() || undo.len() > 4 { return Err(invalid("事务恢复记录无效")); }
    let mut ready = Vec::new(); let mut unique = HashSet::new();
    // Validate all backups and current bytes before restoring anything.
    for (i, item) in undo.iter().enumerate() {
        if !allowed_relative(&item.relative, symbol) || !unique.insert(item.relative.clone()) { return Err(invalid("事务恢复路径无效")); }
        let path = parent.join(&item.relative);
        let old = if item.old.is_some() { read_optional(&journal.join(format!("{i}.old")))? } else { None };
        if old.as_deref().map(hash) != item.old { return Err(invalid("事务备份不完整或已损坏")); }
        let current = read_optional(&path)?;
        if current.is_some() && current.as_deref().map(hash) != item.old && current.as_deref().map(hash) != Some(item.new) { return Err(invalid(format!("中断后 {} 被其它程序修改，备份保留在 {}", path.display(), journal.display()))); }
        ready.push((path, old));
    }
    for (i, (path, old)) in ready.into_iter().enumerate() {
        if let Some(bytes) = old {
            if read_optional(&path)?.as_ref() != Some(&bytes) {
                let staged = journal.join(format!("{i}.restore")); persist(&staged, &bytes)?;
                replace(&staged, &path)?;
            }
        } else if path.exists() { fs::remove_file(&path)?; }
    }
    fs::remove_dir_all(journal)?; Ok(())
}
fn replace(from: &Path, to: &Path) -> Result<()> {
    #[cfg(windows)] if to.exists() { fs::remove_file(to).map_err(invalid)?; }
    fs::rename(from, to).map_err(invalid)
}
fn transact(symbol: &Path, changes: Vec<Change>, fail_after: Option<usize>) -> Result<()> {
    let parent = symbol.parent().unwrap();
    let stage = sidecar(symbol, &format!("stage-{}", crate::util::unique_id()));
    let journal = sidecar(symbol, "transaction");
    fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        let prepared = stage.join("undo"); fs::create_dir(&prepared)?;
        let mut undo = Vec::new();
        for (i, change) in changes.iter().enumerate() {
            let relative = change.path.strip_prefix(parent).map_err(invalid)?.to_path_buf();
            if !allowed_relative(&relative, symbol) { return Err(invalid("同步目标不在绑定库内")); }
            if let Some(bytes) = &change.old { persist(&prepared.join(format!("{i}.old")), bytes)?; }
            persist(&stage.join(format!("{i}.new")), &change.new)?;
            undo.push(Undo { relative, old: change.old.as_deref().map(hash), new: hash(&change.new) });
        }
        persist(&prepared.join("manifest.json"), &serde_json::to_vec(&undo)?)?;
        for change in &changes {
            if read_optional(&change.path)? != change.old { return Err(invalid("库在同步期间被其它程序修改，已取消写入")); }
            fs::create_dir_all(change.path.parent().unwrap())?;
        }
        fs::rename(&prepared, &journal)?;
        for (i, change) in changes.iter().enumerate() {
            if fail_after == Some(i) { return Err(invalid("模拟写入中断")); }
            replace(&stage.join(format!("{i}.new")), &change.path)?;
        }
        let previous = sidecar(symbol, "backup");
        if previous.exists() { fs::remove_dir_all(&previous)?; }
        fs::rename(&journal, previous)?;
        Ok(())
    })();
    if result.is_err() && journal.exists() {
        if let Err(error) = recover(&journal, symbol) { let _ = fs::remove_dir_all(stage); return Err(error); }
    }
    let _ = fs::remove_dir_all(stage);
    result
}

/// Synchronize one supplier ID; no category removal deletes an external component.
pub fn update(path: &Path, entry: Entry) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    update_inner(path, entry, None)
}
fn update_inner(path: &Path, mut entry: Entry, fail_after: Option<usize>) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    targets(path)?;
    if !looks_like_lcsc_token(&entry.id) || (entry.symbol.is_none() && entry.footprint.is_none()) { return Err(invalid("需要 C 编号及符号或封装")); }
    entry.id.make_ascii_uppercase();
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let parent = absolute.parent().unwrap(); fs::create_dir_all(parent)?;
    let path = fs::canonicalize(parent)?.join(absolute.file_name().unwrap());
    let paths = targets(&path)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(sidecar(&path, "lock")).map_err(invalid)?;
    lock.try_lock().map_err(|_| invalid("此 KiCad 库正在同步，请稍后重试"))?;
    if sidecar(&path, "transaction").exists() { recover(&sidecar(&path, "transaction"), &path)?; }
    validate_binding(&path)?;
    let meta_path = sidecar(&path, "meta.json");
    let meta_bytes = read_optional(&meta_path)?;
    let mut ownership: Ownership = meta_bytes.as_ref().map(|bytes| serde_json::from_slice(bytes)).transpose().map_err(invalid)?.unwrap_or_default();
    let owned = ownership.parts.entry(entry.id.clone()).or_default();
    let mut changes = Vec::new();
    let nickname = path.file_stem().unwrap().to_string_lossy();
    let mut symbol_result = None;
    let mut footprint_result = None;
    if let Some(symbol) = &mut entry.symbol {
        symbol.name = entry.id.clone();
        symbol.meta.lcsc = entry.id.clone();
        if entry.footprint.is_some() { symbol.meta.footprint_lib = format!("{nickname}:{}", entry.id); }
        let old_bytes = read_optional(&path)?;
        let text = if let Some(bytes) = &old_bytes { std::str::from_utf8(bytes).map_err(invalid)?.to_string() } else { kicad::symbol_lib_text_many(&[]) };
        let fresh = kicad::symbol_entry(symbol);
        let merged = merge_symbol(&text, &entry.id, &fresh, owned.symbol)?;
        owned.symbol = Some(hash(fresh.trim().as_bytes()));
        changes.push(change(path.clone(), merged.into_bytes(), old_bytes.as_deref().map(hash))?);
        symbol_result = Some(path.clone());
    }
    if let Some(footprint) = &mut entry.footprint {
        footprint.name = entry.id.clone();
        footprint.meta.lcsc = entry.id.clone();
        let model = if let Some(bytes) = &entry.step {
            if !crate::util::looks_like_step(bytes) { return Err(invalid("STEP 不是有效模型")); }
            let target = paths.shapes.join(format!("{}.step", entry.id));
            changes.push(change(target.clone(), bytes.clone(), owned.step)?);
            owned.step = Some(hash(bytes));
            // A bound library can be installed in any project. Absolute model paths
            // avoid depending on the active project's KIPRJMOD or working directory.
            let model = target.to_string_lossy().replace('\\', "/");
            Some(if let Some(unc) = model.strip_prefix("//?/UNC/") { format!("//{unc}") }
                else { model.trim_start_matches("//?/").to_string() })
        } else { None };
        let generated = kicad::footprint_mod_text(footprint, model.as_deref());
        if parse(&generated)?.head() != Some("footprint") { return Err(invalid("生成的封装格式无效")); }
        let target = paths.pretty.join(format!("{}.kicad_mod", entry.id));
        changes.push(change(target.clone(), generated.as_bytes().to_vec(), owned.footprint)?);
        owned.footprint = Some(hash(generated.as_bytes()));
        footprint_result = Some(target);
    }
    changes.push(change(meta_path, serde_json::to_vec_pretty(&ownership)?, meta_bytes.as_deref().map(hash))?);
    transact(&path, changes, fail_after)?;
    Ok((symbol_result, footprint_result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{easyeda, ir::{self, PartMeta}};
    struct Directory(PathBuf);
    impl Directory { fn new() -> Self { let p = std::env::temp_dir().join(format!("lceda-kicad-{}", crate::util::unique_id())); fs::create_dir_all(&p).unwrap(); Self(p) } }
    impl Drop for Directory { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    fn entry(id: &str, mpn: &str) -> Entry {
        let meta = PartMeta { lcsc: id.into(), mpn: mpn.into(), ..Default::default() };
        Entry { id: id.into(),
            symbol: Some(ir::symbol_ir("generic", "fixture", easyeda::parse_symbol(&serde_json::from_str(include_str!("../tests/fixtures/FNR3015S2R2MT_symbol_easyeda.json")).unwrap()).unwrap(), meta.clone())),
            footprint: Some(ir::footprint_ir("generic", "fixture", easyeda::parse_footprint(&serde_json::from_str(include_str!("../tests/fixtures/FNR3015S2R2MT_footprint_easyeda.json")).unwrap()).unwrap(), meta)),
            step: Some(b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;".to_vec()) }
    }
    fn bytes(path: &Path) -> Vec<u8> { fs::read(path).unwrap() }
    const CUSTOM: &str = "(symbol \"custom (quote \\\" 中文)\" (property \"Value\" \"retain\") (symbol \"custom_0_1\"))";
    #[test]
    fn retains_custom_symbols_unknown_fields_inheritance_and_updates_stable_ids() {
        let d = Directory::new(); let path = d.0.join("用户库.kicad_sym");
        let original = format!("(kicad_symbol_lib (version 20211014) (generator custom)\n ; keep ( comment\n (future_field \"( untouched )\")\n {CUSTOM}\n (symbol \"derived\" (extends \"custom (quote \\\" 中文)\"))\n)\n");
        fs::write(&path, original).unwrap();
        update(&path, entry("C2040", "Same MPN")).unwrap();
        let before = fs::read_to_string(&path).unwrap();
        update(&path, entry("C9999", "Same MPN")).unwrap();
        let mut revised = entry("C2040", "Updated MPN"); revised.symbol.as_mut().unwrap().description = "revised".into();
        update(&path, revised).unwrap();
        let after = fs::read_to_string(&path).unwrap(); let root = library(&after).unwrap();
        let names: Vec<_> = root.children.iter().filter(|n| n.head() == Some("symbol")).map(|n| n.arg().unwrap()).collect();
        assert_eq!(names, ["custom (quote \" 中文)", "derived", "C2040", "C9999"]);
        assert!(after.contains(CUSTOM)); assert!(after.contains("(future_field \"( untouched )\")"));
        assert!(after.contains("(extends \"custom (quote \\\" 中文)\")"));
        assert!(after.contains("(version 20241209)")); assert!(after.contains("\"Value\" \"Updated MPN\""));
        assert!(after.contains("\"Footprint\" \"用户库:C2040\"")); assert_ne!(before, after);
        let fp = fs::read_to_string(d.0.join("用户库.pretty/C2040.kicad_mod")).unwrap();
        assert!(fp.contains("用户库.3dshapes/C2040.step"));
        assert!(sidecar(&path, "backup").join("manifest.json").is_file());
    }
    #[test]
    fn repeat_sync_is_byte_identical_and_no_3d_keeps_unrelated_files() {
        let d = Directory::new(); let path = d.0.join("Parts.kicad_sym");
        update(&path, entry("C2040", "MPN")).unwrap(); let first = bytes(&path);
        let mut e = entry("c2040", "MPN"); update(&path, e.clone()).unwrap(); assert_eq!(first, bytes(&path));
        fs::write(d.0.join("Parts.pretty/user.kicad_mod"), "my manual footprint").unwrap();
        fs::write(d.0.join("Parts.3dshapes/user.step"), "my manual model").unwrap();
        e.step = None; update(&path, e).unwrap();
        assert_eq!(bytes(&d.0.join("Parts.pretty/user.kicad_mod")), b"my manual footprint");
        assert_eq!(bytes(&d.0.join("Parts.3dshapes/user.step")), b"my manual model");
        assert!(!fs::read_to_string(d.0.join("Parts.pretty/C2040.kicad_mod")).unwrap().contains("(model"));
    }
    #[test]
    fn rejects_unowned_and_edited_symbols_and_footprints_before_writing() {
        let d = Directory::new(); let path = d.0.join("Parts.kicad_sym");
        fs::write(&path, "(kicad_symbol_lib (version 20241209) (symbol \"C2040\" (property \"Value\" \"mine\")))").unwrap();
        let original = bytes(&path); assert!(update(&path, entry("C2040", "New")).unwrap_err().to_string().contains("不会覆盖")); assert_eq!(original, bytes(&path));
        fs::remove_file(&path).unwrap(); update(&path, entry("C2040", "New")).unwrap();
        let edited = fs::read_to_string(&path).unwrap().replace("\"Value\" \"New\"", "\"Value\" \"hand edited\""); fs::write(&path, &edited).unwrap();
        assert!(update(&path, entry("C2040", "Another")).is_err()); assert_eq!(bytes(&path), edited.as_bytes());
        let fp = d.0.join("Parts.pretty/C9999.kicad_mod"); fs::write(&fp, "manual conflict").unwrap();
        assert!(update(&path, entry("C9999", "MPN")).is_err()); assert_eq!(bytes(&path), edited.as_bytes()); assert_eq!(bytes(&fp), b"manual conflict");
    }
    #[test]
    fn rollback_restores_all_files_after_partial_replacement() {
        let d = Directory::new(); let path = d.0.join("Parts.kicad_sym");
        update(&path, entry("C2040", "Original")).unwrap();
        let paths = [&path, &d.0.join("Parts.pretty/C2040.kicad_mod"), &d.0.join("Parts.3dshapes/C2040.step"), &sidecar(&path, "meta.json")];
        let original: Vec<_> = paths.iter().map(|p| bytes(p)).collect();
        for count in 0..4 {
            assert!(update_inner(&path, entry("C2040", "Revised"), Some(count)).is_err());
            for (p, expected) in paths.iter().zip(&original) { assert_eq!(&bytes(p), expected); }
            assert!(!sidecar(&path, "transaction").exists());
        }
        assert!(update_inner(&path, entry("C9999", "New"), Some(2)).is_err());
        assert!(!d.0.join("Parts.pretty/C9999.kicad_mod").exists()); assert!(!d.0.join("Parts.3dshapes/C9999.step").exists());
        assert_eq!(bytes(&path), original[0]);
    }
    #[test]
    fn rejects_corruption_invalid_paths_busy_and_readonly_files() {
        let d = Directory::new(); let path = d.0.join("Parts.kicad_sym");
        for text in ["(kicad_symbol_lib (version 20241209)", "(other (version 20241209))", "(kicad_symbol_lib (version 20241209) (symbol \"A\") (symbol \"a\"))"] {
            fs::write(&path, text).unwrap(); assert!(update(&path, entry("C2040", "MPN")).is_err()); assert_eq!(bytes(&path), text.as_bytes());
        }
        fs::remove_file(&path).unwrap(); update(&path, entry("C2040", "MPN")).unwrap(); let original = bytes(&path);
        let lock = OpenOptions::new().read(true).write(true).open(sidecar(&path, "lock")).unwrap(); lock.try_lock().unwrap();
        assert!(update(&path, entry("C9999", "MPN")).is_err()); drop(lock);
        let mut permissions = fs::metadata(&path).unwrap().permissions(); let restore = permissions.clone(); permissions.set_readonly(true); fs::set_permissions(&path, permissions).unwrap();
        assert!(update(&path, entry("C9999", "MPN")).is_err()); fs::set_permissions(&path, restore).unwrap(); assert_eq!(original, bytes(&path));
        assert!(update(&path, entry("C../bad", "MPN")).is_err());
        assert!(!allowed_relative(Path::new("../elsewhere"), &path));
    }
    #[test]
    fn resumes_a_preexisting_journal_and_checks_backup_integrity() {
        let d = Directory::new(); let path = d.0.join("Parts.kicad_sym");
        update(&path, entry("C2040", "Original")).unwrap(); let original = bytes(&path);
        update(&path, entry("C2040", "Revised")).unwrap();
        fs::rename(sidecar(&path, "backup"), sidecar(&path, "transaction")).unwrap();
        recover(&sidecar(&path, "transaction"), &path).unwrap(); assert_eq!(original, bytes(&path));
        update(&path, entry("C9999", "Revised")).unwrap();
        fs::rename(sidecar(&path, "backup"), sidecar(&path, "transaction")).unwrap();
        fs::write(sidecar(&path, "transaction").join("0.old"), "corrupt").unwrap(); let before = bytes(&path);
        assert!(recover(&sidecar(&path, "transaction"), &path).is_err()); assert_eq!(before, bytes(&path));
    }
}
