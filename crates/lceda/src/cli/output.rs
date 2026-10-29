use lceda_core::{
    export::ExportRequest,
    models::{DownloadPaths, SearchItem},
};
use serde_json::{json, Value};

pub(super) struct Outcome {
    pub data: Value,
    pub text: String,
    pub code: i32,
}

impl Outcome {
    pub fn success(data: Value, text: String) -> Self {
        Self {
            data,
            text,
            code: 0,
        }
    }
}

pub(super) fn item(item: &SearchItem) -> Value {
    json!({
        "index": item.index, "lcsc": item.lcsc_id(), "mpn": item.name(),
        "manufacturer": item.manufacturer_label(), "package": crate::library::package_from_item(item),
        "description": item.product_description(),
        "available": {"symbol": item.symbol_uuid().is_some(), "footprint": item.footprint_uuid().is_some(),
            "model3d": item.model_uuid.is_some(), "datasheet": item.datasheet_url().is_some()},
    })
}

pub(super) fn paths(paths: &DownloadPaths) -> Value {
    json!({
        "folder": paths.folder, "step": paths.step, "obj": paths.obj, "mtl": paths.mtl,
        "datasheet": paths.datasheet, "symbol_json": paths.symbol_json, "footprint_json": paths.footprint_json,
        "schlib": paths.schlib, "pcblib": paths.pcblib, "ad_sync": paths.ad_sync,
        "kicad_sym": paths.kicad_sym, "kicad_mod": paths.kicad_mod,
        "kicad_pretty": paths.kicad_mod.as_ref().and_then(|p| p.parent()),
        "pads_c": paths.pads_c, "pads_d": paths.pads_d, "pads_p": paths.pads_p,
    })
}

pub(super) fn export_row(
    query: &str,
    selected: Option<&SearchItem>,
    result: Result<DownloadPaths, String>,
    req: &ExportRequest,
) -> Value {
    match result {
        Err(error) => {
            json!({"query": query, "item": selected.map(item), "status": "error", "error": error, "paths": null, "missing": []})
        }
        Ok(files) => {
            let mut missing = Vec::new();
            for (enabled, present, name) in [
                (req.step, files.step.is_some(), "step"),
                (req.obj, files.obj.is_some(), "obj"),
                (req.obj, files.mtl.is_some(), "mtl"),
                (req.datasheet, files.datasheet.is_some(), "datasheet"),
                (
                    req.ad,
                    files.schlib.is_some() || files.pcblib.is_some(),
                    "ad",
                ),
                (
                    req.kicad,
                    files.kicad_sym.is_some() || files.kicad_mod.is_some(),
                    "kicad",
                ),
                (
                    req.pads,
                    files.pads_c.is_some() || files.pads_d.is_some() || files.pads_p.is_some(),
                    "pads",
                ),
                (
                    req.source_json,
                    files.symbol_json.is_some() || files.footprint_json.is_some(),
                    "source",
                ),
            ] {
                if enabled && !present {
                    missing.push(name);
                }
            }
            // Do not call a requested library complete when one available side failed to parse/write.
            if let Some(selected) = selected {
                for (enabled, present, name) in [
                    (
                        req.ad && selected.symbol_uuid().is_some(),
                        files.schlib.is_some(),
                        "schlib",
                    ),
                    (
                        req.ad && selected.footprint_uuid().is_some(),
                        files.pcblib.is_some(),
                        "pcblib",
                    ),
                    (
                        req.kicad && selected.symbol_uuid().is_some(),
                        files.kicad_sym.is_some(),
                        "kicad_sym",
                    ),
                    (
                        req.kicad && selected.footprint_uuid().is_some(),
                        files.kicad_mod.is_some(),
                        "kicad_mod",
                    ),
                    (
                        req.pads && selected.symbol_uuid().is_some(),
                        files.pads_c.is_some(),
                        "pads_c",
                    ),
                    (
                        req.pads && selected.footprint_uuid().is_some(),
                        files.pads_d.is_some(),
                        "pads_d",
                    ),
                    (
                        req.pads && selected.has_symbol_or_footprint(),
                        files.pads_p.is_some(),
                        "pads_p",
                    ),
                    (
                        req.source_json && selected.symbol_uuid().is_some(),
                        files.symbol_json.is_some(),
                        "symbol_json",
                    ),
                    (
                        req.source_json && selected.footprint_uuid().is_some(),
                        files.footprint_json.is_some(),
                        "footprint_json",
                    ),
                ] {
                    if enabled && !present {
                        missing.push(name);
                    }
                }
            }
            json!({"query": query, "item": selected.map(item), "status": if missing.is_empty() { "ok" } else { "partial" },
                "error": null, "paths": paths(&files), "missing": missing})
        }
    }
}

pub(super) fn exports(rows: Vec<Value>, req: &ExportRequest) -> Outcome {
    let failed = rows.iter().filter(|row| row["status"] != "ok").count();
    let mut text = String::new();
    for row in &rows {
        text.push_str(&format!(
            "{} {}\n",
            row["status"].as_str().unwrap_or("error").to_uppercase(),
            row["query"].as_str().unwrap_or("")
        ));
        if let Some(error) = row["error"].as_str() {
            text.push_str(&format!("  {error}\n"));
        }
        if let Some(paths) = row["paths"].as_object() {
            for (kind, path) in paths {
                if let Some(path) = path.as_str() {
                    text.push_str(&format!("  {kind}: {path}\n"));
                }
            }
        }
        if row["status"] == "partial" {
            text.push_str(&format!(
                "  Missing requested outputs: {}\n",
                row["missing"]
            ));
        }
    }
    text.push_str(&format!(
        "{} complete, {failed} failed or partial",
        rows.len() - failed
    ));
    Outcome {
        code: if failed == 0 && !rows.is_empty() {
            0
        } else {
            1
        },
        text,
        data: json!({"output": req.out_dir, "merged": req.merge, "name": req.merge_name,
            "total": rows.len(), "succeeded": rows.len() - failed, "failed": failed, "results": rows}),
    }
}

pub(super) fn emit(command: &str, json_mode: bool, outcome: Outcome) -> anyhow::Result<i32> {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&json!({"schema_version": 1, "command": command,
            "ok": outcome.code == 0, "data": outcome.data,
            "error": if outcome.code == 0 { Value::Null } else { json!({"code": "incomplete", "message": "Inspect data for unresolved rows or failed outputs"}) }}))?
        );
    } else {
        println!("{}", outcome.text);
    }
    Ok(outcome.code)
}

pub(super) fn failure(command: &str, json_mode: bool, error: &str) -> anyhow::Result<i32> {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&json!({"schema_version": 1, "command": command,
            "ok": false, "data": null, "error": {"code": "command_failed", "message": error}}))?
        );
    } else {
        eprintln!("Error: {error}");
    }
    Ok(2)
}
