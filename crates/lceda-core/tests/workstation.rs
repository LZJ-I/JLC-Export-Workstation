//! Workstation regressions use cached real EasyEDA sources and never need the live API.
use lceda_core::{
    altium::{self, SchColors},
    easyeda,
    export::{export_items, ExportRequest},
    ir::{self, PartMeta},
    managed_ad::{self, Entry},
    models::SearchItem,
    util::sanitize_filename,
    LcedaClient,
};
use serde_json::{json, Value};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}};

const SYMBOL: &str = include_str!("fixtures/FNR3015S2R2MT_symbol_easyeda.json");
const FOOTPRINT: &str = include_str!("fixtures/FNR3015S2R2MT_footprint_easyeda.json");

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lceda-workstation-{label}-{}-{}", std::process::id(), lceda_core::util::unique_id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn cached_item(out: &Path, id: &str, name: &str) -> SearchItem {
    let item = SearchItem {
        index: 1, display_title: name.into(), title: name.into(), manufacturer: "fixture-maker".into(),
        model_uuid: None,
        raw: json!({"product_code": id, "attributes": {
            "Symbol": "fixture-symbol-local-only", "Footprint": "fixture-footprint-local-only"
        }}),
    };
    let folder = out.join(item.export_stem());
    fs::create_dir_all(&folder).unwrap();
    let base = sanitize_filename(item.name());
    fs::write(folder.join(format!("{base}_symbol_easyeda.json")), SYMBOL).unwrap();
    fs::write(folder.join(format!("{base}_footprint_easyeda.json")), FOOTPRINT).unwrap();
    item
}

#[test]
fn default_batch_with_step_exports_available_libraries_for_parts_without_models() {
    let dir = TemporaryDirectory::new("no-model");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let req = ExportRequest {
        step: true, ad: true, kicad: true, pads: true, source_json: true,
        force: false, out_dir: dir.0.clone(), ..Default::default()
    };
    let mut rows = export_items(&LcedaClient::new(), &[item], &req);
    assert_eq!(rows.len(), 1);
    let (_, result) = rows.remove(0);
    let paths = result.expect("missing optional STEP must not discard symbol/footprint formats");
    assert!(paths.step.is_none(), "no independent STEP exists for this fixture");
    for (kind, path) in [
        ("AD symbol", paths.schlib), ("AD footprint", paths.pcblib),
        ("KiCad symbol", paths.kicad_sym), ("KiCad footprint", paths.kicad_mod),
        ("PADS part", paths.pads_c), ("PADS decal", paths.pads_d), ("PADS PCB", paths.pads_p),
        ("symbol source", paths.symbol_json), ("footprint source", paths.footprint_json),
    ] {
        let path = path.unwrap_or_else(|| panic!("missing {kind}"));
        assert!(path.is_file(), "{kind}: {}", path.display());
        assert!(path.metadata().unwrap().len() > 0, "empty {kind}");
    }
}

fn managed_entry(id: &str, mpn: &str) -> Entry {
    let metadata = PartMeta { lcsc: id.into(), mpn: mpn.into(), manufacturer: "fixture-maker".into(), ..Default::default() };
    let symbol = ir::symbol_ir("same-generic-name", "fixture", easyeda::parse_symbol(&serde_json::from_str(SYMBOL).unwrap()).unwrap(), metadata.clone());
    let footprint = ir::footprint_ir("same-generic-name", "fixture", easyeda::parse_footprint(&serde_json::from_str(FOOTPRINT).unwrap()).unwrap(), metadata);
    Entry {
        id: id.into(), symbol: Some(symbol), footprint: Some(footprint),
        step: Some(format!("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('{id} model'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n").into_bytes()),
    }
}

fn read_stream(cfb: &mut cfb::CompoundFile<fs::File>, path: &str) -> Vec<u8> {
    let mut data = Vec::new();
    cfb.open_stream(path).unwrap_or_else(|e| panic!("{path}: {e}"))
        .read_to_end(&mut data).unwrap();
    data
}

fn parameter_records(data: &[u8]) -> Vec<String> {
    let mut offset = 0usize;
    let mut records = Vec::new();
    while offset + 4 <= data.len() {
        let raw = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        let length = (raw & 0x00ff_ffff) as usize;
        offset += 4;
        assert!(offset + length <= data.len(), "invalid SchLib record boundary");
        if data.get(offset) == Some(&b'|') {
            records.push(String::from_utf8_lossy(&data[offset..offset + length]).into_owned());
        }
        offset += length;
    }
    assert_eq!(offset, data.len());
    records
}

#[test]
fn managed_library_keeps_duplicate_names_distinct_with_models_binding_and_mpn_comment() {
    let dir = TemporaryDirectory::new("managed-ad");
    let first = managed_entry("C167747", "FNR3015S2R2MT");
    let second = managed_entry("C205463", "Different-MPN");
    let mut refreshed = first.clone();
    refreshed.symbol.as_mut().unwrap().description = "updated description".into();
    let expected_steps = [first.step.clone().unwrap(), second.step.clone().unwrap()];
    managed_ad::update(&dir.0, first, SchColors::default()).unwrap();
    managed_ad::update(&dir.0, second, SchColors::default()).unwrap();
    managed_ad::update(&dir.0, refreshed, SchColors::default()).unwrap();

    let catalogue: Value = serde_json::from_slice(&fs::read(dir.0.join("JLC.catalog.json")).unwrap()).unwrap();
    let entries = catalogue["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "updating an ID must replace its previous entry");
    assert_eq!(entries[0]["symbol"]["description"], "updated description");
    let mut sch = cfb::open(dir.0.join("JLC.SchLib")).unwrap();
    let mut pcb = cfb::open(dir.0.join("JLC.PcbLib")).unwrap();
    let count = read_stream(&mut pcb, "/Library/Models/Header");
    assert_eq!(i32::from_le_bytes(count.try_into().unwrap()), 2);
    for (index, (id, mpn)) in [("C167747", "FNR3015S2R2MT"), ("C205463", "Different-MPN")].iter().enumerate() {
        let records = parameter_records(&read_stream(&mut sch, &format!("/{id}/Data")));
        assert!(records.iter().any(|record| record.contains("|RECORD=41|") && record.contains("|NAME=Comment|") && record.contains(&format!("|TEXT={mpn}|"))), "Comment must retain the MPN while library names use {id}");
        assert!(records.iter().any(|record| record.contains("|MODELTYPE=PCBLIB|") && record.contains(&format!("|MODELNAME={id}|"))), "{id} symbol must bind to its own footprint");
        let footprint = read_stream(&mut pcb, &format!("/{id}/Data"));
        let text = String::from_utf8_lossy(&footprint);
        assert!(text.contains("MODEL.EMBED=TRUE"), "{id} footprint lost its 3D body");
        assert!(text.contains("MODEL.3D.ROTZ=90.000"), "{id} lost fixture model placement");
        let compressed = read_stream(&mut pcb, &format!("/Library/Models/{index}"));
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, expected_steps[index], "embedded STEP payload must survive a later component update");
    }
}

#[test]
fn merged_library_write_failure_cannot_report_successful_rows() {
    let dir = TemporaryDirectory::new("merge-failure");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    // Directory blockers fail consistently across Windows/Linux, even when tests run as root.
    fs::create_dir(dir.0.join("blocked.SchLib")).unwrap();
    fs::create_dir(dir.0.join("blocked.PcbLib")).unwrap();
    let req = ExportRequest {
        ad: true, source_json: true, merge: true, merge_name: "blocked".into(),
        force: false, out_dir: dir.0.clone(), ..Default::default()
    };
    let rows = export_items(&LcedaClient::new(), &[item], &req);
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|(_, result)| result.is_err()), "requested combined output failed, so no affected row may claim success");
    let message = rows.iter().filter_map(|(_, r)| r.as_ref().err()).map(ToString::to_string).collect::<Vec<_>>().join("\n");
    assert!(!message.is_empty());
}

const OPAQUE: &[u8] = b"custom third-party library data must survive\0\xff";

#[test]
fn mixed_ad_and_kicad_bindings_sync_only_requested_targets() {
    let dir = TemporaryDirectory::new("mixed-bound-sync");
    let first = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let second = cached_item(&dir.0, "C205463", "Other-MPN");
    let sch = dir.0.join("AD.SchLib");
    let sym = dir.0.join("KiCad.kicad_sym");
    let mut req = ExportRequest { ad: true, kicad: true, bound_only: true, out_dir: dir.0.clone(), ..Default::default() };
    req.ad_library_files.insert("C167747".into(), sch.clone());
    req.kicad_library_files.insert("C205463".into(), sym.clone());
    let rows = export_items(&LcedaClient::new(), &[first.clone(), second.clone()], &req);
    let ad = rows[0].1.as_ref().unwrap(); let ki = rows[1].1.as_ref().unwrap();
    assert_eq!(ad.schlib.as_ref(), Some(&sch)); assert!(ad.kicad_sym.is_none());
    assert_eq!(fs::canonicalize(ki.kicad_sym.as_ref().unwrap()).unwrap(), fs::canonicalize(&sym).unwrap()); assert!(ki.schlib.is_none());
    assert!(!dir.0.join(first.export_stem()).join("FNR3015S2R2MT.kicad_sym").exists());
    assert!(!dir.0.join(second.export_stem()).join("Other-MPN.SchLib").exists());
    let before = fs::read(&sym).unwrap();
    req.ad_library_files.insert("C205463".into(), sch.clone());
    req.kicad_library_files.insert("C167747".into(), sym.clone());
    let rows = export_items(&LcedaClient::new(), &[first, second], &req);
    assert!(rows.iter().all(|(_, result)| result.as_ref().is_ok_and(|p| p.schlib.is_some() && p.kicad_sym.is_some())));
    let after = fs::read_to_string(&sym).unwrap();
    assert!(after.contains("(symbol \"C167747\"")); assert!(after.contains("(symbol \"C205463\"")); assert_ne!(after.as_bytes(), before);
}

#[test]
fn independent_export_copies_from_a_bound_library_leave_both_targets_untouched() {
    let dir = TemporaryDirectory::new("export-bound-copies");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let sch = dir.0.join("Bound.SchLib"); let sym = dir.0.join("Bound.kicad_sym");
    seed_custom_library(&sch);
    fs::write(&sym, "(kicad_symbol_lib (version 20241209) (generator custom) (symbol \"User\"))").unwrap();
    let ad_before = fs::read(&sch).unwrap(); let ki_before = fs::read(&sym).unwrap();
    let mut req = ExportRequest { ad: true, kicad: true, sync_bindings: false, out_dir: dir.0.clone(), ..Default::default() };
    req.ad_library_files.insert("C167747".into(), sch.clone()); req.kicad_library_files.insert("C167747".into(), sym.clone());
    let rows = export_items(&LcedaClient::new(), &[item], &req); let p = rows[0].1.as_ref().unwrap();
    assert_ne!(p.schlib.as_ref(), Some(&sch)); assert_ne!(p.kicad_sym.as_ref(), Some(&sym));
    assert!(p.schlib.as_ref().unwrap().is_file()); assert!(p.kicad_sym.as_ref().unwrap().is_file());
    assert_eq!(ad_before, fs::read(&sch).unwrap()); assert_eq!(ki_before, fs::read(&sym).unwrap());
}

#[test]
fn mixed_bound_and_unbound_kicad_merge_keeps_targets_and_prevents_clobbering() {
    let dir = TemporaryDirectory::new("mixed-kicad-merge");
    let first = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let second = cached_item(&dir.0, "C205463", "Other-MPN");
    let bound = dir.0.join("bound.kicad_sym");
    let mut req = ExportRequest { kicad: true, merge: true, merge_name: "combined".into(), out_dir: dir.0.clone(), ..Default::default() };
    req.kicad_library_files.insert("C167747".into(), bound.clone());
    let rows = export_items(&LcedaClient::new(), &[first.clone(), second.clone()], &req);
    assert_eq!(fs::canonicalize(rows[0].1.as_ref().unwrap().kicad_sym.as_ref().unwrap()).unwrap(), fs::canonicalize(&bound).unwrap());
    assert_eq!(rows[1].1.as_ref().unwrap().kicad_sym.as_ref(), Some(&dir.0.join("combined.kicad_sym")));
    assert!(!fs::read_to_string(dir.0.join("combined.kicad_sym")).unwrap().contains("(symbol \"FNR3015S2R2MT\""));
    let before = fs::read(&bound).unwrap();
    req.merge_name = "bound".into();
    let rows = export_items(&LcedaClient::new(), &[second], &req);
    assert!(rows[0].1.is_err()); assert_eq!(before, fs::read(&bound).unwrap());
}

fn seed_custom_library(sch: &Path) {
    fs::create_dir_all(sch.parent().unwrap()).unwrap();
    let mut entry = managed_entry("CustomOriginal", "Custom-MPN");
    let symbol = entry.symbol.as_mut().unwrap();
    symbol.name = "CustomOriginal".into();
    let footprint = entry.footprint.as_mut().unwrap();
    footprint.name = "CustomOriginal".into();
    altium::write_schlib(sch, symbol).unwrap();
    altium::write_pcblib_with_step(&sch.with_extension("PcbLib"), footprint, entry.step.as_deref()).unwrap();
    for path in [sch.to_path_buf(), sch.with_extension("PcbLib")] {
        let mut file = cfb::open_rw(path).unwrap();
        file.create_storage_all("/UserOpaque").unwrap();
        file.create_stream("/UserOpaque/PrivatePayload").unwrap().write_all(OPAQUE).unwrap();
        file.flush().unwrap();
    }
}

fn assert_original_preserved(sch: &Path) {
    for path in [sch.to_path_buf(), sch.with_extension("PcbLib")] {
        let mut file = cfb::open(path).unwrap();
        assert!(file.exists("/CustomOriginal/Data"), "custom component disappeared");
        assert_eq!(read_stream(&mut file, "/UserOpaque/PrivatePayload"), OPAQUE);
    }
}

#[test]
fn binding_at_ordinary_output_path_updates_without_rebuilding_original_library() {
    let dir = TemporaryDirectory::new("bound-at-output");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let sch = dir.0.join(item.export_stem()).join(format!("{}.SchLib", sanitize_filename(item.name())));
    seed_custom_library(&sch);
    let mut req = ExportRequest { ad: true, out_dir: dir.0.clone(), ..Default::default() };
    req.ad_library_files.insert("C167747".into(), sch.clone());
    let rows = export_items(&LcedaClient::new(), &[item], &req);
    let paths = rows[0].1.as_ref().unwrap();
    assert_eq!(paths.schlib.as_ref(), Some(&sch));
    assert_original_preserved(&sch);
    assert!(cfb::open(&sch).unwrap().exists("/C167747/Data"));
    assert!(cfb::open(sch.with_extension("PcbLib")).unwrap().exists("/C167747/Data"));
}

#[test]
fn bound_only_ad_merge_keeps_bound_paths_and_still_merges_kicad() {
    for kicad in [false, true] {
        let dir = TemporaryDirectory::new("bound-only-merge");
        let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
        let sch = dir.0.join("combined.SchLib");
        seed_custom_library(&sch);
        let mut req = ExportRequest {
            ad: true, kicad, merge: true, merge_name: "combined".into(),
            out_dir: dir.0.clone(), ..Default::default()
        };
        req.ad_library_files.insert("C167747".into(), sch.clone());
        let rows = export_items(&LcedaClient::new(), &[item], &req);
        let paths = rows[0].1.as_ref().unwrap();
        assert_eq!(paths.schlib.as_ref(), Some(&sch));
        assert_eq!(paths.pcblib.as_ref(), Some(&sch.with_extension("PcbLib")));
        if kicad {
            assert_eq!(paths.kicad_sym.as_ref(), Some(&dir.0.join("combined.kicad_sym")));
            assert!(paths.kicad_mod.as_ref().unwrap().is_file());
        }
        assert_original_preserved(&sch);
    }
}

#[test]
fn merged_pcb_failure_does_not_claim_success_when_symbol_output_exists() {
    let dir = TemporaryDirectory::new("partial-merge-failure");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    fs::create_dir(dir.0.join("partial.PcbLib")).unwrap();
    let req = ExportRequest {
        ad: true, merge: true, merge_name: "partial".into(),
        out_dir: dir.0.clone(), ..Default::default()
    };
    let rows = export_items(&LcedaClient::new(), &[item], &req);
    assert!(dir.0.join("partial.SchLib").is_file());
    assert!(rows[0].1.is_err(), "one written format cannot hide failure of another requested AD library");
}

#[test]
fn mixed_merge_collision_rejects_unbound_output_and_preserves_bound_originals() {
    let dir = TemporaryDirectory::new("mixed-collision");
    let bound = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let unbound = cached_item(&dir.0, "C205463", "Different-MPN");
    let sch = dir.0.join("combined.SchLib");
    seed_custom_library(&sch);
    let mut req = ExportRequest {
        ad: true, merge: true, merge_name: "combined".into(), out_dir: dir.0.clone(), ..Default::default()
    };
    req.ad_library_files.insert("C167747".into(), sch.clone());
    let rows = export_items(&LcedaClient::new(), &[bound, unbound], &req);
    assert!(rows.iter().all(|(_, result)| result.as_ref().err().is_some_and(|error| error.to_string().contains("冲突"))));
    assert_original_preserved(&sch);
    assert!(cfb::open(&sch).unwrap().exists("/C167747/Data"));
    assert!(!cfb::open(&sch).unwrap().exists("/Different-MPN/Data"));
}

#[test]
fn ordinary_export_protects_another_binding_even_through_case_and_relative_aliases() {
    let dir = TemporaryDirectory::new("reserved-case-alias");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let sch = dir.0.join(item.export_stem()).join(format!("{}.SchLib", sanitize_filename(item.name())));
    seed_custom_library(&sch);
    let before_sch = fs::read(&sch).unwrap();
    let before_pcb = fs::read(sch.with_extension("PcbLib")).unwrap();
    // The file need not already exist with this exact case on a Windows/shared volume.
    let cwd = std::env::current_dir().unwrap();
    let reserved = if let Some(common) = cwd.ancestors().find(|ancestor| sch.starts_with(ancestor)) {
        let mut relative = PathBuf::new();
        for _ in cwd.strip_prefix(common).unwrap().components() { relative.push(".."); }
        relative.join(sch.strip_prefix(common).unwrap()).with_extension("sChLiB")
    } else { sch.with_extension("sChLiB") };
    let mut req = ExportRequest { ad: true, out_dir: dir.0.clone(), ..Default::default() };
    req.ad_library_files.insert("C999999".into(), reserved);
    let rows = export_items(&LcedaClient::new(), &[item], &req);
    assert!(rows[0].1.as_ref().err().unwrap().to_string().contains("冲突"));
    assert_eq!(fs::read(&sch).unwrap(), before_sch);
    assert_eq!(fs::read(sch.with_extension("PcbLib")).unwrap(), before_pcb);
}

#[cfg(unix)]
#[test]
fn ordinary_export_protects_a_binding_through_a_canonical_directory_alias() {
    let dir = TemporaryDirectory::new("reserved-symlink");
    let item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
    let part_dir = dir.0.join(item.export_stem());
    let sch = part_dir.join(format!("{}.SchLib", sanitize_filename(item.name())));
    seed_custom_library(&sch);
    let alias = dir.0.join("alias");
    std::os::unix::fs::symlink(&part_dir, &alias).unwrap();
    let mut req = ExportRequest { ad: true, out_dir: dir.0.clone(), ..Default::default() };
    req.ad_library_files.insert("C999999".into(), alias.join(sch.file_name().unwrap()));
    let rows = export_items(&LcedaClient::new(), &[item], &req);
    assert!(rows[0].1.as_ref().err().unwrap().to_string().contains("冲突"));
    assert_original_preserved(&sch);
}

#[test]
fn separate_step_does_not_embed_when_ad_embedding_is_disabled() {
    for mode in ["ordinary", "merged", "bound", "managed"] {
        let dir = TemporaryDirectory::new(&format!("embed-off-{mode}"));
        let mut item = cached_item(&dir.0, "C167747", "FNR3015S2R2MT");
        item.model_uuid = Some("cached-model-local-only".into());
        let base = sanitize_filename(item.name());
        let step_path = dir.0.join(item.export_stem()).join(format!("{base}.step"));
        let step = managed_entry("C167747", "FNR3015S2R2MT").step.unwrap();
        fs::write(&step_path, &step).unwrap();
        let mut req = ExportRequest {
            step: true, ad: true, ad_embed_3d: false, out_dir: dir.0.clone(), ..Default::default()
        };
        match mode {
            "merged" => req.merge = true,
            "bound" => { req.ad_library_files.insert("C167747".into(), dir.0.join("bound.SchLib")); },
            "managed" => req.ad_library_dir = Some(dir.0.join("managed")),
            _ => {},
        }
        let rows = export_items(&LcedaClient::new(), &[item], &req);
        let paths = rows[0].1.as_ref().unwrap_or_else(|error| panic!("{mode}: {error}"));
        assert_eq!(paths.step.as_ref(), Some(&step_path));
        assert_eq!(fs::read(&step_path).unwrap(), step);
        let mut pcb = cfb::open(paths.pcblib.as_ref().unwrap()).unwrap();
        let header = read_stream(&mut pcb, "/Library/Models/Header");
        assert_eq!(i32::from_le_bytes(header.try_into().unwrap()), 0, "{mode}: independent STEP must not bypass the AD embedding toggle");
    }
}
