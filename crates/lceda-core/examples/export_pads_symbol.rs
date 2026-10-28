//! Export a saved EasyEDA symbol through the production PADS writer.
//! Usage: cargo run -p lceda-core --example export_pads_symbol -- SOURCE.json OUT_DIR NAME
use lceda_core::{easyeda, ir, pads};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("expected SOURCE.json OUT_DIR NAME".into());
    }
    let source = serde_json::from_slice(&fs::read(&args[0])?)?;
    let name = args[2].to_str().ok_or("invalid name")?;
    let symbol = ir::symbol_ir(name, "", easyeda::parse_symbol(&source)?, ir::PartMeta::default());
    let dir = PathBuf::from(&args[1]);
    pads::write_part_files_with_source(&dir, name, Some(&symbol), None, Some(&source))?;
    println!("Exported {} terminals to {}", symbol.pins.len(), dir.display());
    Ok(())
}
