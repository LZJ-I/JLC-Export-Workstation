//! EasyEDA Pro `dataStr` 行格式解析（JSON 数组，一行一个图元）。

use crate::error::{Error, Result};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct EasyedaSymbol {
    pub pins: Vec<SymbolPin>,
    pub rects: Vec<SymbolRect>,
    pub polys: Vec<Vec<(f64, f64)>>,
    pub ellipses: Vec<SymbolEllipse>,
    pub part_box: Option<(f64, f64, f64, f64)>,
}

#[derive(Debug, Clone)]
pub struct SymbolPin {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub length: f64,
    pub rotation: f64,
    pub number: String,
    pub name: String,
    pub pin_type: String,
    /// 立创 `ATTR.NAME.valueVisible`。缺省当显示。
    pub show_name: bool,
    /// 立创 `ATTR.NUMBER.valueVisible`。缺省当显示。
    pub show_number: bool,
}

impl Default for SymbolPin {
    fn default() -> Self {
        Self {
            id: String::new(),
            x: 0.0,
            y: 0.0,
            length: 20.0,
            rotation: 0.0,
            number: String::new(),
            name: String::new(),
            pin_type: String::new(),
            show_name: true,
            show_number: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolRect {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Debug, Clone)]
pub struct SymbolEllipse {
    pub x: f64,
    pub y: f64,
    pub rx: f64,
    pub ry: f64,
}

#[derive(Debug, Clone, Default)]
pub struct EasyedaFootprint {
    pub pads: Vec<FootprintPad>,
    pub tracks: Vec<FootprintTrack>,
    pub circles: Vec<FootprintCircle>,
    pub arcs: Vec<FootprintArc>,
    pub regions: Vec<FootprintRegion>,
    pub model: EasyedaModel3d,
}

/// EasyEDA Pro `model_3d.transform`：尺寸 + ZXY 旋转 + 偏移（库坐标，mil）。
/// 官方顺序：sizeX,sizeY,sizeZ,rotZ,rotX,rotY,offX,offY,offZ
/// <https://prodocs.easyeda.com/en/format/pcb/component/>
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EasyedaModel3d {
    pub rot_x: f64,
    pub rot_y: f64,
    pub rot_z: f64,
    pub off_x: f64,
    pub off_y: f64,
    pub off_z: f64,
}

#[derive(Debug, Clone)]
pub struct FootprintPad {
    pub designator: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub hole: f64,
    pub hole_slot: f64,
    pub hole_shape: String,
    pub rotation: f64,
    pub layer: i32,
    pub shape: String,
    pub polygon: Option<Vec<(f64, f64)>>,
}

#[derive(Debug, Clone)]
pub struct FootprintTrack {
    pub layer: i32,
    pub width: f64,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone)]
pub struct FootprintCircle {
    pub layer: i32,
    pub width: f64,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

#[derive(Debug, Clone)]
pub struct FootprintArc {
    pub layer: i32,
    pub width: f64,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone)]
pub struct FootprintRegion {
    pub layer: i32,
    pub points: Vec<(f64, f64)>,
}

pub fn parse_component_json(value: &Value) -> Result<Vec<Value>> {
    let data_str = value
        .pointer("/result/dataStr")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("EasyEDA JSON 缺少 result.dataStr"))?;
    parse_datastr(data_str)
}

pub fn parse_datastr(data_str: &str) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for raw in data_str.lines() {
        let line = raw.trim();
        if line.is_empty() || !line.starts_with('[') {
            continue;
        }
        if let Ok(Value::Array(_)) = serde_json::from_str::<Value>(line) {
            rows.push(serde_json::from_str(line)?);
        }
    }
    Ok(rows)
}

pub fn parse_symbol(value: &Value) -> Result<EasyedaSymbol> {
    let rows = parse_component_json(value)?;
    let mut symbol = EasyedaSymbol::default();
    let mut attrs: HashMap<String, HashMap<String, AttrEntry>> = HashMap::new();

    for row in &rows {
        match row_type(row).as_str() {
            "PIN" => {
                let pin = SymbolPin {
                    id: get_string(row, 1),
                    x: get_f64(row, 4),
                    y: get_f64(row, 5),
                    length: {
                        let v = get_f64(row, 6);
                        if v == 0.0 { 20.0 } else { v }
                    },
                    rotation: get_f64(row, 7),
                    ..Default::default()
                };
                if !pin.id.is_empty() {
                    symbol.pins.push(pin);
                }
            }
            "ATTR" => {
                let parent = get_string(row, 2);
                let key = get_string(row, 3);
                let val = get_string(row, 4);
                if !parent.is_empty() && !key.is_empty() {
                    attrs.entry(parent).or_default().insert(
                        key,
                        AttrEntry {
                            value: val,
                            visible: json_bool(row.get(6)).unwrap_or(true),
                        },
                    );
                }
            }
            "PART" => {
                if let Some(meta) = row.get(2) {
                    if let Some(bbox) = meta.get("BBOX").and_then(Value::as_array) {
                        if bbox.len() >= 4 {
                            let x1 = json_f64(&bbox[0]);
                            let y1 = json_f64(&bbox[1]);
                            let x2 = json_f64(&bbox[2]);
                            let y2 = json_f64(&bbox[3]);
                            symbol.part_box = Some((x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)));
                        }
                    }
                }
            }
            "RECT" => symbol.rects.push(SymbolRect {
                x1: get_f64(row, 2),
                y1: get_f64(row, 3),
                x2: get_f64(row, 4),
                y2: get_f64(row, 5),
            }),
            "POLY" => {
                if let Some(shape) = row.get(2) {
                    let pts = parse_path_points(shape);
                    if pts.len() >= 2 {
                        symbol.polys.push(pts);
                    }
                }
            }
            "CIRCLE" => {
                let r = get_f64(row, 4).abs();
                if r > 1e-6 {
                    symbol.ellipses.push(SymbolEllipse {
                        x: get_f64(row, 2),
                        y: get_f64(row, 3),
                        rx: r,
                        ry: r,
                    });
                }
            }
            "ELLIPSE" => {
                let rx = get_f64(row, 4).abs();
                let ry = get_f64(row, 5).abs();
                if rx > 1e-6 && ry > 1e-6 {
                    symbol.ellipses.push(SymbolEllipse {
                        x: get_f64(row, 2),
                        y: get_f64(row, 3),
                        rx,
                        ry,
                    });
                }
            }
            _ => {}
        }
    }

    for (i, pin) in symbol.pins.iter_mut().enumerate() {
        let map = attrs.get(&pin.id);
        if let Some(a) = attr_get(map, &["NUMBER", "Pin Number"]) {
            if !a.value.is_empty() {
                pin.number = a.value.clone();
            }
            pin.show_number = a.visible;
        }
        if pin.number.is_empty() {
            pin.number = (i + 1).to_string();
        }
        if let Some(a) = attr_get(map, &["NAME", "Pin Name"]) {
            if !a.value.is_empty() {
                pin.name = a.value.clone();
            }
            pin.show_name = a.visible;
        }
        if pin.name.is_empty() {
            pin.name = pin.number.clone();
        }
        pin.pin_type = attr_get(map, &["Pin Type"])
            .map(|a| a.value.clone())
            .unwrap_or_default();
    }
    Ok(symbol)
}

#[derive(Debug, Clone)]
struct AttrEntry {
    value: String,
    visible: bool,
}

fn attr_get<'a>(
    map: Option<&'a HashMap<String, AttrEntry>>,
    keys: &[&str],
) -> Option<&'a AttrEntry> {
    let map = map?;
    keys.iter().find_map(|k| map.get(*k))
}

/// 符号级位号：`ATTR Designator` 或 `HEAD.c_para.pre`。
pub fn symbol_designator(value: &Value) -> Option<String> {
    let Ok(rows) = parse_component_json(value) else {
        return None;
    };
    let mut from_attr = None;
    let mut from_pre = None;
    for row in &rows {
        match row_type(row).as_str() {
            "ATTR" => {
                let parent = get_string(row, 2);
                let key = get_string(row, 3);
                let val = get_string(row, 4);
                if parent.is_empty() && key.eq_ignore_ascii_case("Designator") && !val.is_empty() {
                    from_attr = Some(val);
                }
            }
            "HEAD" => {
                let meta = row.get(1).or_else(|| row.get(2));
                if let Some(meta) = meta {
                    if let Some(pre) = meta
                        .pointer("/c_para/pre")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        from_pre = Some(pre.to_string());
                    } else if let Some(pre) = meta
                        .get("pre")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        from_pre = Some(pre.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    from_attr.or(from_pre)
}

/// 立创封装/符号的原始库名，优先 `result.display_title`。
pub fn component_display_title(value: &Value) -> Option<String> {
    let root = value.get("result").unwrap_or(value);
    for key in ["display_title", "title"] {
        if let Some(s) = root.get(key).and_then(Value::as_str) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub fn parse_footprint(value: &Value) -> Result<EasyedaFootprint> {
    let rows = parse_component_json(value)?;
    let mut fp = EasyedaFootprint {
        model: parse_model_3d(value),
        ..EasyedaFootprint::default()
    };
    let mut fallback = 1usize;

    for row in &rows {
        match row_type(row).as_str() {
            "PAD" => {
                let layer = get_i32(row, 4);
                let mut designator = get_string(row, 5);
                if designator.trim().is_empty() {
                    designator = fallback.to_string();
                    fallback += 1;
                }
                let x = get_f64(row, 6);
                let y = get_f64(row, 7);
                let mut rotation = get_f64_opt(row, 8).unwrap_or(f64::NAN);
                if rotation.is_nan() {
                    rotation = get_f64(row, 14);
                }
                let (hole_shape, hole, hole_slot) = parse_hole(row.get(9));
                let (shape, width, height, polygon) = parse_pad_shape(row.get(10));
                fp.pads.push(FootprintPad {
                    designator,
                    x,
                    y,
                    width: if width <= 0.0 { 10.0 } else { width },
                    height: if height <= 0.0 { width.max(10.0) } else { height },
                    hole,
                    hole_slot,
                    hole_shape,
                    rotation: if rotation.is_nan() { 0.0 } else { rotation },
                    layer,
                    shape,
                    polygon,
                });
            }
            "POLY" => {
                let layer = get_i32(row, 4);
                if !is_graphic_layer(layer) {
                    continue;
                }
                let stroke = get_f64(row, 5);
                if let Some(shape) = row.get(6) {
                    if let Some((cx, cy, r)) = try_circle_shape(shape) {
                        fp.circles.push(FootprintCircle {
                            layer,
                            width: stroke,
                            x: cx,
                            y: cy,
                            radius: r,
                        });
                    } else {
                        let pts = parse_path_points(shape);
                        if pts.len() >= 2 {
                            fp.tracks.push(FootprintTrack {
                                layer,
                                width: stroke,
                                points: pts,
                            });
                        }
                    }
                }
            }
            "TRACK" => {
                let layer = get_i32(row, 4);
                if !is_graphic_layer(layer) {
                    continue;
                }
                fp.tracks.push(FootprintTrack {
                    layer,
                    width: get_f64(row, 5),
                    points: vec![(get_f64(row, 6), get_f64(row, 7)), (get_f64(row, 8), get_f64(row, 9))],
                });
            }
            "RECT" => {
                let layer = get_i32(row, 4);
                if !is_graphic_layer(layer) {
                    continue;
                }
                let x1 = get_f64(row, 6);
                let y1 = get_f64(row, 7);
                let x2 = get_f64(row, 8);
                let y2 = get_f64(row, 9);
                fp.tracks.push(FootprintTrack {
                    layer,
                    width: get_f64(row, 5),
                    points: vec![(x1, y1), (x2, y1), (x2, y2), (x1, y2), (x1, y1)],
                });
            }
            "CIRCLE" => {
                let layer = get_i32(row, 4);
                if !is_graphic_layer(layer) {
                    continue;
                }
                let r = get_f64(row, 8).abs();
                if r > 1e-6 {
                    fp.circles.push(FootprintCircle {
                        layer,
                        width: get_f64(row, 5),
                        x: get_f64(row, 6),
                        y: get_f64(row, 7),
                        radius: r,
                    });
                }
            }
            "ARC" => {
                let layer = get_i32(row, 4);
                if !is_graphic_layer(layer) {
                    continue;
                }
                let r = get_f64(row, 8).abs();
                if r > 1e-6 {
                    fp.arcs.push(FootprintArc {
                        layer,
                        width: get_f64(row, 5),
                        x: get_f64(row, 6),
                        y: get_f64(row, 7),
                        radius: r,
                        start: normalize_angle(get_f64(row, 9)),
                        end: normalize_angle(get_f64(row, 10)),
                    });
                }
            }
            "FILL" => {
                let layer = get_i32(row, 4);
                if !is_graphic_layer(layer) {
                    continue;
                }
                if let Some(Value::Array(shapes)) = row.get(7) {
                    for shape in shapes {
                        if let Some((cx, cy, r)) = try_circle_shape(shape) {
                            let mut pts = Vec::new();
                            for i in 0..32 {
                                let a = std::f64::consts::TAU * i as f64 / 32.0;
                                pts.push((cx + r * a.cos(), cy + r * a.sin()));
                            }
                            fp.regions.push(FootprintRegion { layer, points: pts });
                        } else {
                            let mut pts = parse_path_points(shape);
                            if pts.len() >= 3 {
                                if pts.first() != pts.last() {
                                    if let Some(first) = pts.first().copied() {
                                        pts.push(first);
                                    }
                                }
                                fp.regions.push(FootprintRegion { layer, points: pts });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(fp)
}

fn parse_model_3d(value: &Value) -> EasyedaModel3d {
    value
        .pointer("/result/model_3d/transform")
        .and_then(Value::as_str)
        .map(parse_model_3d_transform)
        .unwrap_or_default()
}

/// 立创封装库里的九元组；不足 6 个数时当作没有姿态。
pub fn parse_model_3d_transform(raw: &str) -> EasyedaModel3d {
    let nums: Vec<f64> = raw
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    if nums.len() < 6 {
        return EasyedaModel3d::default();
    }
    EasyedaModel3d {
        rot_z: nums[3],
        rot_x: nums[4],
        rot_y: nums[5],
        off_x: nums.get(6).copied().unwrap_or(0.0),
        off_y: nums.get(7).copied().unwrap_or(0.0),
        off_z: nums.get(8).copied().unwrap_or(0.0),
    }
}

/// PCB drawings we keep. EasyEDA Pro names from the footprint LAYER table:
/// 3/4 silk, 12 multi, 13 document/courtyard, 48 component shape, 49 marking.
/// 50 PIN_SOLDERING / 51 PIN_FLOATING / 52 COMPONENT_MODEL are 3D visualization.
fn is_graphic_layer(code: i32) -> bool {
    matches!(code, 3 | 4 | 12 | 13 | 48 | 49)
}

fn parse_hole(el: Option<&Value>) -> (String, f64, f64) {
    match el {
        Some(Value::Array(arr)) => {
            let shape = arr.first().and_then(Value::as_str).unwrap_or("ROUND").to_string();
            let hole = arr.get(1).map(json_f64).unwrap_or(0.0);
            let slot = arr.get(2).map(json_f64).unwrap_or(hole);
            (shape, hole, slot)
        }
        Some(v) => {
            let hole = json_f64(v);
            ("ROUND".into(), hole, hole)
        }
        None => ("ROUND".into(), 0.0, 0.0),
    }
}

fn parse_pad_shape(el: Option<&Value>) -> (String, f64, f64, Option<Vec<(f64, f64)>>) {
    let Some(Value::Array(arr)) = el else {
        return ("ROUND".into(), 10.0, 10.0, None);
    };
    let shape = arr.first().and_then(Value::as_str).unwrap_or("ROUND").to_string();
    if shape.eq_ignore_ascii_case("POLY") {
        if let Some(poly) = arr.get(1) {
            let pts = parse_path_points(poly);
            if pts.len() >= 3 {
                let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
                let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
                let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                let width = (max_x - min_x).max(10.0);
                let height = (max_y - min_y).max(10.0);
                if is_axis_aligned_rect(&pts) {
                    return ("RECT".into(), width, height, None);
                }
                return (shape, width, height, Some(pts));
            }
        }
        (shape, 10.0, 10.0, None)
    } else {
        let w = arr.get(1).map(json_f64).unwrap_or(10.0);
        let h = arr.get(2).map(json_f64).unwrap_or(w);
        (shape, w, h, None)
    }
}

fn is_axis_aligned_rect(pts: &[(f64, f64)]) -> bool {
    let mut unique: Vec<(f64, f64)> = Vec::new();
    for &p in pts {
        if unique.iter().any(|&q| (p.0 - q.0).abs() < 1e-4 && (p.1 - q.1).abs() < 1e-4) {
            continue;
        }
        unique.push(p);
    }
    if unique.len() != 4 {
        return false;
    }
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for &(x, y) in &unique {
        if !xs.iter().any(|v| (x - v).abs() < 1e-4) {
            xs.push(x);
        }
        if !ys.iter().any(|v| (y - v).abs() < 1e-4) {
            ys.push(y);
        }
    }
    if xs.len() != 2 || ys.len() != 2 {
        return false;
    }
    for &x in &xs {
        for &y in &ys {
            if !unique.iter().any(|p| (p.0 - x).abs() < 1e-4 && (p.1 - y).abs() < 1e-4) {
                return false;
            }
        }
    }
    true
}

fn try_circle_shape(shape: &Value) -> Option<(f64, f64, f64)> {
    let arr = shape.as_array()?;
    if arr.len() < 4 {
        return None;
    }
    if !arr[0].as_str()?.eq_ignore_ascii_case("CIRCLE") {
        return None;
    }
    let r = json_f64(&arr[3]).abs();
    if r <= 1e-6 {
        return None;
    }
    Some((json_f64(&arr[1]), json_f64(&arr[2]), r))
}

fn parse_path_points(shape: &Value) -> Vec<(f64, f64)> {
    let Some(arr) = shape.as_array() else {
        return Vec::new();
    };
    if arr.first().and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("CIRCLE")) {
        return Vec::new();
    }
    let mut pts = Vec::new();
    let mut i = 0;
    while i < arr.len() {
        if let Some(cmd) = arr[i].as_str() {
            i += 1;
            match cmd.trim().to_ascii_uppercase().as_str() {
                "L" => {
                    while i + 1 < arr.len() {
                        if let (Some(x), Some(y)) = (as_number(&arr[i]), as_number(&arr[i + 1])) {
                            push_pt(&mut pts, x, y);
                            i += 2;
                        } else {
                            break;
                        }
                    }
                }
                "ARC" | "A" => {
                    if i + 2 < arr.len() {
                        if let (Some(_), Some(ex), Some(ey)) =
                            (as_number(&arr[i]), as_number(&arr[i + 1]), as_number(&arr[i + 2]))
                        {
                            push_pt(&mut pts, ex, ey);
                            i += 3;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if i + 1 < arr.len() {
            if let (Some(x), Some(y)) = (as_number(&arr[i]), as_number(&arr[i + 1])) {
                push_pt(&mut pts, x, y);
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    pts
}

fn push_pt(pts: &mut Vec<(f64, f64)>, x: f64, y: f64) {
    if let Some(last) = pts.last() {
        if (last.0 - x).abs() < 1e-9 && (last.1 - y).abs() < 1e-9 {
            return;
        }
    }
    pts.push((x, y));
}

fn row_type(row: &Value) -> String {
    get_string(row, 0)
}

fn get_string(row: &Value, idx: usize) -> String {
    match row.get(idx) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn get_f64(row: &Value, idx: usize) -> f64 {
    row.get(idx).map(json_f64).unwrap_or(0.0)
}

fn get_f64_opt(row: &Value, idx: usize) -> Option<f64> {
    row.get(idx).and_then(|v| {
        if v.is_null() || (v.as_str().is_some_and(|s| s.is_empty())) {
            None
        } else {
            Some(json_f64(v))
        }
    })
}

fn get_i32(row: &Value, idx: usize) -> i32 {
    get_f64(row, idx) as i32
}

fn json_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// 立创 `valueVisible`：bool / 0/1 / "true"|"false"。
fn json_bool(v: Option<&Value>) -> Option<bool> {
    match v {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => Some(n.as_i64().unwrap_or(0) != 0),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn normalize_angle(v: f64) -> f64 {
    let mut a = v % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn symbol_designator_reads_top_level_attr() {
        let json: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/FNR3015S2R2MT_symbol_easyeda.json"
        ))
        .unwrap();
        assert_eq!(symbol_designator(&json).as_deref(), Some("L?"));
        let head = json!({
            "result": {
                "dataStr": "[\"HEAD\",{\"c_para\":{\"pre\":\"SW?\"}}]\n"
            }
        });
        assert_eq!(symbol_designator(&head).as_deref(), Some("SW?"));
    }

    #[test]
    fn component_display_title_prefers_result_display_title() {
        let v = json!({
            "result": {
                "display_title": "SOIC-8_L5.3-W5.3-P1.27-LS6.00-BL",
                "title": "other"
            }
        });
        assert_eq!(
            component_display_title(&v).as_deref(),
            Some("SOIC-8_L5.3-W5.3-P1.27-LS6.00-BL")
        );
    }

    #[test]
    fn parses_pin_row() {
        let ds = r#"["PIN","p1","","",10,20,30,0]
["ATTR","x","p1","NUMBER","1"]
["ATTR","x","p1","NAME","VCC"]
"#;
        let rows = parse_datastr(ds).unwrap();
        assert_eq!(rows.len(), 3);
        let fake = json!({"result": {"dataStr": ds}});
        let sym = parse_symbol(&fake).unwrap();
        assert_eq!(sym.pins.len(), 1);
        assert_eq!(sym.pins[0].number, "1");
        assert_eq!(sym.pins[0].name, "VCC");
        assert!(sym.pins[0].show_name);
        assert!(sym.pins[0].show_number);
    }

    #[test]
    fn pin_attr_value_visible_matches_easyeda() {
        let ds = r#"["PART","LED.1",{"BBOX":[-8,-12,8,12]}]
["PIN","p1","","",-20,0,10,0]
["ATTR","a","p1","NAME","KA1",false,false,-6,0,0,"st3",0]
["ATTR","b","p1","NUMBER","1",false,true,-18,2,0,"st4",0]
["PIN","p2","","",20,0,10,180]
["ATTR","c","p2","Pin Name","KA2",false,0,6,0,0,"st3",0]
["ATTR","d","p2","Pin Number","2",false,1,18,2,0,"st4",0]
"#;
        let sym = parse_symbol(&json!({"result": {"dataStr": ds}})).unwrap();
        assert_eq!(sym.part_box, Some((-8.0, -12.0, 8.0, 12.0)));
        assert!(sym.rects.is_empty(), "BBOX is not a RECT");
        assert_eq!(sym.pins[0].name, "KA1");
        assert!(!sym.pins[0].show_name);
        assert!(sym.pins[0].show_number);
        assert_eq!(sym.pins[1].name, "KA2");
        assert!(!sym.pins[1].show_name);
        assert!(sym.pins[1].show_number);
    }

    #[test]
    fn poly_pad_axis_aligned_becomes_rect() {
        let ds = r#"["DOCTYPE","FOOTPRINT","1.0"]
["PAD","e1",0,"",1,"1",0,0,0,null,["POLY",[-10,-5,"L",10,-5,10,5,-10,5,-10,-5]],[],0,0,0,1]
"#;
        let fp = parse_footprint(&json!({"result": {"dataStr": ds}})).unwrap();
        assert_eq!(fp.pads.len(), 1);
        assert_eq!(fp.pads[0].shape, "RECT");
        assert!(fp.pads[0].polygon.is_none());
    }

    #[test]
    fn poly_pad_keeps_irregular_outline() {
        let ds = r#"["DOCTYPE","FOOTPRINT","1.0"]
["PAD","e1",0,"",1,"1",0,0,0,null,["POLY",[0,0,"L",20,0,10,12]],[],0,0,0,1]
"#;
        let fp = parse_footprint(&json!({"result": {"dataStr": ds}})).unwrap();
        assert_eq!(fp.pads[0].shape, "POLY");
        assert_eq!(fp.pads[0].polygon.as_ref().map(Vec::len), Some(3));
    }

    #[test]
    fn skips_3d_body_construction_but_keeps_assembly_fill() {
        let ds = r#"["DOCTYPE","FOOTPRINT","1.8"]
["POLY","e0",0,"",48,2,[-10,-10,"L",-10,10,10,10,10,-10,-10,-10],0]
["FILL","e1",0,"",49,0.2,0,[["CIRCLE",0,0,2]],0]
["FILL","e2",0,"",50,0.2,0,[[-4,-8,"L",-4,-2,4,-2,4,-8,-4,-8]],0]
["FILL","e3",0,"",51,0.2,0,[[-4,2,"L",-4,8,4,8,4,2,-4,2]],0]
["POLY","e4",0,"",3,6,[-20,-15,"L",20,-15],0]
"#;
        let fp = parse_footprint(&json!({"result": {"dataStr": ds}})).unwrap();
        assert_eq!(fp.tracks.len(), 2, "silk + component-shape outline");
        assert!(fp.tracks.iter().any(|t| t.layer == 3));
        assert!(fp.tracks.iter().any(|t| t.layer == 48));
        assert_eq!(fp.regions.len(), 1, "marking FILL on 49 stays; 50/51 are 3D pins");
        assert_eq!(fp.regions[0].layer, 49);
        assert!(fp.circles.is_empty());
    }

    #[test]
    fn parses_pro_model_transform_as_rotz_rotx_roty() {
        let ds90 = parse_model_3d_transform("252.188,196.85,0,90,0,0,0,0.005,0");
        assert!((ds90.rot_z - 90.0).abs() < 1e-9);
        assert!(ds90.rot_x.abs() < 1e-9);
        assert!(ds90.rot_y.abs() < 1e-9);
        assert!(ds90.off_x.abs() < 1e-9);
        assert!((ds90.off_y - 0.005).abs() < 1e-9);
        let fnr = parse_model_3d_transform("118.11,118.11,0,90,0,0,0,0,0");
        assert!((fnr.rot_z - 90.0).abs() < 1e-9);
        let empty = parse_model_3d_transform("");
        assert_eq!(empty, EasyedaModel3d::default());
    }
}
