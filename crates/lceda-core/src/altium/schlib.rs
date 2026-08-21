use super::binary::{BinWriter, CfbDoc, add_coord_param};
use crate::error::Result;
use crate::ir::SymbolIr;
use crate::util::{altium_section_key, unique_id};
use std::path::Path;

const BLUE_BGR: i32 = 0x00FF0000;
const RED_BGR: i32 = 0x000000FF;

pub fn write(path: &Path, symbol: &SymbolIr) -> Result<()> {
    let key = altium_section_key(&symbol.name);
    let mut cfb = CfbDoc::create(path)?;

    cfb.stream("FileHeader", &file_header(&key))?;
    cfb.stream("Storage", &empty_storage())?;
    cfb.storage(&key)?;
    cfb.stream(&format!("{key}/Data"), &component_data(symbol, &key))?;
    cfb.finish()
}

fn file_header(name: &str) -> Vec<u8> {
    let mut w = BinWriter::new();
    let uid = unique_id();
    let raw = format!(
        "|HEADER=Protel for Windows - Schematic Library Editor Binary File Version 5.0\
         |WEIGHT=1|MINORVERSION=2|UNIQUEID={uid}|FONTIDCOUNT=1|FONTNAME1=Times New Roman|SIZE1=10\
         |USEMBCS=T|ISBOC=T|SHEETSTYLE=9|SYSTEMFONT=1|BORDERON=T|DISPLAY_UNIT=0\
         |COMPCOUNT=1|LIBREF0={name}|PARTCOUNT0=2"
    );
    w.write_params_raw(&raw);
    w.write_i32(1);
    w.write_string_block(name);
    w.into_vec()
}

fn empty_storage() -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[("HEADER", "Icon storage".into())]);
    w.into_vec()
}

fn component_data(symbol: &SymbolIr, libref: &str) -> Vec<u8> {
    let mut w = BinWriter::new();
    let uid = unique_id();
    let mut header = vec![
        ("RECORD", "1".into()),
        ("LIBREFERENCE", libref.into()),
        ("COMPONENTDESCRIPTION", symbol.description.clone()),
        ("PARTCOUNT", "2".into()),
        ("DISPLAYMODECOUNT", "1".into()),
        ("INDEXINSHEET", "-1".into()),
        ("OWNERPARTID", "-1".into()),
        ("CURRENTPARTID", "1".into()),
        ("LIBRARYPATH", "*".into()),
        ("SOURCELIBRARYNAME", "*".into()),
        ("SHEETPARTFILENAME", "*".into()),
        ("TARGETFILENAME", "*".into()),
        ("UNIQUEID", uid),
        ("AREACOLOR", "11599871".into()),
        ("COLOR", BLUE_BGR.to_string()),
        ("PARTIDLOCKED", "T".into()),
        ("DESIGNITEMID", libref.into()),
    ];
    if !symbol.pins.is_empty() {
        header.push(("ALLPINCOUNT", symbol.pins.len().to_string()));
    }
    w.write_params(&header);

    for pin in &symbol.pins {
        write_pin(&mut w, pin);
    }

    for r in &symbol.rects {
        let mut pairs = vec![
            ("RECORD".into(), "14".into()),
            ("OwnerPartId".into(), "1".into()),
            ("LineWidth".into(), "1".into()),
            ("Color".into(), BLUE_BGR.to_string()),
            ("IsSolid".into(), "F".into()),
            ("Transparent".into(), "T".into()),
        ];
        add_coord_param(&mut pairs, "Location.X", r.x1);
        add_coord_param(&mut pairs, "Location.Y", r.y1);
        add_coord_param(&mut pairs, "Corner.X", r.x2);
        add_coord_param(&mut pairs, "Corner.Y", r.y2);
        write_named(&mut w, &pairs);
    }

    for poly in &symbol.polys {
        if poly.len() < 2 {
            continue;
        }
        let mut pairs = vec![
            ("RECORD".into(), "6".into()),
            ("OWNERPARTID".into(), "1".into()),
            ("LINEWIDTH".into(), "1".into()),
            ("COLOR".into(), BLUE_BGR.to_string()),
            ("LOCATIONCOUNT".into(), poly.len().to_string()),
        ];
        for (i, (x, y)) in poly.iter().enumerate() {
            add_coord_param(&mut pairs, &format!("X{}", i + 1), *x);
            add_coord_param(&mut pairs, &format!("Y{}", i + 1), *y);
        }
        write_named(&mut w, &pairs);
    }

    for e in &symbol.ellipses {
        let mut pairs = vec![
            ("RECORD".into(), "8".into()),
            ("OwnerPartId".into(), "1".into()),
            ("LineWidth".into(), "1".into()),
            ("Color".into(), BLUE_BGR.to_string()),
            ("AreaColor".into(), "0".into()),
            ("IsSolid".into(), "F".into()),
            ("Transparent".into(), "T".into()),
        ];
        add_coord_param(&mut pairs, "Location.X", e.x);
        add_coord_param(&mut pairs, "Location.Y", e.y);
        add_coord_param(&mut pairs, "Radius", e.rx);
        add_coord_param(&mut pairs, "SecondaryRadius", e.ry);
        write_named(&mut w, &pairs);
    }

    w.write_params(&[
        ("RECORD", "34".into()),
        ("OWNERPARTID", "-1".into()),
        ("COLOR", "8388608".into()),
        ("FONTID", "1".into()),
        ("TEXT", "U?".into()),
        ("NAME", "Designator".into()),
        ("READONLYSTATE", "1".into()),
    ]);

    write_footprint_implementation(&mut w, symbol);
    w.into_vec()
}

fn write_footprint_implementation(w: &mut BinWriter, symbol: &SymbolIr) {
    w.write_params(&[("RECORD", "44".into())]);
    let model = symbol.meta.footprint_lib.trim();
    if model.is_empty() {
        return;
    }
    w.write_params(&[
        ("RECORD", "45".into()),
        ("DESCRIPTION", "PCB footprint".into()),
        ("MODELNAME", model.into()),
        ("MODELTYPE", "PCBLIB".into()),
        ("DATAFILECOUNT", "1".into()),
        ("MODELDATAFILEKIND1", "PCBLib".into()),
        ("ISCURRENT", "T".into()),
        ("UNIQUEID", unique_id()),
    ]);
    w.write_params(&[("RECORD", "46".into())]);
    let mut seen = std::collections::HashSet::new();
    for pin in &symbol.pins {
        let d = pin.number.trim();
        if d.is_empty() || !seen.insert(d.to_ascii_lowercase()) {
            continue;
        }
        w.write_params(&[
            ("RECORD", "47".into()),
            ("DESINTF", d.into()),
            ("DESIMPCOUNT", "1".into()),
            ("DESIMP0", d.into()),
            ("ISTRIVIAL", "T".into()),
            ("UNIQUEID", unique_id()),
        ]);
    }
    w.write_params(&[("RECORD", "48".into())]);
}

fn write_named(w: &mut BinWriter, pairs: &[(String, String)]) {
    let refs: Vec<(&str, String)> = pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    w.write_params(&refs);
}

fn write_pin(w: &mut BinWriter, pin: &crate::ir::IrPin) {
    let orient = pin_orient(pin.rotation);
    let loc_x = dxp_num(pin.x);
    let loc_y = dxp_num(pin.y);
    let len = dxp_num(pin.length.max(2.54));
    let mut conglomerate = orient;
    conglomerate |= 0x08 | 0x10; // show name + designator
    w.write_block(0x01, |w| {
        w.write_i32(2);
        w.write_u8(0);
        w.write_i16(1); // OwnerPartId
        w.write_u8(0);
        w.write_u8(0);
        w.write_u8(0);
        w.write_u8(0);
        w.write_u8(0);
        w.write_pascal_short("");
        w.write_u8(0); // FormalType
        w.write_u8(4); // Passive
        w.write_u8(conglomerate);
        w.write_i16(len as i16);
        w.write_i16(loc_x as i16);
        w.write_i16(loc_y as i16);
        w.write_i32(RED_BGR);
        w.write_pascal_short(&pin.name);
        w.write_pascal_short(&pin.number);
        w.write_pascal_short("");
        w.write_pascal_short("");
        w.write_pascal_short("");
    });
}

fn pin_orient(rotation_deg: f64) -> u8 {
    let a = crate::easyeda::normalize_angle(rotation_deg);
    ((a / 90.0).round() as i32).rem_euclid(4) as u8
}

fn dxp_num(mm: f64) -> i32 {
    // 1 DXP = 10 mil = 0.254 mm
    (mm / 0.254).round() as i32
}

// expose normalize_angle - I'll add pub(crate) in easyeda instead of this
