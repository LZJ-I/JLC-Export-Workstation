use crate::gui;
use crate::i18n::{self, Lang};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lceda_core::client::LcedaClient;
use lceda_core::export::{self, ExportRequest};
use lceda_core::models::{self, DownloadPaths};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "lceda", version, about = "JLC-Export：搜索、3D、Altium / KiCad / PADS 库导出")]
pub struct Cli {
    /// zh / en（默认跟随系统；图形界面会记住上次选择）
    #[arg(long, global = true)]
    pub lang: Option<String>,

    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// 打开图形界面
    Gui,
    /// 按关键字搜索
    Search {
        keyword: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// 下载 / 导出指定器件
    Get {
        keyword: String,
        #[arg(long, default_value_t = 1)]
        index: usize,
        #[arg(long)]
        step: bool,
        #[arg(long)]
        obj: bool,
        /// 写出 .SchLib / .PcbLib
        #[arg(long)]
        ad: bool,
        /// 写出 KiCad 符号 / 封装
        #[arg(long)]
        kicad: bool,
        /// 写出 PADS Logic/Layout ASCII 库（.c / .d / .p）
        #[arg(long)]
        pads: bool,
        /// 下载数据手册 PDF
        #[arg(long)]
        datasheet: bool,
        /// 同时（或仅）保存 EasyEDA JSON
        #[arg(long)]
        source: bool,
        /// 不把 STEP 嵌入 PcbLib（默认嵌入）
        #[arg(long = "no-ad-3d")]
        no_ad_3d: bool,
        /// 不把 STEP 写入 KiCad .3dshapes（默认写入）
        #[arg(long = "no-kicad-3d")]
        no_kicad_3d: bool,
        #[arg(short, long, default_value = "out")]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// 从文本文件批量导出（每行一个编号或关键字）
    Batch {
        file: PathBuf,
        #[arg(long)]
        step: bool,
        #[arg(long)]
        obj: bool,
        #[arg(long)]
        ad: bool,
        #[arg(long)]
        kicad: bool,
        #[arg(long)]
        pads: bool,
        #[arg(long)]
        datasheet: bool,
        #[arg(long)]
        source: bool,
        /// 不把 STEP 嵌入 PcbLib（默认嵌入）
        #[arg(long = "no-ad-3d")]
        no_ad_3d: bool,
        /// 不把 STEP 写入 KiCad .3dshapes（默认写入）
        #[arg(long = "no-kicad-3d")]
        no_kicad_3d: bool,
        /// 将本次批量的 AD/KiCad 合成一个库（不追加已有文件）
        #[arg(long)]
        merge: bool,
        #[arg(short, long, default_value = "out")]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

fn resolve_lang(cli_lang: Option<&str>) -> Lang {
    if let Some(code) = cli_lang {
        return Lang::from_code(code);
    }
    if let Some(code) = crate::prefs::load().lang {
        return Lang::from_code(&code);
    }
    Lang::detect()
}

pub fn run() -> Result<i32> {
    let cli = Cli::parse();
    let lang = resolve_lang(cli.lang.as_deref());

    match cli.cmd {
        None | Some(Cmd::Gui) => {
            gui::run(lang)?;
            Ok(0)
        }
        Some(Cmd::Search { keyword, limit }) => {
            let client = LcedaClient::new();
            let items = client.search(&keyword)?;
            if items.is_empty() {
                println!("{}", i18n::t(lang, "no_results"));
                return Ok(0);
            }
            println!("Found {}:", items.len());
            for item in items.iter().take(limit) {
                let flag = if item.model_uuid.is_some() { "yes" } else { "no" };
                let lcsc = item.lcsc_id().unwrap_or_default();
                println!(
                    "[{:>3}] {} | {} | {} | 3D: {}",
                    item.index,
                    item.name(),
                    if lcsc.is_empty() { "-" } else { &lcsc },
                    if item.manufacturer.is_empty() {
                        "-"
                    } else {
                        &item.manufacturer
                    },
                    flag
                );
            }
            Ok(0)
        }
        Some(Cmd::Get {
            keyword,
            index,
            step,
            obj,
            ad,
            kicad,
            pads,
            datasheet,
            source,
            no_ad_3d,
            no_kicad_3d,
            output,
            force,
        }) => {
            let client = LcedaClient::new();
            let item = client.select(&keyword, index)?;
            let req = build_req(
                step,
                obj,
                ad,
                kicad,
                pads,
                datasheet,
                source,
                !no_ad_3d,
                !no_kicad_3d,
                output,
                force,
            );
            let paths = export::export(&client, &item, &req).context("export failed")?;
            print_paths(&paths);
            Ok(0)
        }
        Some(Cmd::Batch {
            file,
            step,
            obj,
            ad,
            kicad,
            pads,
            datasheet,
            source,
            no_ad_3d,
            no_kicad_3d,
            merge,
            output,
            force,
        }) => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            let ids = models::parse_id_list(&text);
            if ids.is_empty() {
                println!("empty list");
                return Ok(1);
            }
            let mut req = build_req(
                step,
                obj,
                ad,
                kicad,
                pads,
                datasheet,
                source,
                !no_ad_3d,
                !no_kicad_3d,
                output,
                force,
            );
            req.merge = merge;
            let client = LcedaClient::new();
            let mut failed = 0;
            for (kw, result) in export::export_batch(&client, &ids, &req) {
                match result {
                    Ok(paths) => {
                        println!("OK {kw}");
                        print_paths(&paths);
                    }
                    Err(e) => {
                        failed += 1;
                        println!("FAIL {kw}: {e}");
                    }
                }
            }
            if failed > 0 { Ok(1) } else { Ok(0) }
        }
    }
}

fn build_req(
    step: bool,
    obj: bool,
    ad: bool,
    kicad: bool,
    pads: bool,
    datasheet: bool,
    source: bool,
    ad_embed_3d: bool,
    kicad_attach_3d: bool,
    output: PathBuf,
    force: bool,
) -> ExportRequest {
    let mut req = ExportRequest {
        step,
        obj,
        ad,
        kicad,
        pads,
        datasheet,
        source_json: source || ad || kicad || pads,
        force,
        out_dir: output,
        ad_embed_3d,
        kicad_attach_3d,
        merge: false,
        merge_name: "lceda".into(),
    };
    if !req.step && !req.obj && !req.ad && !req.kicad && !req.pads && !req.datasheet && !req.source_json
    {
        req.step = true;
        req.ad = true;
        req.kicad = true;
        req.source_json = true;
    }
    req
}

fn print_paths(paths: &DownloadPaths) {
    if let Some(p) = &paths.folder {
        println!("DIR: {}", p.display());
    }
    if let Some(p) = &paths.step {
        println!("STEP: {}", p.display());
    }
    if let Some(p) = &paths.obj {
        println!("OBJ: {}", p.display());
    }
    if let Some(p) = &paths.datasheet {
        println!("PDF: {}", p.display());
    }
    if let Some(p) = &paths.symbol_json {
        println!("Symbol JSON: {}", p.display());
    }
    if let Some(p) = &paths.footprint_json {
        println!("Footprint JSON: {}", p.display());
    }
    if let Some(p) = &paths.schlib {
        println!("SchLib: {}", p.display());
    }
    if let Some(p) = &paths.pcblib {
        println!("PcbLib: {}", p.display());
    }
    if let Some(p) = &paths.kicad_sym {
        println!("KiCad symbol: {}", p.display());
    }
    if let Some(p) = &paths.kicad_mod {
        println!("KiCad footprint: {}", p.display());
    }
    if let Some(p) = &paths.pads_c {
        println!("PADS .c: {}", p.display());
    }
    if let Some(p) = &paths.pads_d {
        println!("PADS .d: {}", p.display());
    }
    if let Some(p) = &paths.pads_p {
        println!("PADS .p: {}", p.display());
    }
}
