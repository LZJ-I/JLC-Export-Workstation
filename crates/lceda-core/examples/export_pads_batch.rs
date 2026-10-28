//! Live PADS export using the same entry point as the application.
//! cargo run -p lceda-core --example export_pads_batch -- OUT_DIR QUERY...
use lceda_core::{export::{export, ExportRequest}, models::SearchItem, util::sanitize_filename, LcedaClient};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out_dir = PathBuf::from(args.next().ok_or("expected OUT_DIR QUERY...")?);
    fs::create_dir_all(&out_dir)?;
    let client = LcedaClient::new();
    let req = ExportRequest { pads: true, source_json: true, out_dir: out_dir.clone(), ..Default::default() };
    let mut manifest = Vec::new();
    for query in args {
        let selection = out_dir.join(format!("{}.selection.json", sanitize_filename(&query)));
        let item = if selection.exists() {
            let value: serde_json::Value = serde_json::from_slice(&fs::read(&selection)?)?;
            SearchItem { index: 1, display_title: value["display_title"].as_str().unwrap().into(),
                title: value["title"].as_str().unwrap().into(), manufacturer: value["manufacturer"].as_str().unwrap().into(),
                model_uuid: None, raw: value["raw"].clone() }
        } else {
            client.search(&query)?.into_iter().find(|i| i.name().eq_ignore_ascii_case(&query)
                || i.lcsc_id().is_some_and(|id| id.eq_ignore_ascii_case(&query)))
                .ok_or_else(|| format!("no exact match for {query}"))?
        };
        println!("Exporting {query}: {}", item.export_stem());
        // Store selection so a later run can regenerate without repeating search.
        fs::write(selection, serde_json::to_vec_pretty(&serde_json::json!({
            "display_title":item.display_title,"title":item.title,"manufacturer":item.manufacturer,"raw":item.raw
        }))?)?;
        let paths = export(&client, &item, &req)?;
        let symbol_path = paths.symbol_json.as_ref().ok_or("missing symbol source")?;
        let raw = serde_json::from_slice(&fs::read(symbol_path)?)?;
        let source = lceda_core::easyeda::parse_symbol(&raw)?;
        let c = paths.pads_c.ok_or("missing schematic decal")?;
        let d = paths.pads_d.ok_or("missing PCB decal")?;
        let p = paths.pads_p.ok_or("missing part type")?;
        manifest.push(serde_json::json!({"query":query,"name":item.name(),"lcsc":item.lcsc_id(),
            "pins":source.pins.len(),"c":c,"d":d,"p":p}));
        fs::write(out_dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    }
    Ok(())
}
