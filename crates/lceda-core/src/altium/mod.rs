pub mod binary;
pub mod pcblib;
pub mod sch_color;
pub mod schlib;

pub use sch_color::{PinStyle, SchColorScheme, SchColors};

use crate::error::Result;
use crate::ir::{FootprintIr, SymbolIr};
use std::path::Path;

pub fn write_schlib(path: &Path, symbol: &SymbolIr) -> Result<()> {
    schlib::write(path, symbol)
}

pub fn write_schlib_with_colors(path: &Path, symbol: &SymbolIr, colors: SchColors) -> Result<()> {
    schlib::write_with_colors(path, symbol, colors)
}

pub fn write_schlib_many(path: &Path, symbols: &[&SymbolIr]) -> Result<()> {
    schlib::write_many(path, symbols)
}

pub fn write_schlib_many_with_colors(
    path: &Path,
    symbols: &[&SymbolIr],
    colors: SchColors,
) -> Result<()> {
    schlib::write_many_with_colors(path, symbols, colors)
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
        let hdr_text = String::from_utf8_lossy(&hdr);
        assert!(hdr_text.contains("COLOR2=0"), "default classic black pin text: {hdr_text}");
        let mut data = Vec::new();
        cfb.open_stream("RES/Data").unwrap().read_to_end(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("Color=128") || text.contains("COLOR=128"),
            "default SchLib must use Altium maroon 128, got {text}"
        );
        assert!(text.contains("16711680"), "designator/comment blue 16711680: {text}");
        assert!(text.contains("NAME=Comment"), "{text}");
        assert!(
            text.contains("RECORD=2") && text.contains("NAME_CUSTOMFONTID"),
            "classic pins are ASCII: {text}"
        );
        assert!(!cfb.exists("RES/PinTextData"));
    }

    #[test]
    fn writes_schlib_easyeda_colors() {
        let dir = std::env::temp_dir().join("lceda-test-sch-easyeda");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("RES.SchLib");
        let symbol = sample_symbol("RES");
        write_schlib_with_colors(&path, &symbol, SchColors::easyeda()).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut data = Vec::new();
        cfb.open_stream("RES/Data").unwrap().read_to_end(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("Color=136") || text.contains("COLOR=136"),
            "立创官方外壳 #880000=136, got {text}"
        );
        assert!(text.contains("RECORD=2"), "{text}");
        assert!(text.contains("NAME_CUSTOMFONTID=2"), "{text}");
        assert!(!cfb.exists("RES/PinTextData"));
        let mut hdr = Vec::new();
        cfb.open_stream("FileHeader").unwrap().read_to_end(&mut hdr).unwrap();
        let hdr = String::from_utf8_lossy(&hdr);
        assert!(hdr.contains("FONTIDCOUNT=4"), "{hdr}");
        assert!(hdr.contains("COLOR2=16711680"), "官方管脚字蓝: {hdr}");
        assert!(hdr.contains("COLOR3=255"), "电源红: {hdr}");
        assert!(hdr.contains("COLOR4=0"), "地黑: {hdr}");
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
            model: Default::default(),
        };
        write_pcblib(&path, &fp).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        assert!(cfb.exists("FileHeader"));
        assert!(cfb.exists("FileVersionInfo/Data"));
        assert!(cfb.exists("Library/Data"));
        assert!(cfb.exists("Library/PadViaLibrary/Data"));
        assert!(cfb.exists("Library/ComponentParamsTOC/Data"));
        assert!(cfb.exists("Library/ModelsNoEmbed/Header"));
        assert!(cfb.exists("R0402/Data"));
        assert!(cfb.exists("R0402/UniqueIdPrimitiveInformation/Data"));
        let mut hdr = Vec::new();
        cfb.open_stream("FileHeader").unwrap().read_to_end(&mut hdr).unwrap();
        assert_eq!(hdr.len(), 53, "AD FileHeader is version + 5.01 + UniqueId");
        let mut lib = Vec::new();
        cfb.open_stream("Library/Data").unwrap().read_to_end(&mut lib).unwrap();
        assert!(lib.len() > 10_000, "Library/Data must carry the board stack, got {}", lib.len());
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
            model: Default::default(),
        };
        write_pcblib(&path, &fp).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut hdr = Vec::new();
        cfb.open_stream("IND/Header").unwrap().read_to_end(&mut hdr).unwrap();
        assert_eq!(hdr.len(), 4);
        let count = i32::from_le_bytes(hdr.try_into().unwrap());
        // 2 pads + 4 courtyard track segments; overlay FILLs are not written as Region
        assert_eq!(count, 6, "Header primitive count must match written Data records");
        let mut uid = Vec::new();
        cfb.open_stream("IND/UniqueIdPrimitiveInformation/Data")
            .unwrap()
            .read_to_end(&mut uid)
            .unwrap();
        let uid_text = String::from_utf8_lossy(&uid);
        assert!(uid_text.contains("Pad"));
        assert!(uid_text.contains("Track"));
        assert!(uid_text.contains("UNIQUEID="));
        assert!(uid_text.contains("PRIMITIVEINDEX=0"));
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
            model: crate::ir::Model3dPlacement {
                rot_z: 90.0,
                ..Default::default()
            },
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
        let mut data = Vec::new();
        cfb.open_stream("BODY/Data").unwrap().read_to_end(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains("MODEL.3D.ROTZ=90.000"), "{text}");
        assert!(text.contains("MODEL.3D.ROTX=0.000"));
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
            model: Default::default(),
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
            model: Default::default(),
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
        let lib_text = String::from_utf8_lossy(&lib);
        assert!(lib_text.contains("KIND=Protel_Advanced_PCB_Library"));
        assert!(cfb.exists("FPA/Data"));
        assert!(cfb.exists("FPB/Data"));
    }

    #[test]
    fn writes_schlib_altium_classic_pin_text() {
        let dir = std::env::temp_dir().join("lceda-test-sch-classic");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("RES.SchLib");
        write_schlib_with_colors(&path, &sample_symbol("RES"), SchColors::altium_classic()).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut data = Vec::new();
        cfb.open_stream("RES/Data").unwrap().read_to_end(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("RECORD=2") && text.contains("COLOR=128"),
            "AD 经典按嘉立创官方导出写 ASCII 管脚: {text}"
        );
        assert!(text.contains("NAME_CUSTOMFONTID=2"), "{text}");
        assert!(text.contains("DESIGNATOR_CUSTOMFONTID=2"), "{text}");
        assert!(
            !cfb.exists("RES/PinTextData"),
            "官方导出不写 PinTextData"
        );
        let mut hdr = Vec::new();
        cfb.open_stream("FileHeader").unwrap().read_to_end(&mut hdr).unwrap();
        let hdr = String::from_utf8_lossy(&hdr);
        assert!(hdr.contains("FONTIDCOUNT=2"), "{hdr}");
        assert!(hdr.contains("COLOR2=0"), "管脚字黑体: {hdr}");
    }

    #[test]
    fn writes_schlib_easyeda_power_and_ground_pins() {
        let dir = std::env::temp_dir().join("lceda-test-sch-elec");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("U.SchLib");
        let mut symbol = sample_symbol("U");
        symbol.pins = vec![
            IrPin {
                number: "1".into(),
                name: "VCC".into(),
                x: 0.0,
                y: 5.08,
                length: 2.54,
                rotation: 180.0,
                pin_type: String::new(),
            },
            IrPin {
                number: "2".into(),
                name: "GND".into(),
                x: 0.0,
                y: 0.0,
                length: 2.54,
                rotation: 180.0,
                pin_type: String::new(),
            },
            IrPin {
                number: "3".into(),
                name: "TXD".into(),
                x: 5.08,
                y: 2.54,
                length: 2.54,
                rotation: 0.0,
                pin_type: "OUT".into(),
            },
        ];
        write_schlib_with_colors(&path, &symbol, SchColors::easyeda()).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut data = Vec::new();
        cfb.open_stream("U/Data").unwrap().read_to_end(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains("NAME=TXD") && text.contains("COLOR=136"), "{text}");
        assert!(data.windows(4).any(|w| w == 255i32.to_le_bytes()), "VCC red binary");
        assert!(data.windows(4).any(|w| w == 0i32.to_le_bytes()), "GND black binary");
    }

    #[test]
    fn writes_schlib_custom_pin_text_colors() {
        let dir = std::env::temp_dir().join("lceda-test-sch-pintext");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("RES.SchLib");
        let mut colors = SchColors::altium_classic();
        colors.pin_name = SchColors::rgb(0, 0, 0);
        colors.pin_number = SchColors::rgb(0, 128, 0);
        write_schlib_with_colors(&path, &sample_symbol("RES"), colors).unwrap();
        let mut cfb = cfb::CompoundFile::open(std::fs::File::open(&path).unwrap()).unwrap();
        let mut data = Vec::new();
        cfb.open_stream("RES/Data").unwrap().read_to_end(&mut data).unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains("NAME_CUSTOMFONTID=2"), "{text}");
        assert!(text.contains("DESIGNATOR_CUSTOMFONTID=3"), "{text}");
        assert!(!cfb.exists("RES/PinTextData"));
        let mut hdr = Vec::new();
        cfb.open_stream("FileHeader").unwrap().read_to_end(&mut hdr).unwrap();
        let hdr = String::from_utf8_lossy(&hdr);
        assert!(hdr.contains("COLOR2=0"), "{hdr}");
        assert!(hdr.contains("COLOR3=32768"), "{hdr}");
    }
}

/// EasyEDA Pro 图层 → Altium PcbLib 二进制 layer byte。
/// 48 外形跟 EasyEDALoader 的 ComponentShape 一样落到机械层；49 标记放到丝印，方便看见 Pin 1。
pub fn pcb_layer(easy: i32, hole_mm: f64) -> u8 {
    if easy == 12 || hole_mm > 1e-6 {
        return 74; // MultiLayer
    }
    match easy {
        1 => 1,       // Top
        2 => 32,      // Bottom
        3 | 49 => 33, // Top overlay / component marking
        4 => 34,      // Bottom overlay
        5 => 37,      // Top solder
        6 => 38,      // Bottom solder
        7 => 35,      // Top paste
        8 => 36,      // Bottom paste
        11 | 48 => 57, // Mechanical1 / component shape
        13 => 58,     // Mechanical2 / document courtyard
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
