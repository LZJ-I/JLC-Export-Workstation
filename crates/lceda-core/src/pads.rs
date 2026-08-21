//! PADS Logic / Layout ASCII 库（V9）：`.c` 原理图 decal、`.d` 封装、`.p` 器件类型。
//!
//! 规格：Mentor *PADS Parts Library ASCII Format*（`*PADS-LIBRARY-*-V9*`）。
//! 仅经典 PADS Logic / Layout 可导入；PADS Professional / Xpedition 不能当 Central Library。

use crate::error::Result;
use crate::ir::{FootprintIr, IrPad, IrPin, SymbolIr};
use crate::util::ensure_parent;
use std::collections::HashSet;
use std::path::Path;

const TS: &str = "2026.8.21.0.0.0";
const FONT: &str = "\"Default Font\"";
/// PADS Logic/Layout 拒绝线宽 0（报「图形宽度不正确 0」）。
const MIN_LINE_MIL: f64 = 5.0;

pub fn write_part_files(
    dir: &Path,
    base: &str,
    symbol: Option<&SymbolIr>,
    footprint: Option<&FootprintIr>,
) -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>, Option<std::path::PathBuf>)> {
    let ident = pads_ident(base, 40);
    let mut c = None;
    let mut d = None;
    let mut p = None;
    if let Some(sym) = symbol {
        let path = dir.join(format!("{ident}.c"));
        write_text(&path, &sch_library(&[sym]))?;
        c = Some(path);
    }
    if let Some(fp) = footprint {
        let path = dir.join(format!("{ident}.d"));
        write_text(&path, &pcb_library(&[fp]))?;
        d = Some(path);
    }
    if symbol.is_some() || footprint.is_some() {
        let path = dir.join(format!("{ident}.p"));
        write_text(&path, &part_library(&[(symbol, footprint)]))?;
        p = Some(path);
    }
    Ok((c, d, p))
}

pub fn sch_library(symbols: &[&SymbolIr]) -> String {
    let mut out = String::from("*PADS-LIBRARY-SCH-DECALS-V9*\n\n");
    for (i, sym) in symbols.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&sch_decal(sym));
    }
    out.push_str("*END*\n");
    out
}

pub fn pcb_library(footprints: &[&FootprintIr]) -> String {
    let mut out = String::from("*PADS-LIBRARY-PCB-DECALS-V9*\n\n");
    for (i, fp) in footprints.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&pcb_decal(fp));
    }
    out.push_str("*END*\n");
    out
}

pub fn part_library(parts: &[(Option<&SymbolIr>, Option<&FootprintIr>)]) -> String {
    let mut out = String::from("*PADS-LIBRARY-PART-TYPES-V9*\n\n");
    for (i, (sym, fp)) in parts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&part_type(*sym, *fp));
    }
    out.push_str("*END*\n");
    out
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    ensure_parent(path)?;
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    std::fs::write(path, crlf)?;
    Ok(())
}

fn sch_decal(symbol: &SymbolIr) -> String {
    let name = pads_ident(&symbol.name, 40);
    let mut pieces = Vec::new();
    for r in &symbol.rects {
        let x1 = mils_snap(r.x1);
        let y1 = mils_snap(r.y1);
        let x2 = mils_snap(r.x2);
        let y2 = mils_snap(r.y2);
        pieces.push(closed_rect(x1, y1, x2, y2, 10.0, 0));
    }
    for poly in &symbol.polys {
        if let Some(p) = open_poly(poly, true, 10.0, 0) {
            pieces.push(p);
        }
    }
    for e in &symbol.ellipses {
        pieces.push(circle_piece(mils_snap(e.x), mils_snap(e.y), mils_snap(e.rx.max(e.ry)), 10.0, 0));
    }
    for pin in &symbol.pins {
        if let Some(p) = pin_shaft(pin) {
            pieces.push(p);
        }
    }

    let terms: Vec<String> = symbol.pins.iter().map(sch_terminal).collect();
    let labels = 2;
    let txt = 0;
    let vis = 0;
    let mut out = format!(
        "{name} 0 0 50 8 50 8 {labels} {pieces} {txt} {terms} {vis}\nTIMESTAMP {TS}\n{FONT}\n{FONT}\n",
        pieces = pieces.len(),
        terms = terms.len(),
    );
    let (ref_x, ref_y, type_x, type_y) = sch_label_xy(symbol);
    out.push_str(&format!("{ref_x} {ref_y} 0 0 50 8 {FONT}\nREF-DES\n"));
    out.push_str(&format!("{type_x} {type_y} 0 0 50 8 {FONT}\nPARTTYPE\n"));
    for p in pieces {
        out.push_str(&p);
    }
    for t in terms {
        out.push_str(&t);
    }
    out
}

fn sch_label_xy(symbol: &SymbolIr) -> (f64, f64, f64, f64) {
    let mut top = symbol
        .rects
        .iter()
        .map(|r| mils_snap(r.y2.max(r.y1)))
        .fold(0.0_f64, f64::max);
    let mut bot = symbol
        .rects
        .iter()
        .map(|r| mils_snap(r.y2.min(r.y1)))
        .fold(0.0_f64, f64::min);
    for pin in &symbol.pins {
        top = top.max(mils_snap(pin.y));
        bot = bot.min(mils_snap(pin.y));
    }
    if (top - bot).abs() < 1.0 {
        top = 200.0;
        bot = -200.0;
    }
    (0.0, top + 80.0, 0.0, bot - 80.0)
}

fn pin_shaft(pin: &IrPin) -> Option<String> {
    let len = mils_snap(pin.length.max(1.27));
    if len < 5.0 {
        return None;
    }
    let (x, y) = (mils_snap(pin.x), mils_snap(pin.y));
    let (dx, dy) = body_delta(pin.rotation, len);
    let bx = snap10(x + dx);
    let by = snap10(y + dy);
    if (bx - x).abs() < 0.5 && (by - y).abs() < 0.5 {
        return None;
    }
    Some(format!(
        "OPEN 2 10 0 1\n{} {}\n{} {}\n",
        n(x),
        n(y),
        n(bx),
        n(by)
    ))
}

fn sch_terminal(pin: &IrPin) -> String {
    let x = mils_snap(pin.x);
    let y = mils_snap(pin.y);
    let len = mils_snap(pin.length.max(1.27));
    let (dx, dy) = body_delta(pin.rotation, 1.0);
    let rtn = if dx.abs() >= dy.abs() { 0 } else { 90 };
    // 位号在电连接外侧，管脚名在本体内侧（PADS 习惯）。
    let pnx = snap10(-dx * 50.0);
    let pny = snap10(-dy * 50.0);
    let name_off = (len * 0.25 + 30.0).clamp(30.0, 80.0);
    let pnmx = snap10(dx * name_off);
    let pnmy = snap10(dy * name_off);
    let (pnjust, pnmjust) = pin_text_just(dx, dy);
    format!(
        "T{} {} {rtn} 0 {} {} 0 {pnjust} {} {} 0 {pnmjust} 0\nP 0 0 0 0 0 0 0 0 192\n",
        n(x),
        n(y),
        n(pnx),
        n(pny),
        n(pnmx),
        n(pnmy)
    )
}

fn pin_text_just(dx: f64, dy: f64) -> (i32, i32) {
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 {
            (2, 0)
        } else {
            (0, 2)
        }
    } else if dy >= 0.0 {
        (2, 0)
    } else {
        (0, 2)
    }
}

fn pcb_decal(fp: &FootprintIr) -> String {
    let name = pads_ident(&fp.name, 40);
    let mut pieces = Vec::new();
    for t in &fp.tracks {
        if !silk_layer(t.layer) {
            continue;
        }
        if let Some(p) = open_poly(&t.points, false, line_mil(t.width), silk_pads_layer(t.layer)) {
            pieces.push(p);
        }
    }
    for c in &fp.circles {
        if !silk_layer(c.layer) {
            continue;
        }
        pieces.push(circle_piece(
            mils(c.x),
            mils(c.y),
            mils(c.radius),
            line_mil(c.width),
            silk_pads_layer(c.layer),
        ));
    }
    for r in &fp.regions {
        if r.points.len() < 3 {
            continue;
        }
        if silk_layer(r.layer) {
            if let Some(p) = closed_poly(&r.points, MIN_LINE_MIL, silk_pads_layer(r.layer)) {
                pieces.push(p);
            }
        } else if r.layer == 1 {
            if let Some(p) = copcls(&r.points, 1) {
                pieces.push(p);
            }
        }
    }

    let (cx, cy) = silk_center(fp);
    let terms: Vec<(String, String)> = fp
        .pads
        .iter()
        .enumerate()
        .map(|(i, pad)| pcb_terminal(pad, i + 1, cx, cy))
        .collect();
    let stacks: Vec<String> = fp
        .pads
        .iter()
        .enumerate()
        .map(|(i, pad)| pad_stack(pad, i + 1))
        .collect();

    let labels = 2;
    let txt = 0;
    let attrs = 0;
    let maxlayers = 0;
    let mut out = format!(
        "{name} I 0 0 {attrs} {labels} {pieces} {txt} {terms} {stacks} {maxlayers}\nTIMESTAMP {TS}\n",
        pieces = pieces.len(),
        terms = terms.len(),
        stacks = stacks.len(),
    );
    out.push_str(&format!(
        "{} {} 0 0 50 5 26 0 33 {FONT}\nREF-DES\n",
        n(cx),
        n(cy + 50.0)
    ));
    out.push_str(&format!(
        "{} {} 0 0 50 5 26 0 34 {FONT}\nPARTTYPE\n",
        n(cx),
        n(cy - 50.0)
    ));
    for p in pieces {
        out.push_str(&p);
    }
    for (t, _) in &terms {
        out.push_str(t);
    }
    for s in stacks {
        out.push_str(&s);
    }
    out
}

fn pcb_terminal(pad: &IrPad, idx: usize, cx: f64, cy: f64) -> (String, String) {
    let pin = pcb_pin(pad, idx);
    let x = mils(pad.x);
    let y = mils(pad.y);
    let mut vx = x - cx;
    let mut vy = y - cy;
    let mag = (vx * vx + vy * vy).sqrt();
    if mag < 5.0 {
        vx = 0.0;
        vy = 1.0;
    } else {
        vx /= mag;
        vy /= mag;
    }
    let reach = mils(pad.width).max(mils(pad.height)).max(20.0) * 0.55 + 12.0;
    let nx = x + vx * reach;
    let ny = y + vy * reach;
    (
        format!("T{} {} {} {} {pin}\n", n(x), n(y), n(nx), n(ny)),
        pin,
    )
}

fn pad_stack(pad: &IrPad, idx: usize) -> String {
    let pin = pcb_pin(pad, idx);
    let drill = mils(pad.hole);
    let plated = "P";
    let mut w = mils(pad.width).max(1.0);
    let mut h = mils(pad.height).max(1.0);
    let mut ori = wrap180(pad.rotation);
    if w + 0.01 < h {
        std::mem::swap(&mut w, &mut h);
        ori = wrap180(ori + 90.0);
    }
    let round = is_round(&pad.shape, pad.width, pad.height);
    let smd = drill <= 0.05;
    let mut header = format!("PAD {pin} 3 {plated} {}", n(drill));
    if pad.hole_slot > 0.05 {
        header.push_str(&format!(" {} {} 0", n(ori), n(mils(pad.hole_slot))));
    }
    header.push('\n');

    let top = if round {
        format!("-2 {} R\n", n(w.max(h)))
    } else if (w - h).abs() < 0.5 {
        format!("-2 {} S 0\n", n(w))
    } else {
        format!("-2 {} RF 0 {} {} 0\n", n(h), n(ori), n(w))
    };
    if smd {
        format!("{header}{top}-1 0 R\n0 0 R\n")
    } else {
        let inner = if round {
            format!("-1 {} R\n", n(w.max(h)))
        } else if (w - h).abs() < 0.5 {
            format!("-1 {} S 0\n", n(w))
        } else {
            format!("-1 {} RF 0 {} {} 0\n", n(h), n(ori), n(w))
        };
        format!("{header}{top}{inner}{}", inner.replace("-1", "0"))
    }
}

fn part_type(symbol: Option<&SymbolIr>, footprint: Option<&FootprintIr>) -> String {
    let part_name = pads_ident(
        symbol
            .map(|s| s.name.as_str())
            .or_else(|| footprint.map(|f| f.name.as_str()))
            .unwrap_or("PART"),
        40,
    );
    let pcb_name = footprint
        .map(|f| pads_ident(&f.name, 40))
        .unwrap_or_else(|| "0".into());
    let cae_name = symbol
        .map(|s| pads_ident(&s.name, 40))
        .unwrap_or_else(|| part_name.clone());
    let pins = symbol.map(|s| s.pins.as_slice()).unwrap_or(&[]);
    let attrs = attr_count(symbol, footprint);
    let gates = if symbol.is_some() { 1 } else { 0 };
    let mut out = format!("{part_name} {pcb_name} I UND {attrs} {gates} 0 0 0\nTIMESTAMP {TS}\n");
    if let Some(meta) = symbol.map(|s| &s.meta).or_else(|| footprint.map(|f| &f.meta)) {
        if !meta.lcsc.is_empty() {
            out.push_str(&format!("\"LCSC\" {}\n", clean_attr(&meta.lcsc)));
        }
        if !meta.mpn.is_empty() {
            out.push_str(&format!("\"MPN\" {}\n", clean_attr(&meta.mpn)));
        }
        if !meta.manufacturer.is_empty() {
            out.push_str(&format!("\"Manufacturer\" {}\n", clean_attr(&meta.manufacturer)));
        }
        if !meta.datasheet.is_empty() {
            out.push_str(&format!("\"Datasheet\" {}\n", clean_attr(&meta.datasheet)));
        }
    }
    if symbol.is_some() {
        out.push_str(&format!("GATE 1 {} 0\n{cae_name}\n", pins.len().max(1)));
        let mut used = HashSet::new();
        let mut used_names = HashSet::new();
        if pins.is_empty() {
            out.push_str("1 0 U NC\n");
        } else {
            for (i, pin) in pins.iter().enumerate() {
                let num = unique_pin_num(&pin.number, i + 1, &mut used);
                let et = elec_type(&pin.pin_type);
                let nm = unique_pin_name(&pin.name, &num, &mut used_names);
                out.push_str(&format!("{num} 0 {et} {nm}\n"));
            }
        }
    }
    out
}

fn attr_count(symbol: Option<&SymbolIr>, footprint: Option<&FootprintIr>) -> usize {
    let meta = symbol.map(|s| &s.meta).or_else(|| footprint.map(|f| &f.meta));
    let Some(m) = meta else { return 0 };
    let mut n = 0;
    if !m.lcsc.is_empty() {
        n += 1;
    }
    if !m.mpn.is_empty() {
        n += 1;
    }
    if !m.manufacturer.is_empty() {
        n += 1;
    }
    if !m.datasheet.is_empty() {
        n += 1;
    }
    n
}

fn closed_rect(x1: f64, y1: f64, x2: f64, y2: f64, width: f64, layer: i32) -> String {
    format!(
        "CLOSED 5 {} {layer} 1\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}\n",
        n(width),
        n(x1),
        n(y1),
        n(x2),
        n(y1),
        n(x2),
        n(y2),
        n(x1),
        n(y2),
        n(x1),
        n(y1),
    )
}

fn open_poly(pts: &[(f64, f64)], sch: bool, width: f64, layer: i32) -> Option<String> {
    if pts.len() < 2 {
        return None;
    }
    let conv = |p: (f64, f64)| if sch { (mils_snap(p.0), mils_snap(p.1)) } else { (mils(p.0), mils(p.1)) };
    let pts: Vec<(f64, f64)> = pts.iter().copied().map(conv).collect();
    let mut out = format!("OPEN {} {} {layer} 1\n", pts.len(), n(width));
    for (x, y) in pts {
        out.push_str(&format!("{} {}\n", n(x), n(y)));
    }
    Some(out)
}

fn closed_poly(pts: &[(f64, f64)], width: f64, layer: i32) -> Option<String> {
    if pts.len() < 3 {
        return None;
    }
    let mut out = format!(
        "CLOSED {} {} {layer} 1\n",
        pts.len() + 1,
        n(width.max(MIN_LINE_MIL))
    );
    for &(x, y) in pts {
        out.push_str(&format!("{} {}\n", n(mils(x)), n(mils(y))));
    }
    let (x, y) = pts[0];
    out.push_str(&format!("{} {}\n", n(mils(x)), n(mils(y))));
    Some(out)
}

fn copcls(pts: &[(f64, f64)], layer: i32) -> Option<String> {
    if pts.len() < 3 {
        return None;
    }
    let mut out = format!("COPCLS {} {} {layer} 1\n", pts.len() + 1, n(MIN_LINE_MIL));
    for &(x, y) in pts {
        out.push_str(&format!("{} {}\n", n(mils(x)), n(mils(y))));
    }
    let (x, y) = pts[0];
    out.push_str(&format!("{} {}\n", n(mils(x)), n(mils(y))));
    Some(out)
}

fn circle_piece(x: f64, y: f64, r: f64, width: f64, layer: i32) -> String {
    let r = r.max(1.0);
    format!(
        "CIRCLE 2 {} {layer} 1\n{} {}\n{} {}\n",
        n(width),
        n(x - r),
        n(y),
        n(x + r),
        n(y)
    )
}

fn silk_center(fp: &FootprintIr) -> (f64, f64) {
    if fp.pads.is_empty() {
        return (0.0, 0.0);
    }
    let sx: f64 = fp.pads.iter().map(|p| mils(p.x)).sum();
    let sy: f64 = fp.pads.iter().map(|p| mils(p.y)).sum();
    let n = fp.pads.len() as f64;
    (sx / n, sy / n)
}

fn silk_layer(layer: i32) -> bool {
    matches!(layer, 3 | 4 | 11 | 12 | 13 | 48 | 49)
}

fn silk_pads_layer(layer: i32) -> i32 {
    match layer {
        4 => 29,
        _ => 26,
    }
}

fn is_round(shape: &str, w: f64, h: f64) -> bool {
    let s = shape.to_ascii_uppercase();
    s.contains("CIRC") || ((w - h).abs() < 1e-4 && !s.contains("RECT") && !s.contains("POLY"))
}

fn pcb_pin(pad: &IrPad, idx: usize) -> String {
    pin_num_token(&pad.designator, idx)
}

fn pin_num_token(raw: &str, fallback: usize) -> String {
    let t: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(7)
        .collect();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t
    }
}

fn unique_pin_num(raw: &str, fallback: usize, used: &mut HashSet<String>) -> String {
    let mut n = pin_num_token(raw, fallback);
    if !used.insert(n.clone()) {
        n = fallback.to_string();
        used.insert(n.clone());
    }
    n
}

fn unique_pin_name(raw: &str, fallback: &str, used: &mut HashSet<String>) -> String {
    let base = pin_name_token(raw, fallback);
    if used.insert(base.clone()) {
        return base;
    }
    for i in 2..10000 {
        let cand = format!("{base}_{i}");
        let cand: String = cand.chars().take(40).collect();
        if used.insert(cand.clone()) {
            return cand;
        }
    }
    fallback.to_string()
}

fn pin_name_token(raw: &str, fallback: &str) -> String {
    let t: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let t = t.trim_matches('_');
    if t.is_empty() {
        fallback.chars().take(40).collect()
    } else {
        t.chars().take(40).collect()
    }
}

fn elec_type(src: &str) -> &'static str {
    match src.to_ascii_uppercase().as_str() {
        "IN" | "INPUT" => "L",
        "OUT" | "OUTPUT" => "S",
        "I/O" | "IO" | "BIDIR" | "BIDIRECTIONAL" => "B",
        "PWR" | "POWER" | "POWER_IN" | "POWER_OUT" => "P",
        "GND" | "GROUND" => "G",
        "NC" | "NO_CONNECT" => "Z",
        _ => "U",
    }
}

fn clean_attr(s: &str) -> String {
    let ascii: String = s
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .collect();
    ascii.replace("()", "").trim().chars().take(2000).collect()
}

pub fn pads_ident(name: &str, max: usize) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let t = out.trim_matches('_');
    let s = if t.is_empty() { "PART" } else { t };
    s.chars().take(max).collect()
}

fn mils(mm: f64) -> f64 {
    mm * 1000.0 / 25.4
}

fn line_mil(mm: f64) -> f64 {
    mils(mm).max(MIN_LINE_MIL)
}

fn mils_snap(mm: f64) -> f64 {
    snap10(mils(mm))
}

fn snap10(v: f64) -> f64 {
    (v / 10.0).round() * 10.0
}

fn wrap180(deg: f64) -> f64 {
    let mut a = deg % 180.0;
    if a < 0.0 {
        a += 180.0;
    }
    if a >= 179.999 {
        179.998
    } else {
        a
    }
}

/// EasyEDA：0° 为左侧脚（电端在外、本体在 +X）。与 AD 写出相同，先 +180 再取象限。
fn body_delta(rot_deg: f64, len: f64) -> (f64, f64) {
    let a = (((rot_deg + 180.0) / 90.0).round() as i32).rem_euclid(4);
    match a {
        0 => (-len, 0.0),
        1 => (0.0, -len),
        2 => (len, 0.0),
        _ => (0.0, len),
    }
}

fn n(v: f64) -> String {
    if v.abs() < 1e-9 {
        return "0".into();
    }
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrPad, IrPin, IrRect, IrRegion, PartMeta};

    fn sample_symbol() -> SymbolIr {
        SymbolIr {
            name: "RES".into(),
            description: "test".into(),
            meta: PartMeta {
                lcsc: "C2040".into(),
                mpn: "RES".into(),
                ..Default::default()
            },
            pins: vec![
                IrPin {
                    number: "1".into(),
                    name: "1".into(),
                    x: -2.54,
                    y: 0.0,
                    length: 2.54,
                    rotation: 0.0,
                    pin_type: "PASSIVE".into(),
                },
                IrPin {
                    number: "2".into(),
                    name: "2".into(),
                    x: 2.54,
                    y: 0.0,
                    length: 2.54,
                    rotation: 180.0,
                    pin_type: "PASSIVE".into(),
                },
            ],
            rects: vec![IrRect {
                x1: -1.0,
                y1: -1.0,
                x2: 1.0,
                y2: 1.0,
            }],
            polys: vec![],
            ellipses: vec![],
        }
    }

    fn sample_fp() -> FootprintIr {
        FootprintIr {
            name: "R0402".into(),
            description: "0402".into(),
            meta: PartMeta::default(),
            pads: vec![IrPad {
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
            }],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
        }
    }

    #[test]
    fn sch_has_header_and_terminals() {
        let text = sch_library(&[&sample_symbol()]);
        assert!(text.starts_with("*PADS-LIBRARY-SCH-DECALS-V9*"));
        assert!(text.contains("REF-DES"));
        assert!(text.contains("PARTTYPE"));
        assert!(text.contains("CLOSED"));
        assert!(text.contains('\n') && text.contains("T"));
        assert!(text.contains("*END*"));
    }

    #[test]
    fn sch_left_pin_shaft_goes_inward() {
        let text = sch_library(&[&sample_symbol()]);
        assert!(
            text.contains("OPEN 2 10 0 1\n-100 0\n0 0\n"),
            "left pin shaft should go toward body (+X): {text}"
        );
        let term = text
            .lines()
            .find(|l| l.starts_with("T-100 0 "))
            .unwrap_or("");
        let cols: Vec<&str> = term.split_whitespace().collect();
        assert!(cols.len() >= 6, "terminal: {term}");
        let pnx: f64 = cols[4].parse().unwrap();
        assert!(pnx < 0.0, "pin number should sit outside (left): {term}");
    }

    #[test]
    fn pcb_has_pad_stack() {
        let text = pcb_library(&[&sample_fp()]);
        assert!(text.starts_with("*PADS-LIBRARY-PCB-DECALS-V9*"));
        assert!(text.contains("PAD 1 3"));
        assert!(text.contains("-2 "));
        assert!(text.contains("*END*"));
    }

    #[test]
    fn pcb_closed_piece_never_zero_width() {
        let mut fp = sample_fp();
        fp.regions = vec![IrRegion {
            layer: 3,
            points: vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)],
        }];
        let text = pcb_library(&[&fp]);
        assert!(
            !text.lines().any(|l| l.starts_with("CLOSED ") && l.split_whitespace().nth(2) == Some("0")),
            "CLOSED width 0 rejected by PADS: {text}"
        );
        assert!(text.contains("CLOSED 4 5 26 1"));
    }

    #[test]
    fn pcb_pin_numbers_push_outward() {
        let mut fp = sample_fp();
        fp.pads.push(IrPad {
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
        });
        let text = pcb_library(&[&fp]);
        let mut n1 = None;
        let mut n2 = None;
        for line in text.lines() {
            if !line.starts_with('T') || line.starts_with("TIMESTAMP") {
                continue;
            }
            let rest = line.trim_start_matches('T');
            let cols: Vec<&str> = rest.split_whitespace().collect();
            if cols.len() < 5 {
                continue;
            }
            let px: f64 = cols[0].parse().unwrap();
            let lx: f64 = cols[2].parse().unwrap();
            match cols[4] {
                "1" => n1 = Some((px, lx)),
                "2" => n2 = Some((px, lx)),
                _ => {}
            }
        }
        let (p1, l1) = n1.expect("pin 1");
        let (p2, l2) = n2.expect("pin 2");
        assert!(l1 < p1, "pin 1 number should sit further left: {p1} -> {l1}");
        assert!(l2 > p2, "pin 2 number should sit further right: {p2} -> {l2}");
    }

    #[test]
    fn part_binds_cae_and_pcb() {
        let s = sample_symbol();
        let f = sample_fp();
        let text = part_library(&[(Some(&s), Some(&f))]);
        assert!(text.starts_with("*PADS-LIBRARY-PART-TYPES-V9*"));
        assert!(text.contains("R0402"));
        assert!(text.contains("GATE 1 2 0"));
        assert!(text.contains("LCSC"));
        assert!(text.contains("*END*"));
        let s2 = {
            let mut s = sample_symbol();
            s.pins.push(IrPin {
                number: "3".into(),
                name: "GND".into(),
                x: 0.0,
                y: -2.54,
                length: 2.54,
                rotation: 270.0,
                pin_type: "PASSIVE".into(),
            });
            s.pins[0].name = "GND".into();
            s
        };
        let text = part_library(&[(Some(&s2), Some(&sample_fp()))]);
        assert!(text.contains("GND_2"), "duplicate pin names must be uniquified: {text}");
    }
}
