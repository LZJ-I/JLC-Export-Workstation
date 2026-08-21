pub mod binary;
pub mod pcblib;
pub mod schlib;

use crate::error::Result;
use crate::ir::{FootprintIr, SymbolIr};
use std::path::Path;

pub fn write_schlib(path: &Path, symbol: &SymbolIr) -> Result<()> {
    schlib::write(path, symbol)
}

pub fn write_schlib_many(path: &Path, symbols: &[&SymbolIr]) -> Result<()> {
    schlib::write_many(path, symbols)
}

pub fn write_pcblib(path: &Path, footprint: &FootprintIr) -> Result<()> {
    pcblib::write(path, footprint)
}

pub fn write_pcblib_with_step(path: &Path, footprint: &FootprintIr, step: Option<&[u8]>) -> Result<()> {
    pcblib::write_with_step(path, footprint, step)
}

pub fn write_pcblib_library(path: &Path, parts: &[pcblib::PcbLibPart<'_>]) -> Result<()> {
    pcblib::write_library(path, parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{FootprintIr, IrPad, IrPin, IrRect, IrRegion, IrTrack, SymbolIr};
    use std::io::Read;

    #[test]
    fn writes_schlib_compound_file() {
        let dir = std::env::temp_dir().join("lceda-test-sch");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("RES.SchLib");
        let symbol = SymbolIr {
            name: "RES".into(),
            description: "test".into(),
            meta: Default::default(),
            pins: vec![IrPin {
                number: "1".into(),
                name: "1".into(),
                x: 0.0,
                y: 5.08,
                length: 2.54,
                rotation: 270.0,
                pin_type: String::new(),
            }],
            rects: vec![IrRect {
                x1: -1.0,
                y1: -2.54,
                x2: 1.0,
                y2: 2.54,
            }],
            polys: vec![],
            ellipses: vec![],
        };
        write_schlib(&path, &symbol).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        assert!(cfb.exists("FileHeader"));
        assert!(cfb.exists("RES/Data"));
        let mut hdr = Vec::new();
        cfb.open_stream("FileHeader").unwrap().read_to_end(&mut hdr).unwrap();
        assert!(hdr.len() > 32);
    }

    #[test]
    fn writes_pcblib_compound_file() {
        let dir = std::env::temp_dir().join("lceda-test-pcb");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("R0402.PcbLib");
        let fp = FootprintIr {
            name: "R0402".into(),
            description: "0402".into(),
            meta: Default::default(),
            pads: vec![
                IrPad {
                    designator: "1".into(),
                    x: -0.5,
                    y: 0.0,
                    width: 0.6,
                    height: 0.8,
                    hole: 0.0,
                    hole_slot: 0.0,
                    hole_shape: "ROUND".into(),
                    rotation: 0.0,
                    layer: 1,
                    shape: "RECT".into(),
                    polygon: None,
                },
                IrPad {
                    designator: "2".into(),
                    x: 0.5,
                    y: 0.0,
                    width: 0.6,
                    height: 0.8,
                    hole: 0.0,
                    hole_slot: 0.0,
                    hole_shape: "ROUND".into(),
                    rotation: 0.0,
                    layer: 1,
                    shape: "RECT".into(),
                    polygon: None,
                },
            ],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
        };
        write_pcblib(&path, &fp).unwrap();
        let cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        assert!(cfb.exists("FileHeader"));
        assert!(cfb.exists("Library/Data"));
        assert!(cfb.exists("Library/ModelsNoEmbed/Header"));
        assert!(cfb.exists("R0402/Data"));
        assert!(cfb.exists("R0402/UniqueIdPrimitiveInformation/Data"));
    }

    #[test]
    fn pcblib_header_matches_tracks_and_regions() {
        let dir = std::env::temp_dir().join("lceda-test-pcb-count");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("IND.PcbLib");
        let fp = FootprintIr {
            name: "IND".into(),
            description: "inductor | LCSC C1".into(),
            meta: Default::default(),
            pads: vec![
                IrPad {
                    designator: "1".into(),
                    x: -1.0,
                    y: 0.0,
                    width: 1.2,
                    height: 1.5,
                    hole: 0.0,
                    hole_slot: 0.0,
                    hole_shape: "ROUND".into(),
                    rotation: 0.0,
                    layer: 1,
                    shape: "RECT".into(),
                    polygon: None,
                },
                IrPad {
                    designator: "2".into(),
                    x: 1.0,
                    y: 0.0,
                    width: 1.2,
                    height: 1.5,
                    hole: 0.0,
                    hole_slot: 0.0,
                    hole_shape: "ROUND".into(),
                    rotation: 0.0,
                    layer: 1,
                    shape: "RECT".into(),
                    polygon: None,
                },
            ],
            tracks: vec![IrTrack {
                layer: 3,
                width: 0.15,
                points: vec![(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0), (-2.0, -2.0)],
            }],
            circles: vec![],
            arcs: vec![],
            regions: vec![IrRegion {
                layer: 13,
                points: vec![(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)],
            }],
        };
        write_pcblib(&path, &fp).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut hdr = Vec::new();
        cfb.open_stream("IND/Header").unwrap().read_to_end(&mut hdr).unwrap();
        assert_eq!(hdr.len(), 4);
        let count = i32::from_le_bytes(hdr.try_into().unwrap());
        // 2 pads + 4 courtyard track segments + 1 region (object id 11)
        assert_eq!(count, 7, "Header primitive count must match written Data records");
        let mut uid = Vec::new();
        cfb.open_stream("IND/UniqueIdPrimitiveInformation/Data")
            .unwrap()
            .read_to_end(&mut uid)
            .unwrap();
        let uid_text = String::from_utf8_lossy(&uid);
        assert!(uid_text.contains("Region"), "courtyard/copper pours must be Region records");
        assert!(cfb.exists("IND/UniqueIdPrimitiveInformation/Data"));
        assert!(path.metadata().unwrap().len() > 64);
    }

    #[test]
    fn pcblib_embeds_step_as_component_body() {
        let dir = std::env::temp_dir().join("lceda-test-pcb-step");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("BODY.PcbLib");
        let fp = FootprintIr {
            name: "BODY".into(),
            description: "with 3d".into(),
            meta: Default::default(),
            pads: vec![IrPad {
                designator: "1".into(),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                hole: 0.0,
                hole_slot: 0.0,
                hole_shape: "ROUND".into(),
                rotation: 0.0,
                layer: 1,
                shape: "RECT".into(),
                polygon: None,
            }],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
        };
        let step = b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
        write_pcblib_with_step(&path, &fp, Some(step)).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut hdr = Vec::new();
        cfb.open_stream("Library/Models/Header")
            .unwrap()
            .read_to_end(&mut hdr)
            .unwrap();
        assert_eq!(i32::from_le_bytes(hdr.try_into().unwrap()), 1);
        assert!(cfb.exists("Library/Models/0"));
        let mut uid = Vec::new();
        cfb.open_stream("BODY/UniqueIdPrimitiveInformation/Data")
            .unwrap()
            .read_to_end(&mut uid)
            .unwrap();
        assert!(String::from_utf8_lossy(&uid).contains("ComponentBody"));
        let mut count = Vec::new();
        cfb.open_stream("BODY/Header").unwrap().read_to_end(&mut count).unwrap();
        assert_eq!(i32::from_le_bytes(count.try_into().unwrap()), 2); // pad + body
    }

    #[test]
    fn pcblib_custom_poly_pad_adds_copper_and_mask_regions() {
        let dir = std::env::temp_dir().join("lceda-test-pcb-poly");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("POLY.PcbLib");
        let fp = FootprintIr {
            name: "POLY".into(),
            description: "custom".into(),
            meta: Default::default(),
            pads: vec![IrPad {
                designator: "1".into(),
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.2,
                hole: 0.0,
                hole_slot: 0.0,
                hole_shape: "ROUND".into(),
                rotation: 0.0,
                layer: 1,
                shape: "POLY".into(),
                polygon: Some(vec![(0.0, 0.0), (2.0, 0.0), (1.0, 1.2)]),
            }],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
        };
        write_pcblib(&path, &fp).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut hdr = Vec::new();
        cfb.open_stream("POLY/Header").unwrap().read_to_end(&mut hdr).unwrap();
        // hotspot pad + copper region + top solder mask
        assert_eq!(i32::from_le_bytes(hdr.try_into().unwrap()), 3);
        let mut uid = Vec::new();
        cfb.open_stream("POLY/UniqueIdPrimitiveInformation/Data")
            .unwrap()
            .read_to_end(&mut uid)
            .unwrap();
        let text = String::from_utf8_lossy(&uid);
        assert!(text.contains("Pad"));
        assert!(text.matches("Region").count() >= 2);
    }

    fn sample_symbol(name: &str) -> SymbolIr {
        SymbolIr {
            name: name.into(),
            description: "test".into(),
            meta: Default::default(),
            pins: vec![IrPin {
                number: "1".into(),
                name: "1".into(),
                x: 0.0,
                y: 5.08,
                length: 2.54,
                rotation: 270.0,
                pin_type: String::new(),
            }],
            rects: vec![IrRect {
                x1: -1.0,
                y1: -2.54,
                x2: 1.0,
                y2: 2.54,
            }],
            polys: vec![],
            ellipses: vec![],
        }
    }

    fn sample_fp(name: &str) -> FootprintIr {
        FootprintIr {
            name: name.into(),
            description: name.into(),
            meta: Default::default(),
            pads: vec![IrPad {
                designator: "1".into(),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                hole: 0.0,
                hole_slot: 0.0,
                hole_shape: "ROUND".into(),
                rotation: 0.0,
                layer: 1,
                shape: "RECT".into(),
                polygon: None,
            }],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
        }
    }

    #[test]
    fn writes_merged_schlib_and_pcblib() {
        let dir = std::env::temp_dir().join("lceda-test-merge");
        let _ = std::fs::create_dir_all(&dir);
        let a = sample_symbol("AAA");
        let b = sample_symbol("BBB");
        let sch = dir.join("lceda.SchLib");
        write_schlib_many(&sch, &[&a, &b]).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&sch).unwrap()).unwrap();
        assert!(cfb.exists("AAA/Data"));
        assert!(cfb.exists("BBB/Data"));
        let mut hdr = Vec::new();
        cfb.open_stream("FileHeader").unwrap().read_to_end(&mut hdr).unwrap();
        let text = String::from_utf8_lossy(&hdr);
        assert!(text.contains("COMPCOUNT=2"));
        assert!(text.contains("LIBREF0=AAA"));
        assert!(text.contains("LIBREF1=BBB"));

        let fa = sample_fp("FPA");
        let fb = sample_fp("FPB");
        let pcb = dir.join("lceda.PcbLib");
        write_pcblib_library(
            &pcb,
            &[
                pcblib::PcbLibPart {
                    fp: &fa,
                    step: None,
                },
                pcblib::PcbLibPart {
                    fp: &fb,
                    step: None,
                },
            ],
        )
        .unwrap();
        let cfb = cfb::CompoundFile::open(std::fs::File::open(&pcb).unwrap()).unwrap();
        assert!(cfb.exists("FPA/Data"));
        assert!(cfb.exists("FPB/Data"));
        let mut lib = Vec::new();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&pcb).unwrap()).unwrap();
        cfb.open_stream("Library/Data").unwrap().read_to_end(&mut lib).unwrap();
        assert!(String::from_utf8_lossy(&lib).contains("WEIGHT=2"));
    }
}

/// EasyEDA 图层 → Altium PcbLib 二进制 layer byte。
pub fn pcb_layer(easy: i32, hole_mm: f64) -> u8 {
    if easy == 12 || hole_mm > 1e-6 {
        return 74; // MultiLayer
    }
    match easy {
        1 => 1,   // Top
        2 => 32,  // Bottom
        3 | 49 => 33, // Top overlay
        4 => 34,  // Bottom overlay
        5 => 37,  // Top solder
        6 => 38,  // Bottom solder
        7 => 35,  // Top paste
        8 => 36,  // Bottom paste
        11 | 48 => 57, // Mechanical1
        13 => 58,
        50 => 61,
        51 => 62,
        _ => 33,
    }
}

pub fn pad_shape_byte(shape: &str, width: f64, height: f64) -> u8 {
    let s = shape.to_ascii_uppercase();
    if s.contains("POLY") || s.contains("RECT") {
        2
    } else if s.contains("OCT") {
        3
    } else if s.contains("OVAL") {
        9
    } else if (width - height).abs() < 1e-6 {
        1
    } else {
        9
    }
}

pub fn hole_type_byte(shape: &str) -> u8 {
    let s = shape.to_ascii_uppercase();
    if s.contains("SLOT") {
        2
    } else if s.contains("SQUARE") || s.contains("RECT") {
        1
    } else {
        0
    }
}
