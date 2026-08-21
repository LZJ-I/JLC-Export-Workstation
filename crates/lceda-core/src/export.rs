use crate::altium;
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
    /// Batch: write one combined library instead of per-part folders for AD/KiCad.
    pub merge: bool,
    pub merge_name: String,
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
            merge: false,
            merge_name: "lceda".into(),
        }
    }
}

impl ExportRequest {
    pub fn any_library(&self) -> bool {
        self.ad || self.kicad || self.pads || self.source_json
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
    } else if req.ad && req.ad_embed_3d && item.model_uuid.is_some() {
        match client.download_step_bytes(item) {
            Ok(bytes) if looks_like_step(&bytes) => step_bytes = Some(bytes),
            Ok(_) => eprintln!("STEP 不是有效模型，PcbLib 将不含 3D"),
            Err(e) => eprintln!("下载 STEP 失败，PcbLib 将不含 3D: {e}"),
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
            fetch_sources(client, item, &part_dir, &base, req.force)?;
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
            ) {
                if !req.kicad {
                    return Err(e);
                }
                eprintln!("{e}");
            }
        }
        if req.kicad && !req.merge {
            let step = out.step.clone();
            export_kicad(
                &mut out,
                &part_dir,
                &base,
                symbol_ir.as_ref(),
                footprint_ir.as_ref(),
                step.as_deref(),
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
            match altium::write_schlib_many(&sch, &symbols) {
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
            let mut step_rel = None;
            if let Some(bytes) = part.step.as_ref() {
                fs::create_dir_all(&shapes)?;
                let dest = shapes.join(format!("{}.step", sanitize_filename(&fp.name)));
                fs::write(&dest, bytes)?;
                step_rel = Some(format!("../{name}.3dshapes/{}.step", sanitize_filename(&fp.name)));
            } else if let Some(src) = part.paths.step.as_ref() {
                fs::create_dir_all(&shapes)?;
                let dest = shapes.join(format!("{}.step", sanitize_filename(&fp.name)));
                if src != dest.as_path() {
                    let _ = fs::copy(src, &dest);
                }
                step_rel = Some(format!("../{name}.3dshapes/{}.step", sanitize_filename(&fp.name)));
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
        match altium::write_schlib(&sch, &sym) {
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

fn export_kicad(
    out: &mut DownloadPaths,
    out_dir: &Path,
    base: &str,
    symbol_ir: Option<&SymbolIr>,
    footprint_ir: Option<&FootprintIr>,
    step: Option<&Path>,
) -> Result<()> {
    let pretty = kicad::pretty_dir(out_dir, base);
    let mut step_rel = None;
    if let Some(src) = step {
        let shapes = out_dir.join(format!("{base}.3dshapes"));
        fs::create_dir_all(&shapes)?;
        let dest = shapes.join(format!("{base}.step"));
        if src != dest.as_path() {
            let _ = fs::copy(src, &dest);
        }
        step_rel = Some(format!("../{base}.3dshapes/{base}.step"));
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
            Ok(src) => footprint_ir = Some(ir::footprint_ir(base, &desc, src, meta)),
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
