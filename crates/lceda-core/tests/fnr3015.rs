//! C167747 / FNR3015S2R2MT：曾经打不开 AD 库的真实器件。

use lceda_core::altium::{write_pcblib, write_schlib};
use lceda_core::easyeda;
use lceda_core::ir::{self, SYMBOL_UNIT_MM};
use lceda_core::kicad;
use lceda_core::pads;
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;

fn load(name: &str) -> Value {
    let text = match name {
        "symbol" => include_str!("fixtures/FNR3015S2R2MT_symbol_easyeda.json"),
        "footprint" => include_str!("fixtures/FNR3015S2R2MT_footprint_easyeda.json"),
        _ => panic!("unknown fixture {name}"),
    };
    serde_json::from_str(text).unwrap()
}

fn tmp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("lceda-fnr3015");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn parses_fnr3015_symbol_and_snaps_pins() {
    let src = easyeda::parse_symbol(&load("symbol")).unwrap();
    assert_eq!(src.pins.len(), 2);
    let ir = ir::symbol_ir("FNR3015S2R2MT", "inductor", src, Default::default());
    assert_eq!(ir.pins.len(), 2);
    assert!(!ir.polys.is_empty(), "coil outline should parse as polylines");
    assert!(ir.rects.is_empty(), "inductor BBOX is not a body rectangle");
    assert!(
        ir.pins.iter().all(|p| !p.show_name && !p.show_number),
        "FNR NAME/NUMBER valueVisible=false"
    );
    let tip = 20.0 * SYMBOL_UNIT_MM;
    for pin in &ir.pins {
        assert!(
            pin.x.abs() < tip - 0.05,
            "pin {} still at EasyEDA tip x={}",
            pin.number,
            pin.x
        );
        assert!(pin.length >= 2.54 - 1e-9);
    }
    let left = ir.pins.iter().find(|p| p.number == "1").unwrap();
    let right = ir.pins.iter().find(|p| p.number == "2").unwrap();
    assert!(left.x < 0.0);
    assert!(right.x > 0.0);
}

#[test]
fn parses_fnr3015_footprint_pads_and_courtyard() {
    let src = easyeda::parse_footprint(&load("footprint")).unwrap();
    assert_eq!(src.pads.len(), 2);
    assert!(src.pads.iter().all(|p| p.shape.eq_ignore_ascii_case("RECT")));
    assert!(
        src.tracks.len() >= 4,
        "silk/courtyard POLY should become tracks, got {}",
        src.tracks.len()
    );
    let ir = ir::footprint_ir("FNR3015S2R2MT", "inductor", src, Default::default());
    assert!(
        (ir.model.rot_z - 90.0).abs() < 1e-9,
        "FNR transform is rotZ=90, got {}",
        ir.model.rot_z
    );
    assert_eq!(ir.pads.len(), 2);
    assert!(ir.pads.iter().all(|p| p.polygon.is_none()));
    assert!(
        ir.regions.iter().all(|r| matches!(r.layer, 3 | 4 | 12 | 13 | 49)),
        "3D body FILLs on 50/51 must not become PCB regions"
    );
}

#[test]
fn writes_fnr3015_altium_kicad_and_pads() {
    let sym_src = easyeda::parse_symbol(&load("symbol")).unwrap();
    let fp_src = easyeda::parse_footprint(&load("footprint")).unwrap();
    let mut sym = ir::symbol_ir("FNR3015S2R2MT", "inductor", sym_src, Default::default());
    let fp = ir::footprint_ir("FNR3015S2R2MT", "inductor", fp_src, Default::default());
    sym.meta.footprint_lib = fp.name.clone();

    let dir = tmp_dir();
    let sch = dir.join("FNR3015S2R2MT.SchLib");
    let pcb = dir.join("FNR3015S2R2MT.PcbLib");
    write_schlib(&sch, &sym).unwrap();
    write_pcblib(&pcb, &fp).unwrap();
    assert!(sch.metadata().unwrap().len() > 64);
    assert!(pcb.metadata().unwrap().len() > 64);

    let cfb = cfb::CompoundFile::open(std::fs::File::open(&sch).unwrap()).unwrap();
    assert!(cfb.exists("FileHeader"));
    assert!(cfb.exists("FNR3015S2R2MT/Data"));

    let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&pcb).unwrap()).unwrap();
    assert!(cfb.exists("FileHeader"));
    assert!(cfb.exists("Library/ModelsNoEmbed/Header"));
    assert!(cfb.exists("FNR3015S2R2MT/Data"));
    assert!(cfb.exists("FNR3015S2R2MT/UniqueIdPrimitiveInformation/Data"));
    let mut hdr = Vec::new();
    cfb.open_stream("FNR3015S2R2MT/Header")
        .unwrap()
        .read_to_end(&mut hdr)
        .unwrap();
    let count = i32::from_le_bytes(hdr.as_slice().try_into().unwrap());
    assert!(count >= 2, "at least two pads, got {count}");
    let mut uid = Vec::new();
    cfb.open_stream("FNR3015S2R2MT/UniqueIdPrimitiveInformation/Data")
        .unwrap()
        .read_to_end(&mut uid)
        .unwrap();
    let uid_text = String::from_utf8_lossy(&uid);
    assert!(uid_text.contains("Pad"));
    assert!(uid_text.contains("Track"));

    let k_sym = dir.join("FNR3015S2R2MT.kicad_sym");
    let k_mod = dir.join("FNR3015S2R2MT.kicad_mod");
    kicad::write_symbol_lib(&k_sym, &sym).unwrap();
    kicad::write_footprint_mod(&k_mod, &fp, None).unwrap();
    let ktext = std::fs::read_to_string(&k_sym).unwrap();
    assert!(ktext.contains("FNR3015S2R2MT"));
    assert!(ktext.contains("(pin"));

    pads::write_part_files(&dir, "FNR3015S2R2MT", Some(&sym), Some(&fp)).unwrap();
}

#[test]
fn fnr3015_header_count_matches_unique_id_entries() {
    let fp_src = easyeda::parse_footprint(&load("footprint")).unwrap();
    let fp = ir::footprint_ir("FNR3015S2R2MT", "inductor", fp_src, Default::default());
    let pcb = tmp_dir().join("FNR3015S2R2MT-count.PcbLib");
    write_pcblib(&pcb, &fp).unwrap();
    let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&pcb).unwrap()).unwrap();
    let mut hdr = Vec::new();
    cfb.open_stream("FNR3015S2R2MT/Header")
        .unwrap()
        .read_to_end(&mut hdr)
        .unwrap();
    let count = i32::from_le_bytes(hdr.as_slice().try_into().unwrap());
    let mut uid = Vec::new();
    cfb.open_stream("FNR3015S2R2MT/UniqueIdPrimitiveInformation/Data")
        .unwrap()
        .read_to_end(&mut uid)
        .unwrap();
    let uid_text = String::from_utf8_lossy(&uid);
    let named = uid_text.matches("PRIMITIVEOBJECTID").count();
    assert_eq!(named as i32, count, "UniqueId entries must match Header count");
}
