//! PcbLib 写出。图元布局对齐 [npnp](https://github.com/ref42/npnp) 已验证能被 AD 打开的 EasyEDA 导出，
//! 容器结构对齐 [AltiumSharp](https://github.com/issus/AltiumSharp) 的 from-scratch PcbLib。

use super::binary::{BinWriter, CfbDoc, encode_flags, from_mils, from_mm};
use super::{hole_type_byte, pad_shape_byte, pcb_layer};
use crate::error::{Error, Result};
use crate::ir::{FootprintIr, IrPad};
use crate::util::{looks_like_step, sanitize_filename, unique_altium_section_key};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

const DEFAULT_SOLDER_MASK_MIL: f64 = 1.969;
const DEFAULT_CORNER_RADIUS: u8 = 25;
const DEFAULT_BODY_HEIGHT_MM: f64 = 1.0;

pub struct PcbLibPart<'a> {
    pub fp: &'a FootprintIr,
    pub step: Option<&'a [u8]>,
}

pub fn write(path: &Path, fp: &FootprintIr) -> Result<()> {
    write_library(path, &[PcbLibPart { fp, step: None }])
}

pub fn write_with_step(path: &Path, fp: &FootprintIr, step: Option<&[u8]>) -> Result<()> {
    write_library(path, &[PcbLibPart { fp, step }])
}

pub fn write_library(path: &Path, parts: &[PcbLibPart<'_>]) -> Result<()> {
    if parts.is_empty() {
        return Err(Error::Altium("PcbLib 没有封装".into()));
    }

    let mut used = HashSet::new();
    let keys: Vec<String> = parts
        .iter()
        .map(|p| unique_altium_section_key(&p.fp.name, &mut used))
        .collect();

    let mut models = Vec::new();
    let mut bodies: Vec<Option<BodyInfo>> = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        match part.step {
            Some(bytes) if looks_like_step(bytes) => {
                let id = model_guid(&keys[index], index);
                let name = step_model_name(&part.fp.name);
                bodies.push(Some(BodyInfo {
                    id: id.clone(),
                    name: name.clone(),
                    outline: body_outline(part.fp),
                    height: from_mm(DEFAULT_BODY_HEIGHT_MM),
                }));
                models.push(EmbeddedModel {
                    id,
                    name,
                    data: bytes.to_vec(),
                });
            }
            _ => bodies.push(None),
        }
    }

    let mut cfb = CfbDoc::create(path)?;
    cfb.stream("FileHeader", &file_header())?;

    let key_pairs: Vec<(String, String)> = parts
        .iter()
        .zip(keys.iter())
        .filter(|(part, key)| part.fp.name != **key)
        .map(|(part, key)| (part.fp.name.clone(), key.clone()))
        .collect();
    if !key_pairs.is_empty() {
        cfb.stream("SectionKeys", &section_keys_bytes(&key_pairs))?;
    }

    cfb.storage("Library")?;
    cfb.stream("Library/Header", &i32_stream(1))?;
    cfb.stream("Library/Data", &library_data(&keys))?;

    cfb.storage("Library/Models")?;
    cfb.stream("Library/Models/Header", &i32_stream(models.len() as i32))?;
    cfb.stream("Library/Models/Data", &models_data(&models))?;
    for (index, model) in models.iter().enumerate() {
        cfb.stream(&format!("Library/Models/{index}"), &zlib_store(&model.data)?)?;
    }

    cfb.storage("Library/Textures")?;
    cfb.stream("Library/Textures/Header", &i32_stream(0))?;
    cfb.stream("Library/Textures/Data", &[])?;

    cfb.storage("Library/ModelsNoEmbed")?;
    cfb.stream("Library/ModelsNoEmbed/Header", &i32_stream(0))?;
    cfb.stream("Library/ModelsNoEmbed/Data", &[])?;

    for (i, part) in parts.iter().enumerate() {
        let key = &keys[i];
        let height = bodies[i].as_ref().map(|b| b.height).unwrap_or(0);
        let (data, names) = footprint_data(part.fp, key, bodies[i].as_ref());
        cfb.storage(key)?;
        cfb.stream(&format!("{key}/Header"), &i32_stream(names.len() as i32))?;
        cfb.stream(&format!("{key}/Parameters"), &footprint_params(part.fp, key, height))?;
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
    }

    cfb.finish()
}

struct EmbeddedModel {
    id: String,
    name: String,
    data: Vec<u8>,
}

struct BodyInfo {
    id: String,
    name: String,
    outline: Vec<(i32, i32)>,
    height: i32,
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

fn library_data(names: &[String]) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[
        ("HEADER", "PCB 6.0 Binary Library File".into()),
        ("WEIGHT", names.len().to_string()),
    ]);
    w.write_u32(names.len() as u32);
    for name in names {
        w.write_string_block(name);
    }
    w.into_vec()
}

fn section_keys_bytes(pairs: &[(String, String)]) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_i32(pairs.len() as i32);
    for (name, key) in pairs {
        w.write_block(0, |w| {
            w.write_pascal_short(name);
            w.write_u8(0);
        });
        w.write_string_block(key);
    }
    w.into_vec()
}

fn models_data(models: &[EmbeddedModel]) -> Vec<u8> {
    let mut w = BinWriter::new();
    for model in models {
        let text = format!(
            "EMBED=TRUE|MODELSOURCE=Undefined|ID={}|ROTX=0.000|ROTY=0.000|ROTZ=0.000|DZ=0|CHECKSUM=0|NAME={}",
            model.id, model.name
        );
        let mut inner = BinWriter::new();
        inner.write_cstring(&text);
        let data = inner.into_vec();
        w.write_i32(data.len() as i32);
        w.write_bytes(&data);
    }
    w.into_vec()
}

fn footprint_params(fp: &FootprintIr, name: &str, height_raw: i32) -> Vec<u8> {
    let mut w = BinWriter::new();
    w.write_params(&[
        ("PATTERN", name.into()),
        ("HEIGHT", format_raw_mil(height_raw)),
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

fn footprint_data(fp: &FootprintIr, name: &str, body: Option<&BodyInfo>) -> (Vec<u8>, Vec<&'static str>) {
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

    if let Some(body) = body {
        w.write_u8(12);
        write_component_body(&mut w, body);
        names.push("ComponentBody");
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

fn write_component_body(w: &mut BinWriter, body: &BodyInfo) {
    w.write_block(0, |w| {
        write_common(w, 57); // MECHANICAL1
        w.write_u32(0);
        w.write_u8(0);
        w.write_params(&[
            ("V7_LAYER", "MECHANICAL1".into()),
            ("NAME", "__LCEDA_BODY__".into()),
            ("KIND", "0".into()),
            ("SUBPOLYINDEX", "-1".into()),
            ("UNIONINDEX", "0".into()),
            ("ARCRESOLUTION", "0.5mil".into()),
            ("ISSHAPEBASED", "FALSE".into()),
            ("CAVITYHEIGHT", "0mil".into()),
            ("STANDOFFHEIGHT", "0mil".into()),
            ("OVERALLHEIGHT", format_raw_mil(body.height)),
            ("BODYPROJECTION", "0".into()),
            ("BODYCOLOR3D", "8421504".into()),
            ("BODYOPACITY3D", "1.000".into()),
            ("IDENTIFIER", "".into()),
            ("TEXTURE", "".into()),
            ("TEXTURECENTERX", "0mil".into()),
            ("TEXTURECENTERY", "0mil".into()),
            ("TEXTURESIZEX", "0mil".into()),
            ("TEXTURESIZEY", "0mil".into()),
            ("TEXTUREROTATION", "0.000".into()),
            ("MODELID", body.id.clone()),
            ("MODEL.CHECKSUM", "0".into()),
            ("MODEL.EMBED", "TRUE".into()),
            ("MODEL.NAME", body.name.clone()),
            ("MODEL.2D.X", "0mil".into()),
            ("MODEL.2D.Y", "0mil".into()),
            ("MODEL.2D.ROTATION", "0.000".into()),
            ("MODEL.3D.ROTX", "0.000".into()),
            ("MODEL.3D.ROTY", "0.000".into()),
            ("MODEL.3D.ROTZ", "0.000".into()),
            ("MODEL.3D.DZ", "0mil".into()),
            ("MODEL.MODELTYPE", "1".into()),
            ("MODEL.MODELSOURCE", "Undefined".into()),
        ]);
        w.write_u32(body.outline.len() as u32);
        for &(x, y) in &body.outline {
            w.write_f64(x as f64);
            w.write_f64(y as f64);
        }
    });
}

fn body_outline(fp: &FootprintIr) -> Vec<(i32, i32)> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    {
        let mut add = |x: f64, y: f64| {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        };
        for pad in &fp.pads {
            let hx = pad.width.abs() / 2.0;
            let hy = pad.height.abs() / 2.0;
            add(pad.x - hx, pad.y - hy);
            add(pad.x + hx, pad.y + hy);
        }
        for t in &fp.tracks {
            for &(x, y) in &t.points {
                add(x, y);
            }
        }
        for c in &fp.circles {
            add(c.x - c.radius, c.y - c.radius);
            add(c.x + c.radius, c.y + c.radius);
        }
        for r in &fp.regions {
            for &(x, y) in &r.points {
                add(x, y);
            }
        }
    }
    if !min_x.is_finite() {
        min_x = -1.0;
        max_x = 1.0;
        min_y = -1.0;
        max_y = 1.0;
    }
    let pad = 0.1;
    vec![
        (from_mm(min_x - pad), from_mm(min_y - pad)),
        (from_mm(max_x + pad), from_mm(min_y - pad)),
        (from_mm(max_x + pad), from_mm(max_y + pad)),
        (from_mm(min_x - pad), from_mm(max_y + pad)),
    ]
}

fn step_model_name(fp_name: &str) -> String {
    let mut name = sanitize_filename(fp_name);
    if !name.to_ascii_lowercase().ends_with(".step") && !name.to_ascii_lowercase().ends_with(".stp")
    {
        name.push_str(".step");
    }
    name
}

fn model_guid(key: &str, index: usize) -> String {
    let h = fnv1a(&format!("{key}|{index}"));
    format!(
        "{{{:08X}-{:04X}-4{:03X}-8{:03X}-{:012X}}}",
        (h >> 32) as u32,
        (h >> 16) as u16,
        (h as u16) & 0x0FFF,
        ((h >> 8) as u16) & 0x0FFF,
        h & 0x0000_FFFF_FFFF
    )
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn format_raw_mil(raw: i32) -> String {
    let mut text = format!("{:.4}", raw as f64 / 10_000.0);
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text = "0".into();
    }
    format!("{text}mil")
}

fn zlib_store(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data)?;
    Ok(enc.finish()?)
}
