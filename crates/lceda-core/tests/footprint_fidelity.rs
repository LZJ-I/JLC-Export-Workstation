//! Regression checks against the public AltiumSharp pad/region binary layout.
//! https://github.com/issus/AltiumSharp/blob/master/src/OriginalCircuit.Altium/Serialization/Writers/PcbLibWriter.cs
//! Actual AD editor acceptance is a separate, target-machine check.
use lceda_core::{altium, easyeda, ir, kicad};
use std::{fs, io::Read, path::{Path, PathBuf}};

fn output(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join("lceda-footprint-fidelity");
    fs::create_dir_all(&root).unwrap();
    root.join(name)
}

fn block<'a>(bytes: &'a [u8], offset: &mut usize) -> &'a [u8] {
    let size = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap()) as usize & 0x00ff_ffff;
    *offset += 4;
    let payload = &bytes[*offset..*offset + size];
    *offset += size;
    payload
}

fn records(path: &Path, name: &str) -> Vec<(u8, Vec<Vec<u8>>)> {
    let mut cfb = cfb::open(path).unwrap();
    let mut bytes = Vec::new();
    cfb.open_stream(format!("{name}/Data")).unwrap().read_to_end(&mut bytes).unwrap();
    let mut offset = 0;
    block(&bytes, &mut offset); // Footprint name, followed by typed primitives.
    let mut records = Vec::new();
    while offset < bytes.len() {
        let kind = bytes[offset]; offset += 1;
        assert!(matches!(kind, 1 | 2 | 4 | 11 | 12), "unexpected primitive {kind}");
        let count = if kind == 2 { 6 } else { 1 };
        let parts = (0..count).map(|_| block(&bytes, &mut offset).to_vec()).collect();
        records.push((kind, parts));
    }
    let mut count = Vec::new();
    cfb.open_stream(format!("{name}/Header")).unwrap().read_to_end(&mut count).unwrap();
    assert_eq!(u32::from_le_bytes(count.try_into().unwrap()) as usize, records.len());
    let mut unique = Vec::new();
    cfb.open_stream(format!("{name}/UniqueIDPrimitiveInformation/Header")).unwrap().read_to_end(&mut unique).unwrap();
    assert_eq!(u32::from_le_bytes(unique.try_into().unwrap()) as usize, records.len());
    records
}

#[test]
fn real_tssop_oval_pads_keep_semicircular_ends_on_every_layer() {
    let source = serde_json::from_str(include_str!("fixtures/DS90LV048ATMTC_NOPB_footprint_easyeda.json")).unwrap();
    let fp = ir::footprint_ir("TSSOP_OVAL", "C205463", easyeda::parse_footprint(&source).unwrap(), Default::default());
    let path = output("TSSOP_OVAL.PcbLib");
    altium::write_pcblib(&path, &fp).unwrap();
    kicad::write_footprint_mod(&output("TSSOP_OVAL.kicad_mod"), &fp, None).unwrap();
    let records = records(&path, &fp.name);
    let pads: Vec<_> = records.iter().filter(|(kind, _)| *kind == 2).collect();
    assert_eq!(pads.len(), 16);
    for ((_, parts), original) in pads.iter().zip(&fp.pads) {
        let main = &parts[4]; let stack = &parts[5];
        assert_eq!(main[49], 9, "native rounded-rectangle shape with full radius");
        assert_eq!(stack.len(), 596);
        assert_eq!(stack[531], 1, "per-layer shape overrides must be enabled");
        assert!(stack[532..564].iter().all(|&shape| shape == 9));
        assert!(stack[564..596].iter().all(|&radius| radius == 100), "25% produces a rounded rectangle, not an oblong");
        let width = i32::from_le_bytes(main[21..25].try_into().unwrap());
        let height = i32::from_le_bytes(main[25..29].try_into().unwrap());
        assert!((width as f64 * 0.0254 / 10000.0 - original.width).abs() < 0.00001);
        assert!((height as f64 * 0.0254 / 10000.0 - original.height).abs() < 0.00001);
        assert_eq!(f64::from_le_bytes(main[52..60].try_into().unwrap()), original.rotation);
    }
    assert_eq!(records.iter().filter(|(kind, _)| *kind == 11).count(), fp.regions.len(), "pin-1 filled dots must survive too");
}

fn marked_footprint() -> ir::FootprintIr {
    let source = serde_json::json!({"result":{"dataStr": concat!(
        "[\"PAD\",\"p\",0,\"\",1,\"1\",0,0,90,null,[\"RECT\",20,60,0],[],0,0,0,1]\n",
        "[\"FILL\",\"top\",0,\"\",3,0.2,0,[[0,0,\"L\",10,0,0,10,0,0]],0]\n",
        "[\"FILL\",\"bottom\",0,\"\",4,0.2,0,[[20,0,\"L\",30,0,20,10,20,0]],0]\n",
        "[\"FILL\",\"pin1\",0,\"\",49,0.2,0,[[40,0,\"L\",50,0,40,10,40,0]],0]\n",
        "[\"FILL\",\"body\",0,\"\",48,0.2,0,[[-50,-50,\"L\",50,-50,50,50,-50,50,-50,-50]],0]\n",
        "[\"FILL\",\"3dpin\",0,\"\",50,0.2,0,[[0,0,\"L\",20,0,20,60,0,60,0,0]],0]\n"
    )}});
    ir::footprint_ir("PIN1_ARROWS", "Silkscreen and marking fidelity", easyeda::parse_footprint(&source).unwrap(), Default::default())
}

#[test]
fn filled_pin_one_arrows_keep_their_points_layers_and_native_region_type() {
    let fp = marked_footprint();
    let path = output("PIN1_ARROWS.PcbLib");
    altium::write_pcblib(&path, &fp).unwrap();
    kicad::write_footprint_mod(&output("PIN1_ARROWS.kicad_mod"), &fp, None).unwrap();
    let records = records(&path, &fp.name);
    let regions: Vec<_> = records.iter().filter(|(kind, _)| *kind == 11).collect();
    assert_eq!(regions.len(), 3, "keep markings; exclude 3D construction and layer-48 body reference");
    for ((_, parts), original) in regions.iter().zip(fp.regions.iter().filter(|r| r.layer != 48)) {
        let raw = &parts[0];
        assert_eq!(raw[0], altium::pcb_layer(original.layer, 0.0));
        assert_eq!(&raw[13..18], &[0; 5], "no cutout contours or extended vertices");
        let mut offset = 18;
        let params = String::from_utf8_lossy(block(raw, &mut offset));
        assert!(params.contains("KIND=0"));
        assert!(params.contains("ISBOARDCUTOUT=FALSE"));
        assert!(params.contains("ISSHAPEBASED=FALSE"));
        let count = u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        assert_eq!(count, 4);
        for &(x, y) in &original.points {
            let read_x = f64::from_le_bytes(raw[offset..offset + 8].try_into().unwrap());
            let read_y = f64::from_le_bytes(raw[offset + 8..offset + 16].try_into().unwrap());
            assert!((read_x * 0.0254 / 10000.0 - x).abs() < 0.00001);
            assert!((read_y * 0.0254 / 10000.0 - y).abs() < 0.00001);
            offset += 16;
        }
        assert_eq!(offset, raw.len());
    }
    // Keep ordinary rectangular pads rectangular while fixing oval pads.
    let rectangular = &records.iter().find(|(kind, _)| *kind == 2).unwrap().1;
    assert_eq!(rectangular[4][49], 2);
    assert_eq!(rectangular[5][531], 0);
}

#[test]
fn ellipse_and_round_aliases_export_as_oblong_instead_of_rectangle() {
    let mut fp = marked_footprint();
    fp.regions.clear();
    fp.pads[0].width = 1.0; fp.pads[0].height = 2.0;
    for shape in ["OVAL", "ELLIPSE", "ROUND", "CIRCLE"] {
        fp.pads[0].shape = shape.into();
        let module = output(&format!("ALIAS_{shape}.kicad_mod"));
        kicad::write_footprint_mod(&module, &fp, None).unwrap();
        let text = fs::read_to_string(module).unwrap();
        assert!(text.contains("smd oval"), "{shape}: {text}");
        let path = output(&format!("ALIAS_{shape}.PcbLib"));
        altium::write_pcblib(&path, &fp).unwrap();
        let records = records(&path, &fp.name);
        let pad = &records.iter().find(|(kind, _)| *kind == 2).unwrap().1;
        assert!(pad[5][564..596].iter().all(|&radius| radius == 100));
    }
}
