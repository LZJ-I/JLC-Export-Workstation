//! C205463 / DS90LV048ATMTC_NOPB：TSSOP-16，封装里带 3D 引脚 FILL，曾经整库 Failed to load。

use lceda_core::altium::{write_pcblib, write_schlib};
use lceda_core::easyeda;
use lceda_core::ir;
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;

fn load(name: &str) -> Value {
    let text = match name {
        "symbol" => include_str!("fixtures/DS90LV048ATMTC_NOPB_symbol_easyeda.json"),
        "footprint" => include_str!("fixtures/DS90LV048ATMTC_NOPB_footprint_easyeda.json"),
        _ => panic!("unknown fixture {name}"),
    };
    serde_json::from_str(text).unwrap()
}

fn tmp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("lceda-ds90");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn skips_3d_pin_fills_and_keeps_pads_silk_and_shape() {
    let src = easyeda::parse_footprint(&load("footprint")).unwrap();
    assert_eq!(src.pads.len(), 16);
    assert!(src.pads.iter().any(|p| p.shape.eq_ignore_ascii_case("OVAL")));
    assert!(
        src.regions.iter().all(|r| r.layer == 49),
        "PIN_SOLDERING/PIN_FLOATING FILLs must not become regions: {:?}",
        src.regions.iter().map(|r| r.layer).collect::<Vec<_>>()
    );
    assert!(
        src.regions.len() <= 2,
        "marking only, got {} regions",
        src.regions.len()
    );
    assert!(
        src.tracks.iter().any(|t| t.layer == 3),
        "top silk should remain"
    );
    assert!(
        src.tracks.iter().any(|t| t.layer == 48),
        "component shape outline should remain"
    );
    assert!(
        (src.model.rot_z - 90.0).abs() < 1e-9,
        "DS90 transform is rotZ=90, got {}",
        src.model.rot_z
    );
}

#[test]
fn writes_ds90_schlib_and_pcblib() {
    let name = "DS90LV048ATMTC_NOPB";
    let sym_src = easyeda::parse_symbol(&load("symbol")).unwrap();
    let fp_src = easyeda::parse_footprint(&load("footprint")).unwrap();
    let mut sym = ir::symbol_ir(name, name, sym_src, Default::default());
    let fp = ir::footprint_ir(name, name, fp_src, Default::default());
    sym.meta.footprint_lib = fp.name.clone();

    let dir = tmp_dir();
    let sch = dir.join(format!("{name}.SchLib"));
    let pcb = dir.join(format!("{name}.PcbLib"));
    write_schlib(&sch, &sym).unwrap();
    write_pcblib(&pcb, &fp).unwrap();
    assert!(sch.metadata().unwrap().len() > 64);
    assert!(pcb.metadata().unwrap().len() > 64);

    let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&pcb).unwrap()).unwrap();
    assert!(cfb.exists("FileHeader"));
    assert!(cfb.exists(&format!("{name}/Data")));
    let mut lib = Vec::new();
    cfb.open_stream("Library/Data").unwrap().read_to_end(&mut lib).unwrap();
    let text = String::from_utf8_lossy(&lib);
    assert!(text.contains("KIND=Protel_Advanced_PCB_Library"));
    assert!(lib.len() > 10_000);
    let mut file_hdr = Vec::new();
    cfb.open_stream("FileHeader").unwrap().read_to_end(&mut file_hdr).unwrap();
    assert_eq!(file_hdr.len(), 53);
    let mut hdr = Vec::new();
    cfb.open_stream(&format!("{name}/Header"))
        .unwrap()
        .read_to_end(&mut hdr)
        .unwrap();
    let count = i32::from_le_bytes(hdr.as_slice().try_into().unwrap());
    assert!(count >= 16, "at least 16 pads, got {count}");
    assert!(count < 50, "3D pin FILLs must not inflate primitive count, got {count}");
}
