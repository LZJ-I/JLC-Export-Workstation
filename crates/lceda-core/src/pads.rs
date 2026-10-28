//! PADS Logic / Layout 9.4+ ASCII 库（V9）：`.c` 原理图 decal、`.d` 封装、`.p` 器件类型。
//!
//! 规格：Mentor *PADS Parts Library ASCII Format*（`*PADS-LIBRARY-*-V9*`）。
//! 仅经典 PADS Logic / Layout 可导入；PADS Professional / Xpedition 不能当 Central Library。

use crate::error::Result;
use crate::ir::{FootprintIr, IrPad, IrPin, SymbolIr};
use crate::util::ensure_parent;
use std::collections::HashSet;
use std::path::Path;

mod layout;

const TS: &str = "2026.8.21.0.0.0";
const FONT: &str = "\"Default Font\"";
/// PADS Logic/Layout 拒绝线宽 0（报「图形宽度不正确 0」）。
const MIN_LINE_MIL: f64 = 5.0;
/// Explicit solid stroke for OPEN/CLOSED/CIRCLE (PADS 9.4+).
/// The legacy value 1 displayed as dashed in the recipient's Logic editor.
/// Do not apply this to COPCLS or keepouts: their last field has other meanings.
const SOLID_LINE_STYLE: i32 = -1;
// The user requested 7 point lettering. ASCII stores integer mil heights:
// round(7 pt * 1000 mil/in / 72 pt/in) = 97 mil (6.984 pt).
// Fit geometry to this height; never shrink text to resolve collisions.
const SCH_NAME_HEIGHT: f64 = 97.0;
const SCH_NUMBER_HEIGHT: f64 = 97.0;
const SCH_ATTRIBUTE_HEIGHT: f64 = 97.0;
const SCH_TEXT_WIDTH: f64 = 8.0;
const SCH_TEXT_GAP: f64 = 30.0;
/// Standard PADS Logic library pin decal, with a 200 mil shaft in local +X.
/// The last T field is a library name, not a numeric null/sentinel.
const SCH_PIN_DECAL: &str = "PIN";
const SCH_PIN_LENGTH: f64 = 200.0;

pub fn write_part_files(
    dir: &Path,
    base: &str,
    symbol: Option<&SymbolIr>,
    footprint: Option<&FootprintIr>,
) -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>, Option<std::path::PathBuf>)> {
    write_part_files_with_source(dir, base, symbol, footprint, None)
}

/// Preserve EasyEDA PART boundaries as PADS gates instead of flattening units.
pub fn write_part_files_with_source(
    dir: &Path,
    base: &str,
    symbol: Option<&SymbolIr>,
    footprint: Option<&FootprintIr>,
    source: Option<&serde_json::Value>,
) -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>, Option<std::path::PathBuf>)> {
    let units = match (symbol, source) {
        (Some(sym), Some(raw)) => source_units(sym, raw)?,
        _ => Vec::new(),
    };
    let unit_refs: Vec<_> = units.iter().map(|unit| &unit.symbol).collect();
    let ident = pads_ident(base, 40);
    let mut c = None;
    let mut d = None;
    let mut p = None;
    if let Some(sym) = symbol {
        let path = dir.join(format!("{ident}.c"));
        let text = if units.is_empty() { sch_library(&[sym]) } else {
            let decals: Vec<_> = units.iter().map(|unit| sch_decal_with_text(&unit.symbol, &unit.texts)).collect();
            format!("*PADS-LIBRARY-SCH-DECALS-V9*\n\n{}*END*\n", decals.join("\n"))
        };
        write_text(&path, &text)?;
        c = Some(path);
    }
    if let Some(fp) = footprint {
        let path = dir.join(format!("{ident}.d"));
        write_text(&path, &pcb_library(&[fp]))?;
        d = Some(path);
    }
    if symbol.is_some() || footprint.is_some() {
        let path = dir.join(format!("{ident}.p"));
        let text = if units.is_empty() { part_library(&[(symbol, footprint)]) }
            else { format!("*PADS-LIBRARY-PART-TYPES-V9*\n\n{}*END*\n", part_type_units(symbol, footprint, &unit_refs)) };
        write_text(&path, &text)?;
        p = Some(path);
    }
    Ok((c, d, p))
}

struct SourceUnit { symbol: SymbolIr, texts: Vec<SourceText> }
struct SourceText { x: f64, y: f64, rotation: f64, height: f64, value: String }

impl SourceUnit {
    fn new(mut symbol: SymbolIr, mut texts: Vec<SourceText>) -> Self {
        // Some libraries hide a pin's automatic name and place the same visible
        // text beside it. Include these labels in the common pin layout instead
        // of letting a source-font annotation cross the body or another label.
        texts.retain(|text| {
            let nearest = symbol.pins.iter_mut().filter(|pin| !pin.show_name
                && pin.name.eq_ignore_ascii_case(&text.value))
                .map(|pin| {
                    let distance = (mils(pin.x) - text.x).hypot(mils(pin.y) - text.y);
                    (pin, distance)
                }).filter(|(_, distance)| *distance <= 200.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((pin, _)) = nearest { pin.show_name = true; false } else { true }
        });
        Self { symbol, texts }
    }
}

fn source_texts(rows: &[serde_json::Value]) -> Vec<SourceText> {
    let heights: std::collections::HashMap<_, _> = rows.iter().filter(|row| row[0] == "FONTSTYLE")
        .filter_map(|row| Some((row[1].as_str()?, row[5].as_f64().unwrap_or(7.0) * 10.0))).collect();
    rows.iter().filter(|row| row[0] == "TEXT").filter_map(|row| Some(SourceText {
        x: row[2].as_f64()? * 10.0, y: row[3].as_f64()? * 10.0,
        rotation: row[4].as_f64().unwrap_or(0.0), value: row[5].as_str()?.to_string(),
        height: heights.get(row[6].as_str().unwrap_or("")).copied().unwrap_or(70.0),
    })).collect()
}

fn source_units(symbol: &SymbolIr, source: &serde_json::Value) -> Result<Vec<SourceUnit>> {
    let rows = crate::easyeda::parse_component_json(source)?;
    if rows.iter().filter(|row| row[0] == "PART").count() <= 1 {
        return Ok(vec![SourceUnit::new(symbol.clone(), source_texts(&rows))]);
    }
    let mut groups: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut common = Vec::new();
    for row in rows {
        if row[0] == "PART" { groups.push(common.clone()); }
        if let Some(group) = groups.last_mut() { group.push(row); }
        else { common.push(row); }
    }
    groups.into_iter().enumerate().map(|(i, rows)| {
        let data = rows.iter().map(serde_json::to_string).collect::<std::result::Result<Vec<_>, _>>()?.join("\n");
        let mut raw = source.clone();
        raw["result"]["dataStr"] = data.into();
        let suffix = format!("_{}", i + 1);
        let name = format!("{}{}", pads_ident(&symbol.name, 40 - suffix.len()), suffix);
        let mut unit = crate::ir::symbol_ir(&name, &symbol.description,
            crate::easyeda::parse_symbol(&raw)?, symbol.meta.clone());
        unit.designator = symbol.designator.clone();
        Ok(SourceUnit::new(unit, source_texts(&rows)))
    }).collect()
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
    sch_decal_with_text(symbol, &[])
}

fn sch_decal_with_text(symbol: &SymbolIr, extra_texts: &[SourceText]) -> String {
    let (arranged, scale) = layout::arrange_with_scale(symbol);
    let symbol = &arranged;
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
        if let Some(p) = pin_extension(pin) {
            pieces.push(p);
        }
    }

    let terms: Vec<String> = symbol.pins.iter().map(sch_terminal).collect();
    // Follow native V9 libraries' four editor slots: refdes, part type, and two
    // free attribute placeholders. These records deliberately use the compact
    // six-number form and native PART-TYPE spelling, matching vendor .c files.
    let labels = 4;
    // PADS part types require unique electrical pin names (OUT, OUT_2, ...).
    // Draw source names as decal text so repeated names stay visually identical,
    // and each source pin's name visibility is preserved.
    let mut names: Vec<String> = symbol.pins.iter().filter(|p| p.show_name)
        .map(sch_pin_name).collect();
    // Preserve source annotations such as op-amp + / - and supply labels.
    for text in extra_texts {
        names.push(format!("{} {} {} 0 {} 5 0 0 0 0 {FONT}\n{}\n",
            n(text.x * scale.x), n(text.y * scale.y), n(text.rotation), n(text.height),
            clean_attr(&text.value).to_ascii_uppercase()));
    }
    let txt = names.len();
    let vis = 8; // Hide automatic electrical pin names, keep pin numbers visible.
    let mut out = format!(
        "{name} 0 0 {SCH_NUMBER_HEIGHT} {SCH_TEXT_WIDTH} {SCH_NAME_HEIGHT} {SCH_TEXT_WIDTH} {labels} {pieces} {txt} {terms} {vis}\nTIMESTAMP {TS}\n{FONT}\n{FONT}\n",
        pieces = pieces.len(),
        terms = terms.len(),
    );
    let (ref_x, ref_y, type_x, type_y) = sch_label_xy(symbol);
    out.push_str(&format!("{ref_x} {ref_y} 0 0 {SCH_ATTRIBUTE_HEIGHT} {SCH_TEXT_WIDTH}\nREF-DES\n"));
    // Attribute justification uses a different encoding from terminal/free text.
    out.push_str(&format!("{type_x} {type_y} 0 2 {SCH_ATTRIBUTE_HEIGHT} {SCH_TEXT_WIDTH}\nPART-TYPE\n"));
    for slot in 1..=2 {
        let y = type_y - f64::from(slot) * (SCH_ATTRIBUTE_HEIGHT + 50.0);
        out.push_str(&format!("{type_x} {y} 0 2 {SCH_ATTRIBUTE_HEIGHT} {SCH_TEXT_WIDTH}\n*\n"));
    }
    for p in pieces {
        out.push_str(&p);
    }
    for name in names {
        out.push_str(&name);
    }
    for t in terms {
        out.push_str(&t);
    }
    out
}

fn sch_label_xy(symbol: &SymbolIr) -> (f64, f64, f64, f64) {
    let (mut left, mut top, mut bot) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY);
    let mut include = |x: f64, y: f64| {
        left = left.min(x);
        top = top.max(y);
        bot = bot.min(y);
    };
    for r in &symbol.rects {
        include(mils_snap(r.x1), mils_snap(r.y1));
        include(mils_snap(r.x2), mils_snap(r.y2));
    }
    for poly in &symbol.polys {
        for &(x, y) in poly { include(mils_snap(x), mils_snap(y)); }
    }
    for e in &symbol.ellipses {
        include(mils_snap(e.x - e.rx), mils_snap(e.y - e.ry));
        include(mils_snap(e.x + e.rx), mils_snap(e.y + e.ry));
    }
    for pin in &symbol.pins {
        include(mils_snap(pin.x), mils_snap(pin.y));
        let (x, y) = sch_pin_tip(pin);
        // Reserve room for the terminal number as well as the shaft.
        include(x, y + SCH_NUMBER_HEIGHT);
        include(x, y - SCH_NUMBER_HEIGHT);
    }
    if !left.is_finite() {
        left = 0.0;
        top = 200.0;
        bot = -200.0;
    }
    // Align attributes to the body when available, outside every graphic/pin.
    let body_left = symbol.rects.iter().map(|r| mils_snap(r.x1.min(r.x2)))
        .reduce(f64::min).unwrap_or(left);
    (body_left, snap10(top + 80.0), body_left, snap10(bot - 80.0))
}

fn sch_pin_tip(pin: &IrPin) -> (f64, f64) {
    let (dx, dy) = pin_outward(pin.rotation, sch_pin_length(pin));
    (snap10(mils_snap(pin.x) + dx), snap10(mils_snap(pin.y) + dy))
}

fn sch_pin_length(pin: &IrPin) -> f64 {
    mils_snap(pin.length).max(SCH_PIN_LENGTH)
}

fn pin_extension(pin: &IrPin) -> Option<String> {
    // PIN supplies the first 200 mil. Draw only the remaining length, otherwise
    // editing the decal would expose two overlapping shafts on every terminal.
    if sch_pin_length(pin) <= SCH_PIN_LENGTH { return None; }
    let (x, y) = sch_pin_tip(pin);
    let (dx, dy) = pin_outward(pin.rotation, SCH_PIN_LENGTH);
    let (bx, by) = (mils_snap(pin.x), mils_snap(pin.y));
    Some(format!(
        "OPEN 2 10 0 {SOLID_LINE_STYLE}\n{} {}\n{} {}\n",
        n(x - dx),
        n(y - dy),
        n(bx),
        n(by)
    ))
}

fn sch_terminal(pin: &IrPin) -> String {
    let (x, y) = sch_pin_tip(pin);
    let len = sch_pin_length(pin);
    let (dx, dy) = pin_outward(pin.rotation, 1.0);
    // Native PADS library exports use these terminal transforms for PIN:
    // left (0,0), right (0,2), bottom (90,0), top (90,4).
    // Number/name offsets are in pin-local coordinates, before that transform.
    let (rotation, mirror) = if dx < 0.0 { (0, 0) }
        else if dx > 0.0 { (0, 2) }
        else if dy < 0.0 { (90, 0) } else { (90, 4) };
    format!(
        "T{} {} {rotation} {mirror} {} 15 0 1 {} 0 0 16 {SCH_PIN_DECAL}\nP 0 0 0 0 0 0 0 0 192\n",
        n(x),
        n(y),
        n(snap10(len / 2.0)),
        n(len + SCH_TEXT_GAP)
    )
}

fn sch_pin_name(pin: &IrPin) -> String {
    let (dx, dy) = pin_outward(pin.rotation, 1.0);
    let x = mils_snap(pin.x) - dx * SCH_TEXT_GAP;
    let y = mils_snap(pin.y) - dy * SCH_TEXT_GAP;
    let rotation = if dy != 0.0 { 90 } else { 0 };
    let just = if dx + dy > 0.0 { 18 } else { 16 };
    format!("{} {} {rotation} 0 {SCH_NAME_HEIGHT} {SCH_TEXT_WIDTH} 0 {just} 0 0 {FONT}\n{}\n",
        n(x), n(y), clean_attr(&pin.name).to_ascii_uppercase())
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
    let units: Vec<_> = symbol.into_iter().collect();
    part_type_units(symbol, footprint, &units)
}

fn part_type_units(symbol: Option<&SymbolIr>, footprint: Option<&FootprintIr>, units: &[&SymbolIr]) -> String {
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
    let attrs = attr_count(symbol, footprint);
    let gates = units.len();
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
    for unit in units {
        let cae_name = pads_ident(&unit.name, 40);
        let pins = &unit.pins;
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
        "CLOSED 5 {} {layer} {SOLID_LINE_STYLE}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {}\n",
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
    let mut out = format!("OPEN {} {} {layer} {SOLID_LINE_STYLE}\n", pts.len(), n(width));
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
        "CLOSED {} {} {layer} {SOLID_LINE_STYLE}\n",
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
        "CIRCLE 2 {} {layer} {SOLID_LINE_STYLE}\n{} {}\n{} {}\n",
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

/// SymbolIr pins are anchored at the body; rotation points OUT toward the wire.
fn pin_outward(rot_deg: f64, len: f64) -> (f64, f64) {
    let a = ((rot_deg / 90.0).round() as i32).rem_euclid(4);
    match a {
        0 => (len, 0.0),
        1 => (0.0, len),
        2 => (-len, 0.0),
        _ => (0.0, -len),
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
            designator: "R?".into(),
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
                    rotation: 180.0,
                    pin_type: "PASSIVE".into(),
                    ..Default::default()
                },
                IrPin {
                    number: "2".into(),
                    name: "2".into(),
                    x: 2.54,
                    y: 0.0,
                    length: 2.54,
                    rotation: 0.0,
                    pin_type: "PASSIVE".into(),
                    ..Default::default()
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
            model: Default::default(),
        }
    }

    #[test]
    fn sch_has_header_and_terminals() {
        let text = sch_library(&[&sample_symbol()]);
        assert!(text.starts_with("*PADS-LIBRARY-SCH-DECALS-V9*"));
        assert!(text.contains("REF-DES"));
        assert!(text.contains("PART-TYPE"));
        assert!(text.contains("CLOSED"));
        assert!(text.contains('\n') && text.contains("T"));
        assert!(text.contains("*END*"));
    }

    #[test]
    fn sch_editor_attribute_slots_have_bounded_horizontal_full_size_text() {
        let sym = sample_symbol();
        let text = sch_library(&[&sym]);
        let lines: Vec<_> = text.lines().collect();
        let header: Vec<_> = lines[2].split_whitespace().collect();
        assert_eq!((header[3], header[5], header[7]), ("97", "97", "4"));
        let expected = ["REF-DES", "PART-TYPE", "*", "*"];
        for (slot, field) in expected.iter().enumerate() {
            let fields: Vec<f64> = lines[6 + 2 * slot].split_whitespace()
                .map(|s| s.parse().expect("native six-number attribute record")).collect();
            assert_eq!(fields.len(), 6);
            assert_eq!(fields[2], 0.0, "no diagonal rotation on entering the graphics editor");
            assert_eq!(fields[4], 97.0, "keep every editor slot at the requested 7 point height");
            assert!(fields[0].abs() < 1000.0 && fields[1].abs() < 1000.0);
            assert_eq!(&lines[7 + 2 * slot], field);
        }
        assert!(lines[14].starts_with("CLOSED "), "graphics start after all four editor slots");
    }

    #[test]
    fn sch_left_native_pin_connects_tip_to_ir_body_anchor_without_duplicate_shaft() {
        let text = sch_library(&[&sample_symbol()]);
        assert!(!text.contains("OPEN "), "native PIN supplies the complete shaft");
        let term = text
            .lines()
            .find(|l| l.starts_with("T-"))
            .unwrap_or("");
        let cols: Vec<&str> = term.split_whitespace().collect();
        assert!(cols.len() >= 6, "terminal: {term}");
        let pnx: f64 = cols[4].parse().unwrap();
        let pny: f64 = cols[5].parse().unwrap();
        assert_eq!(cols.last(), Some(&"PIN"));
        let (arranged, _) = layout::arrange_with_scale(&sample_symbol());
        assert_eq!(cols[0][1..].parse::<f64>().unwrap(), mils_snap(arranged.pins[0].x) - 200.0);
        assert!(pnx > 0.0 && pnx < 200.0 && pny > 0.0,
            "number should sit above the shaft, between tip and body: {term}");
    }

    #[test]
    fn sch_all_four_sides_have_wire_tips_outside_and_names_inside() {
        for (rotation, expected_tip, expected_name) in [
            (0.0, (200.0, 0.0), (-30.0, 0.0)),
            (90.0, (0.0, 200.0), (0.0, -30.0)),
            (180.0, (-200.0, 0.0), (30.0, 0.0)),
            (270.0, (0.0, -200.0), (0.0, 30.0)),
        ] {
            let pin = IrPin { rotation, name: "TEST".into(), ..Default::default() };
            assert_eq!(sch_pin_tip(&pin), expected_tip);
            let name = sch_pin_name(&pin);
            let cols: Vec<_> = name.lines().next().unwrap().split_whitespace().collect();
            assert_eq!((cols[0].parse::<f64>().unwrap(), cols[1].parse::<f64>().unwrap()), expected_name);
            assert_eq!(cols[2], if rotation == 90.0 || rotation == 270.0 { "90" } else { "0" });
            assert_eq!(cols[7], if rotation < 180.0 { "18" } else { "16" });
        }
    }

    #[test]
    fn native_pin_transforms_join_body_and_long_extensions_do_not_overlap() {
        for rotation in [0.0, 90.0, 180.0, 270.0] {
            for length in [1.27, 2.54, 5.08, 7.62] {
                let pin = IrPin { x: 2.54, y: -5.08, rotation, length, ..Default::default() };
                let terminal = sch_terminal(&pin);
                let t: Vec<_> = terminal.lines().next().unwrap()[1..].split_whitespace().collect();
                assert_eq!(t[12], "PIN", "a numeric 0 requests a nonexistent decal");
                // Decode PIN's native endpoint using the four PADS transforms.
                let inward = match (t[2], t[3]) {
                    ("0", "0") => (200.0, 0.0), ("0", "2") => (-200.0, 0.0),
                    ("90", "0") => (0.0, 200.0), ("90", "4") => (0.0, -200.0),
                    _ => panic!("unsupported native transform"),
                };
                let end = (t[0].parse::<f64>().unwrap() + inward.0,
                    t[1].parse::<f64>().unwrap() + inward.1);
                if let Some(extension) = pin_extension(&pin) {
                    let points: Vec<Vec<f64>> = extension.lines().skip(1)
                        .map(|l| l.split_whitespace().map(|s| s.parse().unwrap()).collect()).collect();
                    assert_eq!(points[0], [end.0, end.1]);
                    assert_eq!(points[1], [100.0, -200.0]);
                    assert!((points[0][0] - points[1][0]).hypot(points[0][1] - points[1][1]) > 0.0);
                } else { assert_eq!(end, (100.0, -200.0)); }
            }
        }
    }

    #[test]
    fn bts7008_real_source_keeps_pin_mapping_and_clear_text() {
        let raw = serde_json::from_str(include_str!("../tests/fixtures/BTS7008-1EPP_symbol_easyeda.json")).unwrap();
        let source = crate::easyeda::parse_symbol(&raw).unwrap();
        let sym = crate::ir::symbol_ir("BTS7008-1EPP", "", source, PartMeta::default());
        let c = sch_library(&[&sym]);
        let p = part_library(&[(Some(&sym), None)]);
        assert_eq!(sym.pins.len(), 15);
        // Native PIN shafts still end at the enlarged body, with external tips.
        let (arranged, _) = layout::arrange_with_scale(&sym);
        let terminals: Vec<_> = c.lines().filter(|l| l.starts_with('T') && !l.starts_with("TIMESTAMP"))
            .map(|l| l[1..].split_whitespace().collect::<Vec<_>>()).collect();
        for (pin, t) in arranged.pins.iter().zip(&terminals) {
            assert_eq!((t[0].parse().unwrap(), t[1].parse().unwrap()), sch_pin_tip(pin));
            assert_eq!(t[12], "PIN");
        }
        // No visible suffixes, no doubled names from the part type and decal.
        assert_eq!(c.lines().filter(|line| *line == "OUT").count(), 6);
        assert_eq!(c.lines().filter(|line| *line == "N.C.").count(), 4);
        assert!(c.lines().nth(2).unwrap().ends_with("15 15 8"));
        let header: Vec<_> = c.lines().nth(2).unwrap().split_whitespace().collect();
        assert_eq!((header[3], header[5]), ("97", "97"));
        assert!(c.contains("0 2 97 8\nPART-TYPE"));
        let actual: Vec<_> = p.lines().skip_while(|line| !line.starts_with("GATE "))
            .skip(2).take(15).map(|line| line.split_whitespace().next().unwrap()).collect();
        let expected: Vec<_> = sym.pins.iter().map(|pin| pin.number.as_str()).collect();
        assert_eq!(actual, expected, "decal terminal order must match the part type");
        assert_eq!(sym.pins.iter().find(|pin| pin.number == "11").unwrap().name, "n.c.");
        assert_eq!(sym.pins.iter().find(|pin| pin.number == "15").unwrap().name, "EP");
    }

    #[test]
    fn sch_attributes_clear_nonrectangular_graphics_and_vertical_pins() {
        let mut sym = sample_symbol();
        sym.rects.clear();
        sym.polys = vec![vec![(-2.54, -25.4), (2.54, 25.4)]];
        let (_, top, _, bottom) = sch_label_xy(&sym);
        assert!(top > 1000.0 && bottom < -1000.0);
        sym.pins.push(IrPin { y: -25.4, length: 5.08, rotation: 270.0, ..Default::default() });
        assert!(sch_label_xy(&sym).3 < -1200.0);
    }

    #[test]
    fn multiple_source_parts_become_separate_gates_with_original_pin_numbers() {
        let raw = serde_json::from_str(include_str!("../tests/fixtures/LM358DR_symbol_easyeda.json")).unwrap();
        let symbol = crate::ir::symbol_ir("any-dual-unit", "",
            crate::easyeda::parse_symbol(&raw).unwrap(), PartMeta::default());
        let units = source_units(&symbol, &raw).unwrap();
        assert_eq!(units.len(), 2);
        let numbers = |unit: &SourceUnit| unit.symbol.pins.iter().map(|p| p.number.clone()).collect::<Vec<_>>();
        assert_eq!(numbers(&units[0]), ["2", "3", "1", "8", "4"]);
        assert_eq!(numbers(&units[1]), ["7", "6", "5"]);
        let refs: Vec<_> = units.iter().map(|unit| &unit.symbol).collect();
        let part = part_type_units(Some(&symbol), None, &refs);
        assert!(part.contains("GATE 1 5 0\nany-dual-unit_1\n"));
        assert!(part.contains("GATE 1 3 0\nany-dual-unit_2\n"));
        assert_eq!(part.lines().filter(|line| line.starts_with("GATE ")).count(), 2);
        let c = sch_decal_with_text(&units[0].symbol, &units[0].texts);
        assert!(c.contains("\n+\n") && c.contains("\n-\n") && c.contains("\n+V\n"));
        // Two units must no longer trigger runaway scaling of coincident pins.
        let scale = layout::arrange_with_scale(&units[0].symbol).1;
        assert!(scale.x < 8.0 && scale.y < 8.0);
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
        assert!(text.contains("CLOSED 4 5 26 -1"));
    }

    #[test]
    fn all_graphic_strokes_use_explicit_solid_without_changing_copper_flags() {
        let primitives = [
            closed_rect(-100.0, -100.0, 100.0, 100.0, 10.0, 0),
            open_poly(&[(0.0, 0.0), (2.54, 0.0)], true, 10.0, 0).unwrap(),
            closed_poly(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], 5.0, 26).unwrap(),
            circle_piece(0.0, 0.0, 50.0, 10.0, 0),
            pin_extension(&IrPin { length: 7.62, ..Default::default() }).unwrap(),
        ];
        for piece in primitives {
            assert_eq!(piece.lines().next().unwrap().split_whitespace().nth(4), Some("-1"));
        }
        let copper = copcls(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], 1).unwrap();
        assert_eq!(copper.lines().next().unwrap().split_whitespace().nth(4), Some("1"));
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
                ..Default::default()
            });
            s.pins[0].name = "GND".into();
            s
        };
        let text = part_library(&[(Some(&s2), Some(&sample_fp()))]);
        assert!(text.contains("GND_2"), "duplicate pin names must be uniquified: {text}");
    }
}
