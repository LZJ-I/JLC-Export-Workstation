use crate::altium;
use crate::altium::SchColors;
use crate::client::LcedaClient;
use crate::easyeda;
use crate::error::{Error, Result};
use crate::ir::{self, FootprintIr, PartMeta, SymbolIr};
use crate::kicad;
use crate::mesh;
use crate::pads;
use crate::models::{DownloadPaths, SearchItem};
use crate::util::{ensure_parent, looks_like_step, sanitize_filename, unique_name};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub step: bool,
    pub obj: bool,
    pub ad: bool,
    pub kicad: bool,
    pub pads: bool,
    pub datasheet: bool,
    pub source_json: bool,
    pub force: bool,
    pub out_dir: PathBuf,
    /// Embed STEP into PcbLib when a 3D model exists.
    pub ad_embed_3d: bool,
    /// Write STEP into `{name}.3dshapes` and reference it from the KiCad footprint.
    pub kicad_attach_3d: bool,
    /// 用器件型号覆盖立创原来的封装名。默认 false，保留通用封装名。
    pub rename_footprint: bool,
    /// Batch: write one combined library instead of per-part folders for AD/KiCad.
    pub merge: bool,
    pub merge_name: String,
    /// Altium 原理图库颜色。默认是 AD 经典暗红/蓝。
    pub sch_colors: SchColors,
}

impl Default for ExportRequest {
    fn default() -> Self {
        Self {
            step: false,
            obj: false,
            ad: false,
            kicad: false,
            pads: false,
            datasheet: false,
            source_json: false,
            force: false,
            out_dir: PathBuf::from("."),
            ad_embed_3d: true,
            kicad_attach_3d: true,
            rename_footprint: false,
            merge: false,
            merge_name: "lceda".into(),
            sch_colors: SchColors::default(),
        }
    }
}

impl ExportRequest {
    pub fn any_library(&self) -> bool {
        self.ad || self.kicad || self.pads || self.source_json
    }

    fn wants_library_step(&self, item: &SearchItem) -> bool {
        item.model_uuid.is_some()
            && ((self.ad && self.ad_embed_3d) || (self.kicad && self.kicad_attach_3d))
    }
}

pub fn export(client: &LcedaClient, item: &SearchItem, req: &ExportRequest) -> Result<DownloadPaths> {
    Ok(export_part(client, item, req)?.paths)
}

struct PartExport {
    paths: DownloadPaths,
    symbol: Option<SymbolIr>,
    footprint: Option<FootprintIr>,
    step: Option<Vec<u8>>,
}

fn export_part(client: &LcedaClient, item: &SearchItem, req: &ExportRequest) -> Result<PartExport> {
    let folder_name = item.export_stem();
    let base = sanitize_filename(item.name());
    let part_dir = req.out_dir.join(&folder_name);
    let mut out = DownloadPaths::default();
    let want_other = req.step || req.obj || req.any_library();
    let mut step_bytes: Option<Vec<u8>> = None;
    let mut symbol_ir = None;
    let mut footprint_ir = None;

    if req.step {
        if item.model_uuid.is_none() {
            return Err(Error::No3dModel);
        }
        let path = part_dir.join(format!("{base}.step"));
        if req.force || !path.exists() {
            let bytes = client.download_step_bytes(item)?;
            if !looks_like_step(&bytes) {
                return Err(Error::msg("下载的 STEP 不是有效模型（可能是接口错误页）"));
            }
            ensure_parent(&path)?;
            fs::write(&path, &bytes)?;
            step_bytes = Some(bytes);
        } else {
            if let Ok(bytes) = fs::read(&path) {
                if looks_like_step(&bytes) {
                    step_bytes = Some(bytes);
                }
            }
        }
        out.step = Some(path);
    } else if req.wants_library_step(item) {
        match client.download_step_bytes(item) {
            Ok(bytes) if looks_like_step(&bytes) => step_bytes = Some(bytes),
            Ok(_) => eprintln!("STEP 不是有效模型，库导出将不含 3D"),
            Err(e) => eprintln!("下载 STEP 失败，库导出将不含 3D: {e}"),
        }
    }

    if req.obj {
        if item.model_uuid.is_none() {
            return Err(Error::No3dModel);
        }
        let obj_path = part_dir.join(format!("{base}.obj"));
        let mtl_path = part_dir.join(format!("{base}.mtl"));
        if req.force || !obj_path.exists() || !mtl_path.exists() {
            let bytes = client.download_obj_bytes(item)?;
            let text = String::from_utf8_lossy(&bytes);
            let (obj, mtl) = mesh::split_obj_mtl(&text);
            ensure_parent(&obj_path)?;
            fs::write(&obj_path, format!("mtllib {base}.mtl\n{obj}"))?;
            fs::write(&mtl_path, mtl)?;
        }
        out.obj = Some(obj_path);
        out.mtl = Some(mtl_path);
    }

    if req.datasheet {
        match item.datasheet_url() {
            Some(url) => {
                let path = part_dir.join(format!("{base}.pdf"));
                if req.force || !path.exists() {
                    match client.download_datasheet_pdf(&url) {
                        Ok(bytes) => {
                            ensure_parent(&path)?;
                            fs::write(&path, bytes)?;
                            out.datasheet = Some(path);
                        }
                        Err(e) if want_other => eprintln!("数据手册: {e}"),
                        Err(e) => return Err(e),
                    }
                } else {
                    out.datasheet = Some(path);
                }
            }
            None if want_other => eprintln!("该器件没有数据手册链接"),
            None => return Err(Error::msg("该器件没有数据手册链接")),
        }
    }

    if req.any_library() {
        if !item.has_symbol_or_footprint() {
            return Err(Error::NoSymbolOrFootprint);
        }
        let (symbol_json, footprint_json, fetched_sym, fetched_fp) =
            fetch_sources(client, item, &part_dir, &base, req.force, req.rename_footprint)?;
        symbol_ir = fetched_sym;
        footprint_ir = fetched_fp;
        if req.source_json || req.ad || req.kicad || req.pads {
            out.symbol_json = symbol_json;
            out.footprint_json = footprint_json;
        }

        if req.ad && !req.merge {
            if let Err(e) = export_altium(
                &mut out,
                &part_dir,
                &base,
                symbol_ir.as_ref(),
                footprint_ir.as_ref(),
                step_bytes.as_deref(),
                req.sch_colors,
            ) {
                if !req.kicad {
                    return Err(e);
                }
                eprintln!("{e}");
            }
        }
        if req.kicad && !req.merge {
            export_kicad(
                &mut out,
                &part_dir,
                &base,
                symbol_ir.as_ref(),
                footprint_ir.as_ref(),
                step_bytes.as_deref(),
                req.kicad_attach_3d,
            )?;
        }
        if req.pads {
            export_pads(
                &mut out,
                &part_dir,
                &base,
                symbol_ir.as_ref(),
                footprint_ir.as_ref(),
            )?;
        }
    }

    if !out.has_files() {
        return Err(Error::msg("没有写出任何文件"));
    }
    out.folder = Some(part_dir);
    Ok(PartExport {
        paths: out,
        symbol: symbol_ir,
        footprint: footprint_ir,
        step: step_bytes,
    })
}

pub fn export_batch(
    client: &LcedaClient,
    keywords: &[String],
    req: &ExportRequest,
) -> Vec<(String, Result<DownloadPaths>)> {
    let merge_libs = req.merge && (req.ad || req.kicad);
    let mut rows = Vec::new();
    let mut collected = Vec::new();
    for kw in keywords {
        let kw = kw.trim();
        if kw.is_empty() {
            continue;
        }
        if merge_libs {
            let result = client.select(kw, 1).and_then(|item| export_part(client, &item, req));
            match result {
                Ok(part) => {
                    rows.push((kw.to_string(), Ok(part.paths.clone())));
                    collected.push(part);
                }
                Err(e) => rows.push((kw.to_string(), Err(e))),
            }
        } else {
            let result = client.select(kw, 1).and_then(|item| export(client, &item, req));
            rows.push((kw.to_string(), result));
        }
    }
    if merge_libs && !collected.is_empty() {
        uniquify_collected(&mut collected);
        match write_merged_libraries(req, &collected) {
            Ok(merged) => {
                let mut idx = 0;
                for (_, result) in &mut rows {
                    if result.is_ok() {
                        let mut paths = collected[idx].paths.clone();
                        if req.ad {
                            paths.schlib = merged.schlib.clone();
                            paths.pcblib = merged.pcblib.clone();
                        }
                        if req.kicad {
                            paths.kicad_sym = merged.kicad_sym.clone();
                            paths.kicad_mod = merged.kicad_mod.clone();
                        }
                        *result = Ok(paths);
                        idx += 1;
                    }
                }
            }
            Err(e) => rows.push((req.merge_name.clone(), Err(e))),
        }
    }
    rows
}

fn uniquify_collected(parts: &mut [PartExport]) {
    let mut used_sym = HashSet::new();
    let mut used_fp = HashSet::new();
    for part in parts.iter_mut() {
        if let Some(sym) = &mut part.symbol {
            sym.name = unique_name(&sym.name, &mut used_sym);
        }
        if let Some(fp) = &mut part.footprint {
            fp.name = unique_name(&fp.name, &mut used_fp);
            if let Some(sym) = &mut part.symbol {
                sym.meta.footprint_lib = fp.name.clone();
            }
        }
    }
}

fn write_merged_libraries(req: &ExportRequest, parts: &[PartExport]) -> Result<DownloadPaths> {
    let name = if req.merge_name.trim().is_empty() {
        "lceda"
    } else {
        req.merge_name.trim()
    };
    let name = sanitize_filename(name);
    let mut out = DownloadPaths::default();
    let mut ad_err: Option<String> = None;

    if req.ad {
        let symbols: Vec<&SymbolIr> = parts.iter().filter_map(|p| p.symbol.as_ref()).collect();
        if !symbols.is_empty() {
            let sch = req.out_dir.join(format!("{name}.SchLib"));
            match altium::write_schlib_many_with_colors(&sch, &symbols, req.sch_colors) {
                Ok(()) if sch.exists() && sch.metadata().map(|m| m.len()).unwrap_or(0) > 64 => {
                    out.schlib = Some(sch);
                }
                Ok(()) => ad_err = Some("SchLib 写出后文件为空".into()),
                Err(e) => ad_err = Some(e.to_string()),
            }
        }
        let pcblib_parts: Vec<altium::pcblib::PcbLibPart<'_>> = parts
            .iter()
            .filter_map(|p| {
                p.footprint.as_ref().map(|fp| altium::pcblib::PcbLibPart {
                    fp,
                    step: p.step.as_deref(),
                })
            })
            .collect();
        if !pcblib_parts.is_empty() {
            let pcb = req.out_dir.join(format!("{name}.PcbLib"));
            match altium::write_pcblib_library(&pcb, &pcblib_parts) {
                Ok(()) if pcb.exists() && pcb.metadata().map(|m| m.len()).unwrap_or(0) > 64 => {
                    out.pcblib = Some(pcb);
                }
                Ok(()) => {
                    ad_err.get_or_insert("PcbLib 写出后文件为空".into());
                }
                Err(e) => {
                    let msg = e.to_string();
                    ad_err = Some(match ad_err {
                        Some(prev) => format!("{prev}; {msg}"),
                        None => msg,
                    });
                }
            }
        }
        if req.ad && out.schlib.is_none() && out.pcblib.is_none() {
            return Err(Error::Altium(ad_err.unwrap_or_else(|| {
                "未能生成合并 SchLib/PcbLib".into()
            })));
        }
    }

    if req.kicad {
        let mut symbols: Vec<SymbolIr> = parts.iter().filter_map(|p| p.symbol.clone()).collect();
        let pretty = kicad::pretty_dir(&req.out_dir, &name);
        let shapes = req.out_dir.join(format!("{name}.3dshapes"));
        for part in parts {
            let Some(fp) = part.footprint.as_ref() else {
                continue;
            };
            let step_name = format!("{}.step", sanitize_filename(&fp.name));
            let mut step_rel = None;
            if req.kicad_attach_3d {
                if let Some(bytes) = part.step.as_ref() {
                    step_rel = write_kicad_step(&shapes, &name, &step_name, bytes)?;
                } else if let Some(src) = part.paths.step.as_ref() {
                    if let Ok(bytes) = fs::read(src) {
                        if looks_like_step(&bytes) {
                            step_rel = write_kicad_step(&shapes, &name, &step_name, &bytes)?;
                        }
                    }
                }
            }
            let path = pretty.join(format!("{}.kicad_mod", sanitize_filename(&fp.name)));
            kicad::write_footprint_mod(&path, fp, step_rel.as_deref())?;
            out.kicad_mod = Some(path);
        }
        for sym in &mut symbols {
            if !sym.meta.footprint_lib.is_empty() {
                let fp = sanitize_filename(&sym.meta.footprint_lib);
                sym.meta.footprint_lib = format!("{name}:{fp}");
            }
        }
        if !symbols.is_empty() {
            let path = req.out_dir.join(format!("{name}.kicad_sym"));
            kicad::write_symbol_lib_many(&path, &symbols)?;
            out.kicad_sym = Some(path);
        }
        if out.kicad_sym.is_none() && out.kicad_mod.is_none() {
            return Err(Error::msg("未能生成合并 KiCad 库"));
        }
    }

    Ok(out)
}

fn export_altium(
    out: &mut DownloadPaths,
    out_dir: &Path,
    base: &str,
    symbol_ir: Option<&SymbolIr>,
    footprint_ir: Option<&FootprintIr>,
    step: Option<&[u8]>,
    colors: SchColors,
) -> Result<()> {
    let mut ad_err: Option<String> = None;
    if let Some(sym) = symbol_ir {
        let mut sym = sym.clone();
        if let Some(fp) = footprint_ir {
            if sym.meta.footprint_lib.trim().is_empty() {
                sym.meta.footprint_lib = fp.name.clone();
            }
        }
        let sch = out_dir.join(format!("{base}.SchLib"));
        match altium::write_schlib_with_colors(&sch, &sym, colors) {
            Ok(()) if sch.exists() && sch.metadata().map(|m| m.len()).unwrap_or(0) > 64 => {
                out.schlib = Some(sch);
            }
            Ok(()) => ad_err = Some("SchLib 写出后文件为空".into()),
            Err(e) => ad_err = Some(e.to_string()),
        }
    }
    if let Some(fp) = footprint_ir {
        let pcb = out_dir.join(format!("{base}.PcbLib"));
        match altium::write_pcblib_with_step(&pcb, fp, step) {
            Ok(()) if pcb.exists() && pcb.metadata().map(|m| m.len()).unwrap_or(0) > 64 => {
                out.pcblib = Some(pcb);
            }
            Ok(()) => {
                ad_err.get_or_insert("PcbLib 写出后文件为空".into());
            }
            Err(e) => {
                let msg = e.to_string();
                ad_err = Some(match ad_err {
                    Some(prev) => format!("{prev}; {msg}"),
                    None => msg,
                });
            }
        }
    }
    if out.schlib.is_none() && out.pcblib.is_none() {
        return Err(Error::Altium(ad_err.unwrap_or_else(|| {
            "未能生成 SchLib/PcbLib，已保留 EasyEDA 源 JSON".into()
        })));
    }
    Ok(())
}

/// Place STEP beside the `.pretty` folder and return a path relative to the `.kicad_mod`.
/// KiCad resolves non-`${}` model paths from the footprint file. npnp aliases
/// `${lib}/name.3dshapes`, which needs extra 3D search-path config.
fn write_kicad_step(
    shapes: &Path,
    lib_base: &str,
    step_file: &str,
    bytes: &[u8],
) -> Result<Option<String>> {
    fs::create_dir_all(shapes)?;
    fs::write(shapes.join(step_file), bytes)?;
    Ok(Some(format!("../{lib_base}.3dshapes/{step_file}")))
}

fn export_kicad(
    out: &mut DownloadPaths,
    out_dir: &Path,
    base: &str,
    symbol_ir: Option<&SymbolIr>,
    footprint_ir: Option<&FootprintIr>,
    step: Option<&[u8]>,
    attach_3d: bool,
) -> Result<()> {
    let pretty = kicad::pretty_dir(out_dir, base);
    let mut step_rel = None;
    if attach_3d {
        if let Some(bytes) = step {
            let shapes = out_dir.join(format!("{base}.3dshapes"));
            let step_file = format!("{base}.step");
            step_rel = write_kicad_step(&shapes, base, &step_file, bytes)?;
        }
    }

    if let Some(sym) = symbol_ir {
        let mut sym = sym.clone();
        if footprint_ir.is_some() {
            sym.meta.footprint_lib = format!("{base}:{base}");
        }
        let path = out_dir.join(format!("{base}.kicad_sym"));
        kicad::write_symbol_lib(&path, &sym)?;
        out.kicad_sym = Some(path);
    }
    if let Some(fp) = footprint_ir {
        let path = pretty.join(format!("{base}.kicad_mod"));
        kicad::write_footprint_mod(&path, fp, step_rel.as_deref())?;
        out.kicad_mod = Some(path);
    }
    if out.kicad_sym.is_none() && out.kicad_mod.is_none() {
        return Err(Error::msg("未能生成 KiCad 库，已保留 EasyEDA 源 JSON"));
    }
    Ok(())
}

fn export_pads(
    out: &mut DownloadPaths,
    out_dir: &Path,
    base: &str,
    symbol_ir: Option<&SymbolIr>,
    footprint_ir: Option<&FootprintIr>,
) -> Result<()> {
    let (c, d, p) = pads::write_part_files(out_dir, base, symbol_ir, footprint_ir)?;
    out.pads_c = c;
    out.pads_d = d;
    out.pads_p = p;
    if out.pads_c.is_none() && out.pads_d.is_none() && out.pads_p.is_none() {
        return Err(Error::msg("未能生成 PADS .c/.d/.p，已保留 EasyEDA 源 JSON"));
    }
    Ok(())
}

fn fetch_sources(
    client: &LcedaClient,
    item: &SearchItem,
    out_dir: &Path,
    base: &str,
    force: bool,
    rename_footprint: bool,
) -> Result<(
    Option<PathBuf>,
    Option<PathBuf>,
    Option<SymbolIr>,
    Option<FootprintIr>,
)> {
    let mut symbol_path = None;
    let mut footprint_path = None;
    let mut symbol_ir = None;
    let mut footprint_ir = None;
    let desc = item.name().to_string();
    let meta: PartMeta = item.meta();

    if let Some(uuid) = item.symbol_uuid() {
        let json = client.component_json(&uuid)?;
        let path = out_dir.join(format!("{base}_symbol_easyeda.json"));
        write_json(&path, &json, force)?;
        symbol_path = Some(path);
        match easyeda::parse_symbol(&json) {
            Ok(src) => symbol_ir = Some(ir::symbol_ir(base, &desc, src, meta.clone())),
            Err(e) => eprintln!("解析原理图失败: {e}"),
        }
    }
    if let Some(uuid) = item.footprint_uuid() {
        let json = client.component_json(&uuid)?;
        let path = out_dir.join(format!("{base}_footprint_easyeda.json"));
        write_json(&path, &json, force)?;
        footprint_path = Some(path);
        match easyeda::parse_footprint(&json) {
            Ok(src) => {
                let name = if rename_footprint {
                    base.to_string()
                } else {
                    easyeda::component_display_title(&json)
                        .map(|s| sanitize_filename(&s))
                        .filter(|s| !s.is_empty() && s != "component")
                        .unwrap_or_else(|| base.to_string())
                };
                footprint_ir = Some(ir::footprint_ir(&name, &desc, src, meta));
            }
            Err(e) => eprintln!("解析封装失败: {e}"),
        }
    }
    Ok((symbol_path, footprint_path, symbol_ir, footprint_ir))
}

fn write_json(path: &Path, value: &Value, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Ok(());
    }
    ensure_parent(path)?;
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{FootprintIr, IrPad};

    fn sample_fp(name: &str) -> FootprintIr {
        FootprintIr {
            name: name.into(),
            description: String::new(),
            meta: Default::default(),
            pads: vec![IrPad {
                designator: "1".into(),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                hole: 0.0,
                hole_slot: 0.0,
                hole_shape: "ROUND".into(),
                rotation: 0.0,
                layer: 1,
                shape: "RECT".into(),
                polygon: None,
            }],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
            model: Default::default(),
        }
    }

    fn dummy_step() -> Vec<u8> {
        b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n".to_vec()
    }

    #[test]
    fn kicad_export_writes_3dshapes_and_npnp_style_model_block() {
        let dir = std::env::temp_dir().join("lceda-test-kicad-3d");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fp = sample_fp("CHIP");
        let mut out = DownloadPaths::default();
        export_kicad(&mut out, &dir, "CHIP", None, Some(&fp), Some(&dummy_step()), true).unwrap();

        let step = dir.join("CHIP.3dshapes").join("CHIP.step");
        assert!(step.exists(), "STEP should sit in name.3dshapes like npnp");
        assert!(looks_like_step(&std::fs::read(&step).unwrap()));
        let mod_path = dir.join("CHIP.pretty").join("CHIP.kicad_mod");
        let text = std::fs::read_to_string(&mod_path).unwrap();
        assert!(
            text.contains("(model \"../CHIP.3dshapes/CHIP.step\""),
            "relative to .kicad_mod so KiCad opens a downloaded folder without extra 3D paths:\n{text}"
        );
        assert!(text.contains("(offset (xyz 0 0 0))"));
        assert!(text.contains("(scale (xyz 1 1 1))"));
        assert!(text.contains("(rotate (xyz 0 0 0))"));
    }

    #[test]
    fn kicad_export_skips_3d_when_disabled() {
        let dir = std::env::temp_dir().join("lceda-test-kicad-no3d");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fp = sample_fp("CHIP");
        let mut out = DownloadPaths::default();
        export_kicad(&mut out, &dir, "CHIP", None, Some(&fp), Some(&dummy_step()), false).unwrap();
        assert!(!dir.join("CHIP.3dshapes").exists());
        let text = std::fs::read_to_string(dir.join("CHIP.pretty").join("CHIP.kicad_mod")).unwrap();
        assert!(!text.contains("(model "));
    }

    #[test]
    fn kicad_export_without_step_still_writes_footprint() {
        let dir = std::env::temp_dir().join("lceda-test-kicad-missing-step");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fp = sample_fp("CHIP");
        let mut out = DownloadPaths::default();
        export_kicad(&mut out, &dir, "CHIP", None, Some(&fp), None, true).unwrap();
        assert!(!dir.join("CHIP.3dshapes").exists());
        let text = std::fs::read_to_string(dir.join("CHIP.pretty").join("CHIP.kicad_mod")).unwrap();
        assert!(!text.contains("(model "));
        assert!(text.contains("(footprint"));
    }

    #[test]
    fn merged_kicad_library_attaches_step_per_footprint() {
        let dir = std::env::temp_dir().join("lceda-test-kicad-merge-3d");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let parts = vec![
            PartExport {
                paths: DownloadPaths::default(),
                symbol: None,
                footprint: Some(sample_fp("AAA")),
                step: Some(dummy_step()),
            },
            PartExport {
                paths: DownloadPaths::default(),
                symbol: None,
                footprint: Some(sample_fp("BBB")),
                step: Some(dummy_step()),
            },
        ];
        let req = ExportRequest {
            kicad: true,
            kicad_attach_3d: true,
            merge: true,
            merge_name: "lceda".into(),
            out_dir: dir.clone(),
            ..Default::default()
        };
        write_merged_libraries(&req, &parts).unwrap();
        assert!(dir.join("lceda.3dshapes").join("AAA.step").exists());
        assert!(dir.join("lceda.3dshapes").join("BBB.step").exists());
        let aaa = std::fs::read_to_string(dir.join("lceda.pretty").join("AAA.kicad_mod")).unwrap();
        assert!(aaa.contains("(model \"../lceda.3dshapes/AAA.step\""));
        let bbb = std::fs::read_to_string(dir.join("lceda.pretty").join("BBB.kicad_mod")).unwrap();
        assert!(bbb.contains("(model \"../lceda.3dshapes/BBB.step\""));
    }

    #[test]
    fn merged_kicad_library_skips_3d_when_disabled() {
        let dir = std::env::temp_dir().join("lceda-test-kicad-merge-no3d");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let parts = vec![PartExport {
            paths: DownloadPaths::default(),
            symbol: None,
            footprint: Some(sample_fp("AAA")),
            step: Some(dummy_step()),
        }];
        let req = ExportRequest {
            kicad: true,
            kicad_attach_3d: false,
            merge: true,
            merge_name: "lceda".into(),
            out_dir: dir.clone(),
            ..Default::default()
        };
        write_merged_libraries(&req, &parts).unwrap();
        assert!(!dir.join("lceda.3dshapes").exists());
        let aaa = std::fs::read_to_string(dir.join("lceda.pretty").join("AAA.kicad_mod")).unwrap();
        assert!(!aaa.contains("(model "));
    }
}

