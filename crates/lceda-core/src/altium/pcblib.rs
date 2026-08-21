//! PcbLib 写出。图元布局对齐 [npnp](https://github.com/ref42/npnp) 已验证能被 AD 打开的 EasyEDA 导出，
//! 容器结构对齐 [AltiumSharp](https://github.com/issus/AltiumSharp) 的 from-scratch PcbLib。

use super::binary::{BinWriter, CfbDoc, encode_flags, from_mils, from_mm};
use super::{hole_type_byte, pad_shape_byte, pcb_layer};
use crate::error::Result;
use crate::ir::{FootprintIr, IrPad};
use crate::util::altium_section_key;
use std::path::Path;

const DEFAULT_SOLDER_MASK_MIL: f64 = 1.969;
const DEFAULT_CORNER_RADIUS: u8 = 25;

pub fn write(path: &Path, fp: &FootprintIr) -> Result<()> {
    let key = altium_section_key(&fp.name);
    let mut cfb = CfbDoc::create(path)?;

    cfb.stream("FileHeader", &file_header())?;

    cfb.storage("Library")?;
    cfb.stream("Library/Header", &i32_stream(1))?;
    cfb.stream("Library/Data", &library_data(&key))?;

    cfb.storage("Library/Models")?;
    cfb.stream("Library/Models/Header", &i32_stream(0))?;
    cfb.stream("Library/Models/Data", &[])?;

    cfb.storage("Library/Textures")?;
    cfb.stream("Library/Textures/Header", &i32_stream(0))?;
    cfb.stream("Library/Textures/Data", &[])?;

    cfb.storage("Library/ModelsNoEmbed")?;
    cfb.stream("Library/ModelsNoEmbed/Header", &i32_stream(0))?;
    cfb.stream("Library/ModelsNoEmbed/Data", &[])?;

    cfb.storage(&key)?;
    let (data, names) = footprint_data(fp, &key);
    cfb.stream(&format!("{key}/Header"), &i32_stream(names.len() as i32))?;
    cfb.stream(&format!("{key}/Parameters"), &footprint_params(fp, &key))?;
    cfb.stream(&format!("{key}/WideStrings"), &empty_params())?;
    cfb.stream(&format!("{key}/Data"), &data)?;

    cfb.storage(&format!("{key}/UniqueIdPrimitiveInformation"))?;
    cfb.stream(
        &format!("{key}/UniqueIdPrimitiveInformation/Header"),
        &i32_stream(names.len() as i32),
    )?;
    cfb.stream(
        &format!("{key}/UniqueIdPrimitiveInformation/Data"),
        &unique_id_primitive_information(&names),
    )?;

    cfb.finish()
}

fn i32_stream(v: i32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn file_header() -> Vec<u8> {
    // npnp / 实测 AD 能打开的 PcbLib：FileHeader 只有版本 Pascal 串。
    let mut w = BinWriter::new();
    let version = "PCB 6.0 Binary Library File";
    w.write_i32(version.len() as i32);
    w.write_pascal_short(version);
    w.into_vec()
}

fn library_data(name: &str) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[
        ("HEADER", "PCB 6.0 Binary Library File".into()),
        ("WEIGHT", "1".into()),
    ]);
    w.write_u32(1);
    w.write_string_block(name);
    w.into_vec()
}

fn footprint_params(fp: &FootprintIr, name: &str) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[
        ("PATTERN", name.into()),
        ("HEIGHT", "0mil".into()),
        ("DESCRIPTION", altium_param(&fp.description)),
        ("ITEMGUID", "".into()),
        ("REVISIONGUID", "".into()),
    ]);
    w.into_vec()
}

fn empty_params() -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[]);
    w.into_vec()
}

fn unique_id_primitive_information(names: &[&str]) -> Vec<u8> {
    let mut w = BinWriter::new();
    for (index, object_name) in names.iter().enumerate() {
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if index > 0 {
            pairs.push(("PRIMITIVEINDEX", index.to_string()));
        }
        pairs.push(("PRIMITIVEOBJECTID", (*object_name).to_string()));
        w.write_params(&pairs);
    }
    w.into_vec()
}

fn footprint_data(fp: &FootprintIr, name: &str) -> (Vec<u8>, Vec<&'static str>) {
    let mut w = BinWriter::new();
    w.write_string_block(name);
    let mut names = Vec::new();

    for pad in &fp.pads {
        w.write_u8(2);
        write_pad(&mut w, pad);
        names.push("Pad");
    }

    for track in &fp.tracks {
        let layer = pcb_layer(track.layer, 0.0);
        let width = from_mm(track.width.max(0.05));
        for win in track.points.windows(2) {
            if nearly_same(win[0], win[1]) {
                continue;
            }
            w.write_u8(4);
            write_track(&mut w, layer, win[0], win[1], width);
            names.push("Track");
        }
    }

    for c in &fp.circles {
        let layer = pcb_layer(c.layer, 0.0);
        w.write_u8(1);
        write_arc(
            &mut w,
            layer,
            from_mm(c.x),
            from_mm(c.y),
            from_mm(c.radius.abs()),
            0.0,
            360.0,
            from_mm(c.width.max(0.05)),
        );
        names.push("Arc");
    }

    for a in &fp.arcs {
        let layer = pcb_layer(a.layer, 0.0);
        w.write_u8(1);
        write_arc(
            &mut w,
            layer,
            from_mm(a.x),
            from_mm(a.y),
            from_mm(a.radius.abs()),
            a.start,
            a.end,
            from_mm(a.width.max(0.05)),
        );
        names.push("Arc");
    }

    for r in &fp.regions {
        if r.points.len() < 3 {
            continue;
        }
        let layer = pcb_layer(r.layer, 0.0);
        w.write_u8(11);
        write_region(&mut w, layer, &closed_outline(&r.points));
        names.push("Region");
    }

    (w.into_vec(), names)
}

fn closed_outline(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut pts = points.to_vec();
    if let (Some(&first), Some(&last)) = (pts.first(), pts.last()) {
        if !nearly_same(first, last) {
            pts.push(first);
        }
    }
    pts
}

fn nearly_same(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

fn altium_param(s: &str) -> String {
    s.replace(['|', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

fn write_common(w: &mut BinWriter, layer: u8) {
    w.write_u8(layer);
    w.write_u16(encode_flags());
    w.write_bytes(&[0xFF; 10]);
}

fn write_track(w: &mut BinWriter, layer: u8, start: (f64, f64), end: (f64, f64), width: i32) {
    w.write_block(0, |w| {
        write_common(w, layer);
        w.write_coord_point(from_mm(start.0), from_mm(start.1));
        w.write_coord_point(from_mm(end.0), from_mm(end.1));
        w.write_coord(width);
        w.write_u16(0);
        w.write_u8(0);
    });
}

fn write_arc(
    w: &mut BinWriter,
    layer: u8,
    x: i32,
    y: i32,
    radius: i32,
    start: f64,
    end: f64,
    width: i32,
) {
    w.write_block(0, |w| {
        write_common(w, layer);
        w.write_coord_point(x, y);
        w.write_coord(radius);
        w.write_f64(start);
        w.write_f64(end);
        w.write_coord(width);
    });
}

fn write_region(w: &mut BinWriter, layer: u8, points: &[(f64, f64)]) {
    w.write_block(0, |w| {
        write_common(w, layer);
        w.write_u32(0);
        w.write_u8(0);
        w.write_params(&[("V7_LAYER", v7_layer_name(layer).into())]);
        w.write_u32(points.len() as u32);
        for &(x, y) in points {
            w.write_f64(from_mm(x) as f64);
            w.write_f64(from_mm(y) as f64);
        }
    });
}

fn v7_layer_name(layer: u8) -> &'static str {
    match layer {
        1 => "TOP",
        32 => "BOTTOM",
        33 => "TOPOVERLAY",
        34 => "BOTTOMOVERLAY",
        35 => "TOPPASTE",
        36 => "BOTTOMPASTE",
        37 => "TOPSOLDER",
        38 => "BOTTOMSOLDER",
        56 => "KEEPOUT",
        57 => "MECHANICAL1",
        58 => "MECHANICAL2",
        61 => "MECHANICAL5",
        62 => "MECHANICAL6",
        74 => "MULTILAYER",
        _ => "TOPOVERLAY",
    }
}

fn write_pad(w: &mut BinWriter, pad: &IrPad) {
    let layer = pcb_layer(pad.layer, pad.hole);
    let shape = pad_shape_byte(&pad.shape, pad.width, pad.height);
    let hole_type = hole_type_byte(&pad.hole_shape);
    let size_x = from_mm(pad.width);
    let size_y = from_mm(pad.height);
    let hole = from_mm(pad.hole);
    let loc_x = from_mm(pad.x);
    let loc_y = from_mm(pad.y);
    let rotation = crate::easyeda::normalize_angle(pad.rotation);
    let solder = from_mils(DEFAULT_SOLDER_MASK_MIL);

    w.write_string_block(&pad.designator);
    w.write_block_raw(0, &[0]);
    w.write_string_block("|&|0");
    w.write_block_raw(0, &[0]);

    w.write_block(0, |w| {
        write_common(w, layer);
        w.write_coord_point(loc_x, loc_y);
        w.write_coord_point(size_x, size_y);
        w.write_coord_point(size_x, size_y);
        w.write_coord_point(size_x, size_y);
        w.write_coord(hole);
        w.write_u8(shape);
        w.write_u8(shape);
        w.write_u8(shape);
        w.write_f64(rotation);
        w.write_bool(true);
        w.write_u8(0);
        w.write_u8(0); // mode simple
        w.write_u8(0); // plane connect
        w.write_coord(0); // relief air gap
        w.write_coord(from_mils(10.0));
        w.write_i16(4);
        w.write_coord(from_mils(10.0));
        w.write_coord(from_mils(20.0));
        w.write_i32(0);
        w.write_coord(0); // paste expansion
        w.write_coord(solder);
        w.write_bytes(&[0; 7]);
        w.write_u8(0); // paste mode
        w.write_u8(2); // solder mode = manual (non-zero expansion)
        w.write_u8(0); // drill type
        w.write_i16(0);
        w.write_i32(0);
        w.write_i16(0); // jumper
        w.write_i16(0);
    });

    w.write_block(0, |w| {
        for _ in 0..29 {
            w.write_i32(size_x);
        }
        for _ in 0..29 {
            w.write_i32(size_y);
        }
        for _ in 0..29 {
            w.write_u8(shape);
        }
        w.write_u8(0);
        w.write_u8(hole_type);
        w.write_i32(from_mm(pad.hole_slot));
        w.write_f64(if hole_type == 2 { rotation } else { 0.0 });
        for _ in 0..32 {
            w.write_i32(0);
        }
        for _ in 0..32 {
            w.write_i32(0);
        }
        let rounded = shape == 9;
        w.write_bool(rounded);
        w.write_u8(shape);
        for _ in 0..30 {
            w.write_u8(shape);
        }
        w.write_u8(shape);
        for _ in 0..32 {
            w.write_u8(if rounded { DEFAULT_CORNER_RADIUS } else { 50 });
        }
    });
}
