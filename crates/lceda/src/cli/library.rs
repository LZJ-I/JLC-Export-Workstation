use super::output::Outcome;
use anyhow::{bail, Result};
use clap::Subcommand;
use lceda_core::native_library::{self, LibraryFormat};
use serde_json::json;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(super) enum Command {
    /// Inspect a .SchLib or .kicad_sym: names, properties, footprints, pins and units.
    Inspect { file: PathBuf },
}

pub(super) fn execute(command: Command) -> Result<Outcome> {
    let Command::Inspect { file } = command;
    let extension = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let format = if extension.eq_ignore_ascii_case("SchLib") {
        LibraryFormat::Ad
    } else if extension.eq_ignore_ascii_case("kicad_sym") {
        LibraryFormat::KiCad
    } else {
        bail!("library inspect accepts .SchLib or .kicad_sym files");
    };
    let library = native_library::inspect(&file, format)?;
    let mut text = format!(
        "{}: {} parts ({})",
        file.display(),
        library.parts.len(),
        format.name()
    );
    for part in &library.parts {
        text.push_str(&format!(
            "\n  {} | {} pins | {} units | {}",
            part.name,
            part.pins,
            part.units,
            part.footprints.join(", ")
        ));
    }
    Ok(Outcome::success(
        json!({"file": file, "format": format, "total": library.parts.len(), "parts": library.parts}),
        text,
    ))
}
