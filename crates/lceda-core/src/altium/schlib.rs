use super::binary::{BinWriter, CfbDoc, add_coord_param};
use super::sch_color::SchColors;
use crate::error::{Error, Result};
use crate::ir::SymbolIr;
use crate::util::{unique_altium_section_key, unique_id};
use std::collections::HashSet;
use std::path::Path;

pub fn write(path: &Path, symbol: &SymbolIr) -> Result<()> {
    write_with_colors(path, symbol, SchColors::default())
}

pub fn write_with_colors(path: &Path, symbol: &SymbolIr, colors: SchColors) -> Result<()> {
    write_many_with_colors(path, &[symbol], colors)
}

pub fn write_many(path: &Path, symbols: &[&SymbolIr]) -> Result<()> {
    write_many_with_colors(path, symbols, SchColors::default())
}

pub fn write_many_with_colors(path: &Path, symbols: &[&SymbolIr], colors: SchColors) -> Result<()> {
    if symbols.is_empty() {
        return Err(Error::Altium("SchLib 没有元件".into()));
    }
    let mut used = HashSet::new();
    let keys: Vec<String> = symbols
        .iter()
        .map(|s| unique_altium_section_key(&s.name, &mut used))
        .collect();
    let mut cfb = CfbDoc::create(path)?;
    cfb.stream("FileHeader", &file_header_many(&keys, colors))?;
    cfb.stream("Storage", &empty_storage())?;

    let key_pairs: Vec<(String, String)> = symbols
        .iter()
        .zip(keys.iter())
        .filter(|(sym, key)| sym.name != **key)
        .map(|(sym, key)| (sym.name.clone(), key.clone()))
        .collect();
    if !key_pairs.is_empty() {
        cfb.stream("SectionKeys", &section_keys_bytes(&key_pairs))?;
    }

    for (sym, key) in symbols.iter().zip(keys.iter()) {
        cfb.storage(key)?;
        cfb.stream(&format!("{key}/Data"), &component_data(sym, key, colors))?;
        if let Some(text) = pin_text_data_stream(sym, colors) {
            cfb.stream(&format!("{key}/PinTextData"), &text)?;
        }
    }
    cfb.finish()
}

fn file_header_many(names: &[String], colors: SchColors) -> Vec<u8> {
    let mut w = BinWriter::new();
    let uid = unique_id();
    let n = names.len();
    let extra = colors.pin_text_fonts();
    let font_count = 1 + extra.len();
    let pin_face = colors.pin_font_name();
    let pin_size = colors.clamped_pin_font_size();
    let mut raw = format!(
        "|HEADER=Protel for Windows - Schematic Library Editor Binary File Version 5.0\
         |WEIGHT={n}|MINORVERSION=2|UNIQUEID={uid}|FONTIDCOUNT={font_count}\
         |FONTNAME1=Times New Roman|SIZE1=10"
    );
    for (i, color) in extra.iter().enumerate() {
        let id = i + 2;
        raw.push_str(&format!(
            "|FONTNAME{id}={pin_face}|SIZE{id}={pin_size}|BOLD{id}=F|ITALIC{id}=F|COLOR{id}={color}"
        ));
    }
    raw.push_str(&format!(
        "|USEMBCS=T|ISBOC=T|SHEETSTYLE=9|SYSTEMFONT=1|BORDERON=T|DISPLAY_UNIT=0\
         |COMPCOUNT={n}"
    ));
    for (i, name) in names.iter().enumerate() {
        raw.push_str(&format!("|LIBREF{i}={name}|PARTCOUNT{i}=2"));
    }
    w.write_params_raw(&raw);
    w.write_i32(n as i32);
    for name in names {
        w.write_string_block(name);
    }
    w.into_vec()
}

fn section_keys_bytes(pairs: &[(String, String)]) -> Vec<u8> {
    let mut w = BinWriter::new();
    let mut pairs_out: Vec<(&str, String)> = vec![("KeyCount", pairs.len().to_string())];
    let owned: Vec<(String, String)> = pairs
        .iter()
        .enumerate()
        .flat_map(|(i, (name, key))| {
            [
                (format!("LibRef{i}"), name.clone()),
                (format!("SectionKey{i}"), key.clone()),
            ]
        })
        .collect();
    for (k, v) in &owned {
        pairs_out.push((k.as_str(), v.clone()));
    }
    w.write_params(&pairs_out);
    w.into_vec()
}

fn empty_storage() -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[("HEADER", "Icon storage".into())]);
    w.into_vec()
}

fn component_data(symbol: &SymbolIr, libref: &str, colors: SchColors) -> Vec<u8> {
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
        ("COLOR", colors.body.to_string()),
        ("PARTIDLOCKED", "T".into()),
        ("DESIGNITEMID", libref.into()),
    ];
    if !symbol.pins.is_empty() {
        header.push(("ALLPINCOUNT", symbol.pins.len().to_string()));
    }
    w.write_params(&header);

    for pin in &symbol.pins {
        write_pin(&mut w, pin, colors);
    }

    for r in &symbol.rects {
        let mut pairs = vec![
            ("RECORD".into(), "14".into()),
            ("OwnerPartId".into(), "1".into()),
            ("LineWidth".into(), "1".into()),
            ("Color".into(), colors.body.to_string()),
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
            ("COLOR".into(), colors.body.to_string()),
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
            ("Color".into(), colors.body.to_string()),
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
        ("COLOR", colors.designator.to_string()),
        ("FONTID", "1".into()),
        ("TEXT", symbol.designator.clone()),
        ("NAME", "Designator".into()),
        ("READONLYSTATE", "1".into()),
    ]);
    w.write_params(&[
        ("RECORD", "34".into()),
        ("OWNERPARTID", "-1".into()),
        ("COLOR", colors.comment.to_string()),
        ("FONTID", "1".into()),
        ("TEXT", symbol.name.clone()),
        ("NAME", "Comment".into()),
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

fn write_pin(w: &mut BinWriter, pin: &crate::ir::IrPin, colors: SchColors) {
    let style = colors.pin_style(&pin.pin_type, &pin.name);
    // 立创官方：线/字分色靠 PinTextData（二进制 COLOR 只管脚线）。
    // Altium 经典 / 自定义：ASCII RECORD=2 + FONT 表（嘉立创官方 SchDoc 路径）。
    if colors.follows_easyeda_pins() {
        write_pin_binary(w, pin, style.line);
    } else if style.uses_local_font() {
        write_pin_ascii(w, pin, colors, style);
    } else {
        write_pin_binary(w, pin, style.line);
    }
}

/// AD 真实 SchLib：`|HEADER=PinTextData|Weight=N` + 按管脚序号索引的 zlib 条目。
/// BOTH 格式 14 字节：`0x10 + font_id(i16) + COLORREF(u32)` × 名/号。
/// 只给立创配色写；经典 / 黑白 / 自定义走 FONT 表或管脚 COLOR。
fn pin_text_data_stream(symbol: &SymbolIr, colors: SchColors) -> Option<Vec<u8>> {
    if !colors.follows_easyeda_pins() || symbol.pins.is_empty() {
        return None;
    }
    let entries: Vec<_> = symbol
        .pins
        .iter()
        .enumerate()
        .map(|(i, pin)| {
            let style = colors.pin_style(&pin.pin_type, &pin.name);
            (
                i,
                colors.font_id_for(style.name) as i16,
                style.name,
                colors.font_id_for(style.number) as i16,
                style.number,
            )
        })
        .collect();
    Some(encode_pin_text_data(&entries))
}

fn encode_pin_text_data(entries: &[(usize, i16, i32, i16, i32)]) -> Vec<u8> {
    let mut w = BinWriter::new();
    let header = format!("|HEADER=PinTextData|Weight={}", entries.len());
    let header_bytes = header.as_bytes();
    w.write_i32((header_bytes.len() + 1) as i32);
    w.write_bytes(header_bytes);
    w.write_u8(0);
    for (index, name_font, name_color, des_font, des_color) in entries {
        w.write_compressed_named(
            &index.to_string(),
            &pin_text_both_attrs(*name_font, *name_color, *des_font, *des_color),
        );
    }
    w.into_vec()
}

fn pin_text_both_attrs(name_font: i16, name_color: i32, des_font: i16, des_color: i32) -> Vec<u8> {
    let mut b = Vec::with_capacity(14);
    b.push(0x10);
    b.extend_from_slice(&name_font.to_le_bytes());
    b.extend_from_slice(&(name_color as u32).to_le_bytes());
    b.push(0x10);
    b.extend_from_slice(&des_font.to_le_bytes());
    b.extend_from_slice(&(des_color as u32).to_le_bytes());
    b
}

/// 线/名/号同色时走二进制管脚（立创电源/地，或黑白）。
fn write_pin_binary(w: &mut BinWriter, pin: &crate::ir::IrPin, color: i32) {
    let orient = pin_orient(pin.rotation);
    let loc_x = dxp_num(pin.x);
    let loc_y = dxp_num(pin.y);
    let len = dxp_num(pin.length.max(2.54));
    let conglomerate = pin_conglomerate(orient, pin.show_name, pin.show_number, 0);
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
        w.write_i32(color);
        w.write_pascal_short(&pin.name);
        w.write_pascal_short(&pin.number);
        w.write_pascal_short("");
        w.write_pascal_short("");
        w.write_pascal_short("");
    });
}

/// 嘉立创官方预览 / AD 导出：ASCII RECORD=2。
/// COLOR 只管脚线；名/号走 FONT 表（NAME_CUSTOMFONTID）。
fn write_pin_ascii(
    w: &mut BinWriter,
    pin: &crate::ir::IrPin,
    colors: SchColors,
    style: super::sch_color::PinStyle,
) {
    let orient = pin_orient(pin.rotation);
    let loc_x = dxp_num(pin.x);
    let loc_y = dxp_num(pin.y);
    let len = dxp_num(pin.length.max(2.54));
    // bit5 与官方导出一致；bit3/4 按立创 valueVisible 显示名和号。
    let conglomerate = pin_conglomerate(orient, pin.show_name, pin.show_number, 0x20);
    let name_font = colors.font_id_for(style.name);
    let number_font = colors.font_id_for(style.number);
    w.write_params(&[
        ("RECORD", "2".into()),
        ("OWNERPARTID", "1".into()),
        ("FORMALTYPE", "1".into()),
        ("ELECTRICAL", "4".into()),
        ("PINCONGLOMERATE", conglomerate.to_string()),
        ("PINLENGTH", len.to_string()),
        ("LOCATION.X", loc_x.to_string()),
        ("LOCATION.Y", loc_y.to_string()),
        ("COLOR", style.line.to_string()),
        ("NAME", pin.name.replace('|', "_")),
        ("DESIGNATOR", pin.number.replace('|', "_")),
        ("NAME_CUSTOMFONTID", name_font.to_string()),
        ("DESIGNATOR_CUSTOMFONTID", number_font.to_string()),
        ("PINNAME_POSITIONCONGLOMERATE", "16".into()),
        ("PINDESIGNATOR_POSITIONCONGLOMERATE", "16".into()),
        ("SHOWPINNAME", altium_bool(pin.show_name)),
        ("SHOWDESIGNATOR", altium_bool(pin.show_number)),
    ]);
}

fn pin_conglomerate(orient: u8, show_name: bool, show_number: bool, extra: u8) -> u8 {
    let mut c = orient | extra;
    if show_name {
        c |= 0x08;
    }
    if show_number {
        c |= 0x10;
    }
    c
}

fn altium_bool(v: bool) -> String {
    if v { "T" } else { "F" }.into()
}

fn pin_orient(rotation_deg: f64) -> u8 {
    let a = crate::easyeda::normalize_angle(rotation_deg);
    ((a / 90.0).round() as i32).rem_euclid(4) as u8
}

fn dxp_num(mm: f64) -> i32 {
    // 1 DXP = 10 mil = 0.254 mm
    (mm / 0.254).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_text_both_matches_ad_library() {
        // TPS7A2012PDBVR.SchLib：font 2、黑字
        assert_eq!(pin_conglomerate(0, true, true, 0), 0x18);
        assert_eq!(pin_conglomerate(2, false, true, 0x20), 0x20 | 0x10 | 2);
        assert_eq!(
            pin_text_both_attrs(2, 0, 2, 0),
            [0x10, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        let blue = 16_711_680i32;
        let raw = pin_text_both_attrs(2, blue, 2, blue);
        assert_eq!(&raw[3..7], &blue.to_le_bytes());
        assert_eq!(&raw[10..14], &blue.to_le_bytes());
    }
}
