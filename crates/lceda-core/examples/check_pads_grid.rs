//! Offline regression/export pack from export_pads_batch's saved selections.
//! cargo run -p lceda-core --example check_pads_grid -- CACHE_DIR OUT_DIR
use lceda_core::{export::{export_items, ExportRequest}, models::SearchItem, pads::SchematicGrid,
    util::sanitize_filename, LcedaClient};
use std::{fs, path::{Path, PathBuf}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 { return Err("expected CACHE_DIR OUT_DIR".into()); }
    let cache = Path::new(&args[0]);
    let out = PathBuf::from(&args[1]);
    let mut selections: Vec<_> = fs::read_dir(cache)?.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".selection.json")).collect();
    selections.sort();
    if selections.is_empty() { return Err("no cached selections".into()); }
    let mut items = Vec::new();
    for selection in selections {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(selection)?)?;
        items.push(SearchItem { index: items.len() + 1, display_title: v["display_title"].as_str().unwrap().into(),
            title: v["title"].as_str().unwrap().into(), manufacturer: v["manufacturer"].as_str().unwrap().into(),
            model_uuid: None, raw: v["raw"].clone() });
    }
    let client = LcedaClient::new();
    let mut report = Vec::new();
    for mils in [10, 25, 50, 100] {
        let dir = out.join(format!("{mils}mil"));
        for item in &items {
            let base = sanitize_filename(item.name());
            let part = dir.join(item.export_stem());
            fs::create_dir_all(&part)?;
            for kind in ["symbol", "footprint"] {
                let file = format!("{base}_{kind}_easyeda.json");
                // Both must exist before calling the exporter: this is offline.
                fs::copy(cache.join(item.export_stem()).join(&file), part.join(file))?;
            }
        }
        let req = ExportRequest { pads: true, source_json: true, out_dir: dir,
            pads_grid: SchematicGrid::new(mils)?, ..Default::default() };
        for (item, (_, result)) in items.iter().zip(export_items(&client, &items, &req)) {
            let paths = result?;
            let c = paths.pads_c.ok_or("missing schematic")?;
            let text = fs::read_to_string(&c)?;
            let mut terminals = 0;
            for line in text.lines().filter(|s| s.starts_with('T') && s.ends_with(" PIN")) {
                let mut coords = line[1..].split_whitespace().take(2).map(|s| s.parse::<f64>().unwrap());
                for coord in &mut coords { assert_eq!(coord.rem_euclid(f64::from(mils)), 0.0, "{}: {line}", c.display()); }
                terminals += 1;
            }
            assert!(terminals > 0);
            let baseline = cache.join(item.export_stem());
            for path in [Some(&c), paths.pads_d.as_ref(), paths.pads_p.as_ref()].into_iter().flatten() {
                if mils == 10 || path.extension().is_some_and(|s| s != "c") {
                    assert_eq!(fs::read(path)?, fs::read(baseline.join(path.file_name().unwrap()))?,
                        "unexpected baseline change: {}", path.display());
                }
            }
            println!("{mils} mil: {} ({terminals} terminals)", item.name());
            report.push(serde_json::json!({"name":item.name(),"grid_mil":mils,"terminals":terminals,
                "c":c,"d":paths.pads_d,"p":paths.pads_p,"terminals_on_grid":true,
                "footprint_and_mapping_unchanged":true,"default_matches_baseline":mils==10}));
        }
    }
    fs::write(out.join("grid-checks.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
