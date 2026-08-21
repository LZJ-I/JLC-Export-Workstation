pub mod binary;
pub mod pcblib;
pub mod schlib;

use crate::error::Result;
use crate::ir::{FootprintIr, SymbolIr};
use std::path::Path;

pub fn write_schlib(path: &Path, symbol: &SymbolIr) -> Result<()> {
    schlib::write(path, symbol)
}

pub fn write_pcblib(path: &Path, footprint: &FootprintIr) -> Result<()> {
    pcblib::write(path, footprint)
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
