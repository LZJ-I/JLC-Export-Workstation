//! Human and agent interfaces share selection, BOM validation and export code with the GUI.
mod bom;
mod library;
mod options;
mod output;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use lceda_core::{
    client::LcedaClient,
    export,
    models::{self, SearchItem},
};
use options::{ExportOptions, MergeOptions};
use output::Outcome;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "lceda",
    version,
    about = "JLC Export Workstation — search, BOM review, Altium / KiCad / PADS export"
)]
pub struct Cli {
    /// GUI language: zh / en (otherwise use saved preference or system language).
    #[arg(long, global = true, value_parser = ["en", "zh"])]
    pub lang: Option<String>,
    /// Emit one versioned JSON response on stdout; diagnostics go to stderr.
    #[arg(long, global = true)]
    json: bool,
    /// Isolate settings and local cache from the default desktop profile.
    #[arg(long, global = true)]
    profile: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the desktop application (also the default with no command).
    Gui,
    /// Search parts; use the returned LCSC ID for subsequent operations.
    Search {
        keyword: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Export an exact LCSC ID or an unambiguous MPN.
    Get {
        keyword: String,
        /// Explicitly choose a search result for an ambiguous MPN (1-based).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        index: Option<u32>,
        #[command(flatten)]
        export: ExportOptions,
        /// Update an existing AD library and its companion PcbLib (close AD first).
        #[arg(long, requires = "ad")]
        ad_library_file: Option<PathBuf>,
        /// Update an existing KiCad library and its companions.
        #[arg(long, requires = "kicad")]
        kicad_library_file: Option<PathBuf>,
    },
    /// Export a text list (one exact LCSC ID or unambiguous MPN per line).
    Batch {
        file: PathBuf,
        #[command(flatten)]
        export: ExportOptions,
        #[command(flatten)]
        merge: MergeOptions,
    },
    /// Import, match, review and export a BOM without opening the GUI.
    Bom {
        #[command(subcommand)]
        command: bom::Command,
    },
    /// Read back native library contents without opening an EDA editor.
    Library {
        #[command(subcommand)]
        command: library::Command,
    },
}

pub fn run() -> Result<i32> {
    let arguments: Vec<_> = std::env::args_os().collect();
    let json_mode = arguments.iter().any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => {
            error.print()?;
            return Ok(0);
        }
        Err(error) => return output::failure("arguments", json_mode, &error.to_string()),
    };
    let json_mode = cli.json;
    if let Some(path) = &cli.profile {
        let result = std::path::absolute(path)
            .context("resolve profile directory")
            .and_then(|path| {
                crate::PROFILE_OVERRIDE
                    .set(path)
                    .map_err(|_| anyhow::anyhow!("Profile already initialized"))
            });
        if let Err(error) = result {
            return output::failure("arguments", json_mode, &format!("{error:#}"));
        }
    }
    let name = match &cli.cmd {
        None | Some(Cmd::Gui) => "gui",
        Some(Cmd::Search { .. }) => "search",
        Some(Cmd::Get { .. }) => "get",
        Some(Cmd::Batch { .. }) => "batch",
        Some(Cmd::Bom { command }) => command.name(),
        Some(Cmd::Library { .. }) => "library.inspect",
    };
    match execute(cli) {
        Ok(outcome) => output::emit(name, json_mode, outcome),
        Err(error) => output::failure(name, json_mode, &format!("{error:#}")),
    }
}

fn execute(cli: Cli) -> Result<Outcome> {
    match cli.cmd {
        None | Some(Cmd::Gui) => {
            if cli.json {
                bail!("--json requires a CLI command, such as search or bom show");
            }
            let lang = cli
                .lang
                .or_else(|| crate::prefs::load().lang)
                .map(|code| crate::i18n::Lang::from_code(&code))
                .unwrap_or_else(crate::i18n::Lang::detect);
            if let Some(instance) = crate::instance::acquire() {
                crate::gui::run(lang, instance)?;
            }
            Ok(Outcome::success(json!({}), String::new()))
        }
        Some(Cmd::Search { keyword, limit }) => {
            let items = LcedaClient::new().search(&keyword)?;
            let mut text = format!(
                "Found {} (showing {}):",
                items.len(),
                limit.min(items.len())
            );
            for item in items.iter().take(limit) {
                text.push_str(&format!(
                    "\n[{}] {} | {} | {} | {}",
                    item.index,
                    item.lcsc_id().unwrap_or_default(),
                    item.name(),
                    item.manufacturer_label(),
                    crate::library::package_from_item(item)
                ));
            }
            Ok(Outcome::success(
                json!({"query": keyword, "total": items.len(),
                "items": items.iter().take(limit).map(output::item).collect::<Vec<_>>()}),
                text,
            ))
        }
        Some(Cmd::Get {
            keyword,
            index,
            export: options,
            ad_library_file,
            kicad_library_file,
        }) => {
            let client = LcedaClient::new();
            let selected = select(&keyword, client.search(&keyword)?, index)?;
            let mut req = options.request();
            if let Some(path) = ad_library_file {
                req.ad_library_files.insert(
                    selected.lcsc_id().context("AD binding needs an LCSC ID")?,
                    path,
                );
            }
            if let Some(path) = kicad_library_file {
                req.kicad_library_files.insert(
                    selected
                        .lcsc_id()
                        .context("KiCad binding needs an LCSC ID")?,
                    path,
                );
            }
            let result = export::export(&client, &selected, &req).map_err(|e| e.to_string());
            Ok(output::exports(
                vec![output::export_row(&keyword, Some(&selected), result, &req)],
                &req,
            ))
        }
        Some(Cmd::Batch {
            file,
            export: options,
            merge,
        }) => {
            let keywords = models::parse_id_list(
                &std::fs::read_to_string(&file)
                    .with_context(|| format!("read {}", file.display()))?,
            );
            if keywords.is_empty() {
                bail!("Part list is empty");
            }
            let mut req = options.request();
            merge.apply(&mut req)?;
            let client = LcedaClient::new();
            let mut selected = Vec::new();
            let mut queries = HashMap::new();
            let mut seen = HashSet::new();
            let mut rows = Vec::new();
            for keyword in keywords {
                if !seen.insert(keyword.to_ascii_lowercase()) {
                    continue;
                }
                let result = client
                    .search(&keyword)
                    .map_err(anyhow::Error::from)
                    .and_then(|items| select(&keyword, items, None));
                match result {
                    Ok(item) => {
                        let id = item.lcsc_id().unwrap_or_else(|| item.name().into());
                        if !queries.contains_key(&id) {
                            queries.insert(id, keyword);
                            selected.push(item);
                        }
                    }
                    Err(error) => rows.push(output::export_row(
                        &keyword,
                        None,
                        Err(error.to_string()),
                        &req,
                    )),
                }
            }
            for (item, result) in export::export_items(&client, &selected, &req) {
                let id = item.lcsc_id().unwrap_or_else(|| item.name().into());
                rows.push(output::export_row(
                    &queries[&id],
                    Some(&item),
                    result.map_err(|e| e.to_string()),
                    &req,
                ));
            }
            Ok(output::exports(rows, &req))
        }
        Some(Cmd::Bom { command }) => bom::execute(command),
        Some(Cmd::Library { command }) => library::execute(command),
    }
}

/// Ranking is a suggestion. Never substitute another C-number or guess an ambiguous model.
fn select(keyword: &str, items: Vec<SearchItem>, index: Option<u32>) -> Result<SearchItem> {
    if models::looks_like_lcsc_token(keyword.trim()) {
        let id = keyword.trim().to_ascii_uppercase();
        return items
            .into_iter()
            .find(|item| {
                item.lcsc_id().as_deref() == Some(id.as_str())
                    && index.is_none_or(|index| item.index == index as usize)
            })
            .with_context(|| format!("No exact result for {id}; no substitute was selected"));
    }
    if let Some(index) = index {
        return items
            .into_iter()
            .find(|item| item.index == index as usize)
            .with_context(|| format!("Search result index {index} does not exist"));
    }
    let mut exact = items.into_iter().filter(|item| {
        item.name().eq_ignore_ascii_case(keyword.trim())
            || item.title.eq_ignore_ascii_case(keyword.trim())
    });
    let first = exact.next();
    match (first, exact.next()) {
        (Some(item), None) => Ok(item),
        _ => bail!("No unique exact match for {keyword}; run search --json, review candidates, then use an LCSC ID or get --index"),
    }
}

#[cfg(test)]
mod tests;
