use anyhow::{bail, Result};
use clap::Args;
use lceda_core::{altium::SchColorScheme, export::ExportRequest, pads::SchematicGrid};
use std::path::PathBuf;

/// One set of export switches shared by single-part, list and BOM commands.
#[derive(Args, Debug, Default)]
pub(super) struct ExportOptions {
    #[arg(long)]
    pub step: bool,
    #[arg(long)]
    pub obj: bool,
    #[arg(long)]
    pub ad: bool,
    #[arg(long)]
    pub kicad: bool,
    #[arg(long)]
    pub pads: bool,
    /// PADS schematic connection grid, 1–1000 mil (default: saved setting).
    #[arg(long)]
    pub pads_grid: Option<SchematicGrid>,
    #[arg(long)]
    pub datasheet: bool,
    /// Keep EasyEDA source JSON (also included with library exports).
    #[arg(long)]
    pub source: bool,
    #[arg(long)]
    pub no_ad_3d: bool,
    #[arg(long)]
    pub no_kicad_3d: bool,
    #[arg(long)]
    pub rename_footprint: bool,
    #[arg(long, value_parser = ["altium", "easyeda", "mono"])]
    pub sch_color: Option<String>,
    #[arg(short, long, default_value = "out")]
    pub output: PathBuf,
    /// Refresh downloaded resources; library files are regenerated on every export.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug, Default)]
pub(super) struct MergeOptions {
    /// Combine selected AD / KiCad parts into one library per format (PADS stays per-part).
    #[arg(long)]
    pub merge: bool,
    /// Combined library filename stem.
    #[arg(long, requires = "merge")]
    pub name: Option<String>,
}

impl ExportOptions {
    pub fn request(&self) -> ExportRequest {
        let prefs = crate::prefs::load();
        let scheme = self
            .sch_color
            .as_deref()
            .map(SchColorScheme::parse)
            .unwrap_or(prefs.sch_scheme);
        let mut req = ExportRequest {
            step: self.step,
            obj: self.obj,
            ad: self.ad,
            kicad: self.kicad,
            pads: self.pads,
            pads_grid: self.pads_grid.unwrap_or(prefs.pads_grid),
            datasheet: self.datasheet,
            source_json: self.source || self.ad || self.kicad || self.pads,
            force: self.force,
            out_dir: self.output.clone(),
            ad_embed_3d: !self.no_ad_3d,
            kicad_attach_3d: !self.no_kicad_3d,
            rename_footprint: self.rename_footprint,
            sch_colors: scheme.colors(prefs.sch_custom),
            desc_fields: prefs.desc_fields.clone(),
            ..Default::default()
        };
        if !req.step && !req.obj && !req.any_library() && !req.datasheet {
            req.step = true;
            req.ad = true;
            req.kicad = true;
            req.source_json = true;
        }
        req
    }
}

impl MergeOptions {
    pub fn apply(&self, req: &mut ExportRequest) -> Result<()> {
        req.merge = self.merge;
        if self.merge && !req.ad && !req.kicad {
            bail!("--merge requires AD or KiCad output; PADS libraries remain per-part");
        }
        if let Some(name) = &self.name {
            let sanitized = lceda_core::util::sanitize_filename(name);
            if name.trim().is_empty() || sanitized != *name || name == "." || name == ".." {
                bail!(
                    "--name must be a filename stem without path separators or reserved characters"
                );
            }
            req.merge_name = name.clone();
        }
        Ok(())
    }
}
