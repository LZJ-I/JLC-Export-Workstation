use crate::altium;
use crate::client::LcedaClient;
use crate::easyeda;
use crate::error::{Error, Result};
use crate::ir::{self, FootprintIr, PartMeta, SymbolIr};
use crate::kicad;
use crate::mesh;
use crate::pads;
use crate::models::{DownloadPaths, SearchItem};
use crate::util::{ensure_parent, looks_like_step, sanitize_filename};
use serde_json::Value;
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
        }
    }
}

impl ExportRequest {
    pub fn any_library(&self) -> bool {
        self.ad || self.kicad || self.pads || self.source_json
    }
}

pub fn export(client: &LcedaClient, item: &SearchItem, req: &ExportRequest) -> Result<DownloadPaths> {
    let folder_name = item.export_stem();
    let base = sanitize_filename(item.name());
    let part_dir = req.out_dir.join(&folder_name);
    let mut out = DownloadPaths::default();
    let want_other = req.step || req.obj || req.any_library();

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
            fs::write(&path, bytes)?;
        }
        out.step = Some(path);
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
        let (symbol_json, footprint_json, symbol_ir, footprint_ir) =
            fetch_sources(client, item, &part_dir, &base, req.force)?;
        if req.source_json || req.ad || req.kicad || req.pads {
            out.symbol_json = symbol_json;
            out.footprint_json = footprint_json;
        }

        if req.ad {
            if let Err(e) = export_altium(&mut out, &part_dir, &base, symbol_ir.as_ref(), footprint_ir.as_ref())
            {
                if !req.kicad {
                    return Err(e);
                }
                eprintln!("{e}");
            }
        }
        if req.kicad {
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
    Ok(out)
}

pub fn export_batch(
    client: &LcedaClient,
    keywords: &[String],
    req: &ExportRequest,
) -> Vec<(String, Result<DownloadPaths>)> {
    let mut rows = Vec::new();
    for kw in keywords {
        let kw = kw.trim();
        if kw.is_empty() {
            continue;
        }
        let result = client.select(kw, 1).and_then(|item| export(client, &item, req));
        rows.push((kw.to_string(), result));
    }
    rows
}

fn export_altium(
    out: &mut DownloadPaths,
    out_dir: &Path,
    base: &str,
    symbol_ir: Option<&SymbolIr>,
    footprint_ir: Option<&FootprintIr>,
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
        match altium::write_pcblib(&pcb, fp) {
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
