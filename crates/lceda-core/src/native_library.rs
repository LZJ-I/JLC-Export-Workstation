//! Inspect and copy existing native EDA libraries without supplier lookups.
use crate::{altium::sch_preview::SchPreview, Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LibraryFormat { Ad, KiCad }
impl LibraryFormat {
    pub const ALL: [Self; 2] = [Self::Ad, Self::KiCad];
    pub fn name(self) -> &'static str { match self { Self::Ad => "AD", Self::KiCad => "KiCad" } }
    pub fn extension(self) -> &'static str { match self { Self::Ad => "SchLib", Self::KiCad => "kicad_sym" } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePart {
    /// Actual OLE storage key / top-level KiCad symbol name, not a supplier ID.
    pub key: String,
    pub name: String,
    pub description: String,
    pub properties: BTreeMap<String, String>,
    pub footprints: Vec<String>,
    pub pins: usize,
    pub units: usize,
}

#[derive(Debug, Clone)]
pub struct NativeLibrary { pub source: PathBuf, pub format: LibraryFormat, pub parts: Vec<NativePart>, pub content_hash: u64 }

pub fn content_hash(bytes:&[u8])->u64 { bytes.iter().fold(0xcbf29ce484222325,|h,b|(h^u64::from(*b)).wrapping_mul(0x100000001b3)) }

pub fn inspect(path: &Path, format: LibraryFormat) -> Result<NativeLibrary> {
    if !path.is_file() || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(format.extension())) {
        return Err(Error::msg(format!("请选择已有的 .{} 库文件", format.extension())));
    }
    if fs::metadata(path)?.len()>256*1024*1024 { return Err(Error::msg("库文件超过 256 MiB")); }
    let content_hash=content_hash(&fs::read(path)?);
    let parts = match format {
        LibraryFormat::Ad => crate::bound_ad::inspect_library(path)?,
        LibraryFormat::KiCad => crate::bound_kicad::inspect_library(path)?,
    };
    if parts.is_empty() { return Err(Error::msg("库文件中没有可导入的器件")); }
    Ok(NativeLibrary { source: path.to_path_buf(), format, parts, content_hash })
}

pub fn preview(path: &Path, format: LibraryFormat, key: &str, unit: usize) -> Result<SchPreview> {
    match format {
        LibraryFormat::Ad => crate::bound_ad::preview_component(path, key, unit),
        LibraryFormat::KiCad => crate::bound_kicad::preview_component(path, key, unit),
    }
}

/// New-file export only. Callers choose a fresh directory; never overwrite a source.
pub fn export_subset(source: &Path, format: LibraryFormat, keys: &[String], destination: &Path) -> Result<()> {
    if keys.is_empty() { return Err(Error::msg("请选择要导出的器件")); }
    if destination.exists() { return Err(Error::msg("导出文件已存在，请选择新目录")); }
    match format {
        LibraryFormat::Ad => crate::bound_ad::export_subset(source, keys, destination),
        LibraryFormat::KiCad => crate::bound_kicad::export_subset(source, keys, destination),
    }
}

/// Keep paired native files with the imported copy. Symlinks are rejected and
/// total size is bounded; an incomplete copy is not added to the user's library.
pub fn copy_companions(source: &Path, destination: &Path, format: LibraryFormat) -> Result<()> {
    match format {
        LibraryFormat::Ad => {
            let paired = source.with_extension("PcbLib");
            if paired.is_file() { copy_file(&paired, &destination.with_extension("PcbLib"))?; }
        }
        LibraryFormat::KiCad => {
            let from = crate::bound_kicad::targets(source)?;
            let to = crate::bound_kicad::targets(destination)?;
            let mut budget = 512 * 1024 * 1024u64;
            for (a,b) in [(&from.pretty,&to.pretty),(&from.shapes,&to.shapes)] {
                if a.exists() { copy_directory(a,b,0,&mut budget)?; }
            }
        }
    }
    Ok(())
}
pub fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    use std::io::Write;
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 256 * 1024 * 1024 {
        return Err(Error::msg("库资源不是普通文件或超过 256 MiB"));
    }
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(destination)?;
    if let Err(error) = std::io::copy(&mut input, &mut output).and_then(|_| output.flush()).and_then(|_| output.sync_all()) {
        drop(output); let _ = fs::remove_file(destination); return Err(error.into());
    }
    Ok(())
}
fn copy_directory(source: &Path, destination: &Path, depth: usize, budget: &mut u64) -> Result<()> {
    let meta = fs::symlink_metadata(source)?;
    if depth > 16 || !meta.is_dir() || meta.file_type().is_symlink() { return Err(Error::msg("配套资源目录无效或嵌套过深")); }
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?; let path = entry.path(); let target = destination.join(entry.file_name());
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() { return Err(Error::msg("配套资源含符号链接，请先复制为普通文件")); }
        if meta.is_dir() { copy_directory(&path,&target,depth+1,budget)?; }
        else { *budget = budget.checked_sub(meta.len()).ok_or_else(|| Error::msg("配套资源合计超过 512 MiB"))?; copy_file(&path,&target)?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {fn new()->Self{let path=std::env::temp_dir().join(format!("native-library-{}",crate::util::unique_id()));fs::create_dir(&path).unwrap();Self(path)}}
    impl Drop for Directory {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    const KICAD:&str=r#"(kicad_symbol_lib (version 20241209) (generator "test")
      (symbol "Base" (property "Reference" "U" (at 0 4 0) (effects (font (size 1.27 1.27))))
        (property "Footprint" "Library:QFN" (at 0 0 0) (effects (font (size 1.27 1.27)) (hide yes)))
        (symbol "Base_1_1" (rectangle (start -2 -2) (end 2 2) (stroke (width 0.254)))
          (pin input line (at -4 0 0) (length 2) (name "IN") (number "1")))
        (symbol "Base_2_1" (circle (center 0 0) (radius 2)) (pin output line (at 4 0 180) (length 2) (name "OUT") (number "2"))))
      (symbol "中文 C2040" (extends "Base") (property "Value" "10kΩ" (at 0 -4 0) (effects (font (size 1.27 1.27))))
        (property "Description" "保留原参数") (unknown_metadata "untouched"))
      (symbol "Other" (property "Value" "Other")))"#;
    #[test]
    fn kicad_import_resolves_inheritance_units_and_exports_dependencies_verbatim(){
        let dir=Directory::new();let source=dir.0.join("Library.kicad_sym");fs::write(&source,KICAD).unwrap();
        let info=inspect(&source,LibraryFormat::KiCad).unwrap();let part=info.parts.iter().find(|p|p.name=="中文 C2040").unwrap();
        assert_eq!(part.units,2);assert_eq!(part.pins,2);assert_eq!(part.description,"保留原参数");assert_eq!(part.footprints,["Library:QFN"]);
        let first=preview(&source,LibraryFormat::KiCad,&part.key,1).unwrap();let second=preview(&source,LibraryFormat::KiCad,&part.key,2).unwrap();
        assert!(first.texts.iter().any(|t|t.text=="10kΩ"));assert!(first.texts.iter().any(|t|t.text=="IN"));
        assert!(!first.texts.iter().any(|t|t.text=="OUT"));assert!(second.texts.iter().any(|t|t.text=="OUT"));assert_eq!(second.ellipses.len(),1);
        let output=dir.0.join("Subset.kicad_sym");export_subset(&source,LibraryFormat::KiCad,&[part.key.clone()],&output).unwrap();
        let saved=inspect(&output,LibraryFormat::KiCad).unwrap();assert_eq!(saved.parts.len(),2);
        assert!(fs::read_to_string(&output).unwrap().contains("(unknown_metadata \"untouched\")"));
        assert_eq!(fs::read_to_string(&source).unwrap(),KICAD);
        assert!(export_subset(&source,LibraryFormat::KiCad,&[part.key.clone()],&source).is_err());
    }
    #[test]
    fn malformed_inheritance_and_missing_selection_fail_before_export(){
        let dir=Directory::new();let source=dir.0.join("Bad.kicad_sym");let output=dir.0.join("Output.kicad_sym");
        for text in ["(kicad_symbol_lib (version 20241209) (symbol \"A\" (extends \"A\")))","(kicad_symbol_lib (version 20241209) (symbol \"A\" (extends \"Missing\")))"]{
            fs::write(&source,text).unwrap();assert!(inspect(&source,LibraryFormat::KiCad).is_err());
            assert!(export_subset(&source,LibraryFormat::KiCad,&["A".into()],&output).is_err());assert!(!output.exists());
        }
        fs::write(&source,KICAD).unwrap();assert!(export_subset(&source,LibraryFormat::KiCad,&["Missing".into()],&output).is_err());assert!(!output.exists());
    }
    #[test]
    fn ad_subset_keeps_storage_mapping_fonts_and_actual_component_records(){
        use crate::ir::{SymbolIr,IrRect};use std::io::Read;
        let dir=Directory::new();let source=dir.0.join("Source.SchLib");let target=dir.0.join("Subset.SchLib");
        let make=|name:&str|SymbolIr{name:name.into(),description:"中文说明".into(),designator:"R?".into(),meta:Default::default(),pins:vec![],rects:vec![IrRect{x1:-1.0,y1:-0.5,x2:1.0,y2:0.5}],polys:vec![],ellipses:vec![]};
        let a=make("中文/器件");let b=make("Keep Other");crate::altium::write_schlib_many(&source,&[&a,&b]).unwrap();
        let original=fs::read(&source).unwrap();let info=inspect(&source,LibraryFormat::Ad).unwrap();let part=info.parts.iter().find(|p|p.name=="中文/器件").unwrap();
        export_subset(&source,LibraryFormat::Ad,&[part.key.clone()],&target).unwrap();let saved=inspect(&target,LibraryFormat::Ad).unwrap();assert_eq!(saved.parts.len(),1);assert_eq!(saved.parts[0].name,a.name);
        preview(&target,LibraryFormat::Ad,&part.key,1).unwrap();
        let data=|path:&Path|{let mut doc=cfb::open(path).unwrap();let mut bytes=Vec::new();doc.open_stream(format!("/{}/Data",part.key)).unwrap().read_to_end(&mut bytes).unwrap();bytes};
        assert_eq!(data(&source),data(&target));assert_eq!(fs::read(&source).unwrap(),original);
    }
}
