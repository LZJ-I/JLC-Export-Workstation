//! Optional resources for imported parts. Never edits a native library or its links.
use crate::{altium, client::LcedaClient, easyeda, ir::{self, FootprintIr}, kicad,
    models::SearchItem, native_library::{self, LibraryFormat, NativePart}, Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Write, path::{Component, Path, PathBuf}};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceKinds { pub footprint: bool, pub model3d: bool }
impl ResourceKinds {
    pub fn any(self) -> bool { self.footprint || self.model3d }
    pub fn available(self, item: &SearchItem) -> bool {
        self.any() && (!self.footprint || item.footprint_uuid().is_some()) && (!self.model3d || item.model_uuid.is_some())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBundle {
    pub directory: PathBuf,
    pub native_key: String,
    pub native_name: String,
    pub original_footprints: Vec<String>,
    pub supplier_id: String,
    pub supplier_name: String,
    pub manufacturer: String,
    pub format: LibraryFormat,
    pub kinds: ResourceKinds,
    /// Relative, regular files only. The original symbol is never part of this set.
    pub files: BTreeMap<String, PathBuf>,
    pub hashes: BTreeMap<String, u64>,
}
impl ResourceBundle {
    pub fn path(&self, kind: &str) -> Option<PathBuf> { self.files.get(kind).filter(|p| safe_relative(p)).map(|p| self.directory.join(p)) }
}
fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?; file.sync_all()?; Ok(())
}

pub fn fetch_footprint(client: &LcedaClient, item: &SearchItem) -> Result<(serde_json::Value, FootprintIr)> {
    let uuid = item.footprint_uuid().ok_or_else(|| Error::msg("候选器件没有封装资源"))?;
    let json = client.component_json(&uuid)?;
    let source = easyeda::parse_footprint(&json)?;
    let footprint = ir::footprint_ir(item.name(), "", source, item.meta());
    if footprint.pads.is_empty() && footprint.tracks.is_empty() && footprint.circles.is_empty() && footprint.arcs.is_empty() && footprint.regions.is_empty() {
        return Err(Error::msg("候选封装没有可用图形，请选择其它候选"));
    }
    Ok((json, footprint))
}

/// One independent directory per part: downloads can run in parallel without shared writes.
pub fn download(client: &LcedaClient, part: &NativePart, format: LibraryFormat, item: &SearchItem,
    kinds: ResourceKinds, parent: &Path) -> Result<ResourceBundle> {
    if !kinds.available(item) { return Err(Error::msg("请选择候选器件具备的封装 / 3D 资源")); }
    let footprint = if kinds.footprint { Some(fetch_footprint(client, item)?) } else { None };
    let step = if kinds.model3d {
        let bytes = client.download_step_bytes(item).map_err(|e| Error::msg(format!("此候选的 STEP 下载失败，请重试或选择其他候选；原库及已有资源未改动。{e}")))?;
        if !crate::util::looks_like_step(&bytes) { return Err(Error::msg("下载的 STEP 不是有效模型，已保留原有资源")); }
        Some(bytes)
    } else { None };
    // OBJ is a preview aid. A missing OBJ must not discard a valid STEP download.
    let obj = if kinds.model3d { client.download_obj_bytes(item).ok() } else { None };
    write_bundle(part, format, item, kinds, parent, footprint, step, obj)
}

fn write_bundle(part: &NativePart, format: LibraryFormat, item: &SearchItem, kinds: ResourceKinds,
    parent: &Path, footprint: Option<(serde_json::Value, FootprintIr)>, step: Option<Vec<u8>>, obj: Option<Vec<u8>>,
) -> Result<ResourceBundle> {
    let supplier_id = item.lcsc_id().ok_or_else(|| Error::msg("候选器件缺少明确的立创编号"))?;
    if !kinds.any() || kinds.footprint != footprint.is_some() || kinds.model3d != step.is_some() {
        return Err(Error::msg("补充资源不完整，原库保持不变"));
    }
    fs::create_dir_all(parent)?;
    let directory = parent.join(crate::util::unique_id()); fs::create_dir(&directory)?;
    let result = (|| {
        let name = format!("JLC_{}_{}", supplier_id, crate::util::unique_id());
        let name: String = name.chars().take(30).collect();
        let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
        if let Some(bytes) = &step {
            let path = PathBuf::from(format!("{name}.3dshapes/{name}.step"));
            write_new(&directory.join(&path), bytes)?; files.insert("step".into(), path);
        }
        if let Some(bytes) = obj.filter(|b| !b.is_empty()) {
            let path = PathBuf::from("preview.obj"); write_new(&directory.join(&path), &bytes)?; files.insert("obj".into(), path);
        }
        if let Some((json, mut footprint)) = footprint {
            footprint.name = name.clone();
            let path = PathBuf::from("footprint_easyeda.json"); write_new(&directory.join(&path), &serde_json::to_vec_pretty(&json)?)?; files.insert("footprint_json".into(), path);
            let path = PathBuf::from("footprint_ir.json"); write_new(&directory.join(&path), &serde_json::to_vec_pretty(&footprint)?)?; files.insert("footprint_ir".into(), path);
            match format {
                LibraryFormat::Ad => {
                    let path = PathBuf::from(format!("{name}.PcbLib"));
                    altium::write_pcblib_with_step(&directory.join(&path), &footprint, step.as_deref())?;
                    files.insert("pcblib".into(), path);
                }
                LibraryFormat::KiCad => {
                    let path = PathBuf::from(format!("{name}.pretty/{name}.kicad_mod"));
                    let model = files.get("step").map(|p| directory.join(p).to_string_lossy().replace('\\', "/"));
                    kicad::write_footprint_mod(&directory.join(&path), &footprint, model.as_deref())?;
                    files.insert("kicad_mod".into(), path);
                }
            }
        }
        let hashes = files.iter().map(|(kind, path)| Ok((kind.clone(), native_library::content_hash(&fs::read(directory.join(path))?)))).collect::<Result<_>>()?;
        let bundle = ResourceBundle { directory: directory.clone(), native_key: part.key.clone(), native_name: part.name.clone(), original_footprints: part.footprints.clone(),
            supplier_id, supplier_name: item.name().to_string(), manufacturer: item.manufacturer_label(), format, kinds, files, hashes };
        write_new(&directory.join("resources.json"), &manifest(&bundle)?)?;
        Ok(bundle)
    })();
    if result.is_err() { let _ = fs::remove_dir_all(&directory); }
    result
}
fn manifest(bundle: &ResourceBundle) -> Result<Vec<u8>> {
    // Do not leak a profile-specific directory into the portable association record.
    let mut value = serde_json::to_value(bundle)?; value.as_object_mut().unwrap().remove("directory");
    value["mode"] = serde_json::json!("supplementary_resources_only");
    value["original_bindings_changed"] = serde_json::json!(false);
    Ok(serde_json::to_vec_pretty(&value)?)
}

/// Copy resources beside a native export, without merging into its paired footprint library.
pub fn export_bundle(bundle: &ResourceBundle, destination: &Path) -> Result<()> {
    if destination.exists() { return Err(Error::msg("补充资源导出目录已存在")); }
    // Validate every file before creating an output directory; never copy external paths.
    for (kind, relative) in &bundle.files {
        if !safe_relative(relative) { return Err(Error::msg("补充资源路径无效")); }
        let source = bundle.directory.join(relative); let meta = fs::symlink_metadata(&source)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 256 * 1024 * 1024 { return Err(Error::msg("补充资源不是普通文件或过大")); }
        if bundle.hashes.get(kind) != Some(&native_library::content_hash(&fs::read(source)?)) { return Err(Error::msg("补充资源已被修改，请重新补全后导出")); }
    }
    fs::create_dir(destination)?;
    let result = (|| {
        for (kind, relative) in &bundle.files {
            let target = destination.join(relative); if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
            native_library::copy_file(&bundle.directory.join(relative), &target)?;
            if bundle.hashes.get(kind)!=Some(&native_library::content_hash(&fs::read(&target)?)){return Err(Error::msg("补充资源在导出期间发生变化，已取消此资源导出"));}
        }
        let mut exported = bundle.clone(); exported.directory = destination.to_path_buf();
        if let (Some(footprint), Some(model)) = (exported.path("kicad_mod"), exported.path("step")) {
            let fp: FootprintIr = serde_json::from_slice(&fs::read(exported.path("footprint_ir").ok_or_else(|| Error::msg("缺少封装预览资源"))?)?)?;
            kicad::write_footprint_mod(&footprint, &fp, Some(&model.to_string_lossy().replace('\\', "/")))?;
            exported.hashes.insert("kicad_mod".into(), native_library::content_hash(&fs::read(footprint)?));
        }
        write_new(&destination.join("resources.json"), &manifest(&exported)?)?; Ok(())
    })();
    if result.is_err() { let _ = fs::remove_dir_all(destination); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (NativePart, SearchItem, FootprintIr) {
        let part = NativePart { key: "Existing".into(), name: "Original".into(), description: "custom".into(), properties: BTreeMap::new(), footprints: vec!["MyExistingFootprint".into()], pins: 2, units: 1 };
        let item = SearchItem { index: 1, display_title: "10k".into(), title: "10k".into(), manufacturer: "test".into(), model_uuid: Some("model".into()), raw: serde_json::json!({"product_code":"C25804","footprint":{"uuid":"fp"}}) };
        let fp = ir::footprint_ir("source", "", easyeda::EasyedaFootprint::default(), item.meta()); (part,item,fp)
    }
    #[test]
    fn default_has_no_resources_and_original_links_survive_both_formats() {
        assert!(!ResourceKinds::default().any());
        let root = std::env::temp_dir().join(format!("supplement-{}",crate::util::unique_id()));
        let (part,item,fp) = fixture();
        for format in LibraryFormat::ALL {
            let bundle = write_bundle(&part,format,&item,ResourceKinds {footprint:true,model3d:false},&root,Some((serde_json::json!({}),fp.clone())),None,None).unwrap();
            assert_eq!(part.footprints,["MyExistingFootprint"]); assert_eq!(bundle.original_footprints,part.footprints);
            assert!(!bundle.files.contains_key("step")); assert!(!bundle.files.contains_key("schlib")); assert!(!bundle.files.contains_key("kicad_sym"));
            let output = root.join(format!("export-{}",format.name())); export_bundle(&bundle,&output).unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(&fs::read(output.join("resources.json")).unwrap()).unwrap();
            assert_eq!(manifest["original_bindings_changed"],false); assert!(manifest.get("directory").is_none());
            assert!(export_bundle(&bundle,&output).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn model_only_never_creates_or_replaces_a_footprint() {
        let root=std::env::temp_dir().join(format!("supplement-{}",crate::util::unique_id()));let(part,item,_)=fixture();
        let bundle=write_bundle(&part,LibraryFormat::Ad,&item,ResourceKinds{footprint:false,model3d:true},&root,None,Some(b"ISO-10303-21;\nEND-ISO-10303-21;".to_vec()),None).unwrap();
        assert_eq!(bundle.files.len(),1);assert!(bundle.path("step").unwrap().is_file());assert_eq!(part.footprints,["MyExistingFootprint"]);
        let output=root.join("output");export_bundle(&bundle,&output).unwrap();
        fs::write(bundle.path("step").unwrap(),"modified").unwrap();let bad=root.join("bad");assert!(export_bundle(&bundle,&bad).is_err());assert!(!bad.exists());
        let mut invalid=bundle.clone();invalid.files.insert("step".into(),PathBuf::from("../outside.step"));assert!(invalid.path("step").is_none());assert!(export_bundle(&invalid,&bad).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn both_resources_embed_step_in_new_ad_footprint_and_rebase_kicad_export() {
        use std::io::Read;
        let root=std::env::temp_dir().join(format!("supplement-{}",crate::util::unique_id()));let(part,item,fp)=fixture();
        let step=b"ISO-10303-21;\nHEADER;ENDSEC;DATA;ENDSEC;END-ISO-10303-21;".to_vec();
        fs::create_dir_all(&root).unwrap();let original=root.join("original.SchLib");fs::write(&original,b"original bytes, keep links").unwrap();
        for format in LibraryFormat::ALL {
            let bundle=write_bundle(&part,format,&item,ResourceKinds{footprint:true,model3d:true},&root,Some((serde_json::json!({}),fp.clone())),Some(step.clone()),None).unwrap();
            if format==LibraryFormat::Ad {
                let mut doc=cfb::open(bundle.path("pcblib").unwrap()).unwrap();let mut compressed=Vec::new();doc.open_stream("/Library/Models/0").unwrap().read_to_end(&mut compressed).unwrap();
                let mut unpacked=Vec::new();flate2::read::ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut unpacked).unwrap();assert_eq!(unpacked,step);
            }
            let output=root.join(format!("export-{}",format.name()));export_bundle(&bundle,&output).unwrap();
            if let Some(relative)=bundle.files.get("kicad_mod") {
                let exported=fs::read_to_string(output.join(relative)).unwrap();
                assert!(exported.contains(&output.to_string_lossy().replace('\\', "/")));
                assert!(!exported.contains(&bundle.directory.to_string_lossy().replace('\\', "/")));
            }
            assert_eq!(fs::read(&original).unwrap(),b"original bytes, keep links");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
