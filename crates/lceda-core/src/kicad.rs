//! KiCad 6+ 文本库：`.kicad_sym` 与 `.kicad_mod`。

use crate::error::Result;
use crate::ir::{FootprintIr, IrPad, SymbolIr};
use crate::util::{ensure_parent, sanitize_filename};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

pub fn write_symbol_lib(path: &Path, symbol: &SymbolIr) -> Result<()> {
    write_symbol_lib_many(path, std::slice::from_ref(symbol))
}

pub fn write_symbol_lib_many(path: &Path, symbols: &[SymbolIr]) -> Result<()> {
    ensure_parent(path)?;
    fs::write(path, symbol_lib_text_many(symbols))?;
    Ok(())
}

pub fn write_footprint_mod(path: &Path, fp: &FootprintIr, step_rel: Option<&str>) -> Result<()> {
    ensure_parent(path)?;
    fs::write(path, footprint_mod_text(fp, step_rel))?;
    Ok(())
}

pub fn pretty_dir(out_dir: &Path, base: &str) -> std::path::PathBuf {
    out_dir.join(format!("{}.pretty", sanitize_filename(base)))
}

fn symbol_lib_text_many(symbols: &[SymbolIr]) -> String {
    let mut out = String::new();
    out.push_str("(kicad_symbol_lib\n");
    out.push_str("  (version 20241209)\n");
    out.push_str("  (generator \"lceda-assistant\")\n");
    for symbol in symbols {
        out.push_str(&symbol_entry(symbol));
    }
    out.push_str(")\n");
    out
}

fn symbol_entry(symbol: &SymbolIr) -> String {
    let name = ident(&symbol.name);
    let mut out = String::new();
    out.push_str(&format!("  (symbol {name}\n"));
    out.push_str("    (exclude_from_sim no)\n    (in_bom yes)\n    (on_board yes)\n");
    push_prop(&mut out, "Reference", &guess_ref(symbol), 0.0, 5.08);
    push_prop(&mut out, "Value", &symbol.name, 0.0, -5.08);
    let fp = if symbol.meta.footprint_lib.is_empty() {
        String::new()
    } else {
        symbol.meta.footprint_lib.clone()
    };
    push_prop(&mut out, "Footprint", &fp, 0.0, -7.62);
    push_prop(&mut out, "Datasheet", &symbol.meta.datasheet, 0.0, -10.16);
    if !symbol.meta.lcsc.is_empty() {
        push_prop(&mut out, "LCSC", &symbol.meta.lcsc, 0.0, -12.7);
    }
    if !symbol.meta.manufacturer.is_empty() {
        push_prop(&mut out, "Manufacturer", &symbol.meta.manufacturer, 0.0, -15.24);
    }
    if !symbol.meta.mpn.is_empty() {
        push_prop(&mut out, "MPN", &symbol.meta.mpn, 0.0, -17.78);
    }
    out.push_str(&format!("    (symbol \"{}_0_1\"\n", unescape_ident(&symbol.name)));

    for r in &symbol.rects {
        out.push_str(&format!(
            "      (rectangle (start {} {}) (end {} {})\n        (stroke (width 0.254) (type default))\n        (fill (type none))\n      )\n",
            n(r.x1),
            n(r.y1),
            n(r.x2),
            n(r.y2)
        ));
    }
    for poly in &symbol.polys {
        if poly.len() < 2 {
            continue;
        }
        for win in poly.windows(2) {
            out.push_str(&format!(
                "      (polyline (pts (xy {} {}) (xy {} {}))\n        (stroke (width 0.254) (type default))\n        (fill (type none))\n      )\n",
                n(win[0].0),
                n(win[0].1),
                n(win[1].0),
                n(win[1].1)
            ));
        }
    }
    for e in &symbol.ellipses {
        if (e.rx - e.ry).abs() < 1e-6 {
            out.push_str(&format!(
                "      (circle (center {} {}) (radius {})\n        (stroke (width 0.254) (type default))\n        (fill (type none))\n      )\n",
                n(e.x),
                n(e.y),
                n(e.rx)
            ));
        } else {
            out.push_str(&format!(
                "      (arc (start {} {}) (mid {} {}) (end {} {})\n        (stroke (width 0.254) (type default))\n        (fill (type none))\n      )\n",
                n(e.x + e.rx),
                n(e.y),
                n(e.x),
                n(e.y + e.ry),
                n(e.x - e.rx),
                n(e.y)
            ));
        }
    }
    for pin in &symbol.pins {
        let rot = snap_angle(pin.rotation);
        let etype = kicad_etype(&pin.pin_type);
        out.push_str(&format!(
            "      (pin {etype} line (at {} {} {rot}) (length {})\n        (name {} (effects (font (size 1.27 1.27))))\n        (number {} (effects (font (size 1.27 1.27))))\n      )\n",
            n(pin.x),
            n(pin.y),
            n(pin.length.max(2.54)),
            quoted(&pin.name),
            quoted(&pin.number)
        ));
    }
    out.push_str("    )\n  )\n");
    out
}

fn footprint_mod_text(fp: &FootprintIr, step_rel: Option<&str>) -> String {
    let name = unescape_ident(&fp.name);
    let smd = fp.pads.iter().all(|p| p.hole <= 1e-6);
    let mut out = String::new();
    out.push_str(&format!("(footprint {}\n", quoted(&name)));
    out.push_str("  (version 20241209)\n");
    out.push_str("  (generator \"lceda-assistant\")\n");
    out.push_str("  (layer \"F.Cu\")\n");
    if !fp.description.is_empty() {
        out.push_str(&format!(
            "  (descr {})\n",
            quoted(&fp.description.replace(['\r', '\n'], "; "))
        ));
    }
    if !fp.meta.lcsc.is_empty() {
        out.push_str(&format!("  (tags {})\n", quoted(&fp.meta.lcsc)));
    }
    out.push_str(if smd {
        "  (attr smd)\n"
    } else {
        "  (attr through_hole)\n"
    });
    out.push_str("  (fp_text reference \"REF**\" (at 0 -1.6 unlocked) (layer \"F.SilkS\")\n    (effects (font (size 1 1) (thickness 0.15)))\n  )\n");
    out.push_str(&format!(
        "  (fp_text value {} (at 0 1.6 unlocked) (layer \"F.Fab\")\n    (effects (font (size 1 1) (thickness 0.15)))\n  )\n",
        quoted(&name)
    ));
    if !fp.meta.lcsc.is_empty() {
        out.push_str(&format!("  (property \"LCSC\" {})\n", quoted(&fp.meta.lcsc)));
    }
    if !fp.meta.manufacturer.is_empty() {
        out.push_str(&format!(
            "  (property \"Manufacturer\" {})\n",
            quoted(&fp.meta.manufacturer)
        ));
    }

    for pad in &fp.pads {
        write_pad(&mut out, pad);
    }
    for t in &fp.tracks {
        let layer = graphic_layer(t.layer);
        let w = t.width.max(0.05);
        for win in t.points.windows(2) {
            out.push_str(&format!(
                "  (fp_line (start {} {}) (end {} {}) (stroke (width {}) (type default)) (layer {layer}))\n",
                n(win[0].0),
                n(win[0].1),
                n(win[1].0),
                n(win[1].1),
                n(w)
            ));
        }
    }
    for c in &fp.circles {
        let layer = graphic_layer(c.layer);
        out.push_str(&format!(
            "  (fp_circle (center {} {}) (end {} {}) (stroke (width {}) (type default)) (fill none) (layer {layer}))\n",
            n(c.x),
            n(c.y),
            n(c.x + c.radius.abs()),
            n(c.y),
            n(c.width.max(0.05))
        ));
    }
    for a in &fp.arcs {
        let layer = graphic_layer(a.layer);
        let (sx, sy) = polar(a.x, a.y, a.radius, a.start);
        let mid_ang = a.start + crate::easyeda::normalize_angle(a.end - a.start) / 2.0;
        let (mx, my) = polar(a.x, a.y, a.radius, mid_ang);
        let (ex, ey) = polar(a.x, a.y, a.radius, a.end);
        out.push_str(&format!(
            "  (fp_arc (start {} {}) (mid {} {}) (end {} {}) (stroke (width {}) (type default)) (layer {layer}))\n",
            n(sx),
            n(sy),
            n(mx),
            n(my),
            n(ex),
            n(ey),
            n(a.width.max(0.05))
        ));
    }
    for r in &fp.regions {
        if r.points.len() < 3 {
            continue;
        }
        let layer = graphic_layer(r.layer);
        out.push_str("  (fp_poly (pts");
        for (x, y) in &r.points {
            out.push_str(&format!(" (xy {} {})", n(*x), n(*y)));
        }
        out.push_str(&format!(
            ") (stroke (width 0) (type default)) (fill solid) (layer {layer}))\n"
        ));
    }
    if let Some(rel) = step_rel {
        let m = &fp.model;
        out.push_str(&format!(
            "  (model {}\n    (offset (xyz {} {} {}))\n    (scale (xyz 1 1 1))\n    (rotate (xyz {} {} {}))\n  )\n",
            quoted(rel),
            n(m.off_x),
            n(m.off_y),
            n(m.off_z),
            n(m.rot_x),
            n(m.rot_y),
            n(m.rot_z),
        ));
    }
    out.push_str(")\n");
    out
}

fn write_pad(out: &mut String, pad: &IrPad) {
    if pad.is_custom_poly() {
        if let Some(pts) = &pad.polygon {
            write_custom_pad(out, pad, pts);
            return;
        }
    }
    let thru = pad.hole > 1e-6;
    let kind = if thru { "thru_hole" } else { "smd" };
    let shape = pad_shape(&pad.shape, pad.width, pad.height);
    let layers = if thru {
        "\"*.Cu\" \"*.Mask\""
    } else if pad.layer == 2 {
        "\"B.Cu\" \"B.Paste\" \"B.Mask\""
    } else {
        "\"F.Cu\" \"F.Paste\" \"F.Mask\""
    };
    let rot = snap_angle(pad.rotation);
    write!(
        out,
        "  (pad {} {kind} {shape} (at {} {} {rot}) (size {} {}) (layers {layers})",
        quoted(&pad.designator),
        n(pad.x),
        n(pad.y),
        n(pad.width.max(0.1)),
        n(pad.height.max(0.1))
    )
    .unwrap();
    if thru {
        if pad.hole_slot > 1e-6 {
            write!(
                out,
                " (drill oval {} {})",
                n(pad.hole.max(0.1)),
                n(pad.hole_slot.max(0.1))
            )
            .unwrap();
        } else {
            write!(out, " (drill {})", n(pad.hole.max(0.1))).unwrap();
        }
    }
    if shape == "roundrect" {
        out.push_str(" (roundrect_rratio 0.25)");
    }
    out.push_str(")\n");
}

fn write_custom_pad(out: &mut String, pad: &IrPad, pts: &[(f64, f64)]) {
    let layers = if pad.layer == 2 {
        "\"B.Cu\" \"B.Paste\" \"B.Mask\""
    } else {
        "\"F.Cu\" \"F.Paste\" \"F.Mask\""
    };
    write!(
        out,
        "  (pad {} smd custom (at {} {} {}) (size {} {}) (layers {layers})\n",
        quoted(&pad.designator),
        n(pad.x),
        n(pad.y),
        snap_angle(pad.rotation),
        n(pad.width.max(0.1)),
        n(pad.height.max(0.1))
    )
    .unwrap();
    out.push_str("    (options (clearance outline) (anchor circle))\n    (primitives\n      (gr_poly\n        (pts");
    for &(x, y) in pts {
        out.push_str(&format!(" (xy {} {})", n(x - pad.x), n(y - pad.y)));
    }
    out.push_str(")\n        (width 0) (fill yes)\n      )\n    )\n  )\n");
}

fn pad_shape(shape: &str, w: f64, h: f64) -> &'static str {
    let s = shape.to_ascii_uppercase();
    if s.contains("CIRC") || ((w - h).abs() < 1e-4 && !s.contains("RECT") && !s.contains("POLY")) {
        "circle"
    } else if s.contains("OVAL") {
        "oval"
    } else if s.contains("ROUND") || s.contains("RECT") {
        "roundrect"
    } else {
        "roundrect"
    }
}

fn graphic_layer(easy: i32) -> &'static str {
    match easy {
        1 => "F.Cu",
        2 => "B.Cu",
        3 | 49 => "F.SilkS",
        4 => "B.SilkS",
        5 => "F.Mask",
        6 => "B.Mask",
        7 => "F.Paste",
        8 => "B.Paste",
        11 | 12 | 13 | 48 => "F.Fab",
        _ => "F.SilkS",
    }
}

fn kicad_etype(src: &str) -> &'static str {
    match src.to_ascii_uppercase().as_str() {
        "IN" | "INPUT" => "input",
        "OUT" | "OUTPUT" => "output",
        "I/O" | "IO" | "BIDIR" | "BIDIRECTIONAL" => "bidirectional",
        "PWR" | "POWER" | "POWER_IN" => "power_in",
        "POWER_OUT" => "power_out",
        "PASSIVE" => "passive",
        "NC" | "NO_CONNECT" => "no_connect",
        _ => "unspecified",
    }
}

fn guess_ref(symbol: &SymbolIr) -> String {
    let n = symbol.name.to_ascii_uppercase();
    if crate::models::looks_like_lcsc_token(&n) {
        return "U".into();
    }
    match n.chars().next() {
        Some('R') => "R".into(),
        Some('C') => "C".into(),
        Some('L') => "L".into(),
        Some('D') => "D".into(),
        Some('Q') => "Q".into(),
        Some('F') => "F".into(),
        Some('Y') => "Y".into(),
        _ => "U".into(),
    }
}

fn polar(cx: f64, cy: f64, r: f64, deg: f64) -> (f64, f64) {
    let a = deg.to_radians();
    (cx + r.abs() * a.cos(), cy + r.abs() * a.sin())
}

fn snap_angle(deg: f64) -> i32 {
    let a = crate::easyeda::normalize_angle(deg);
    ((a / 90.0).round() as i32).rem_euclid(4) * 90
}

fn n(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn ident(s: &str) -> String {
    quoted(&unescape_ident(s))
}

fn unescape_ident(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "part".into()
    } else {
        cleaned
    }
}

fn push_prop(out: &mut String, key: &str, value: &str, x: f64, y: f64) {
    out.push_str(&format!(
        "    (property {k} {v} (at {x} {y} 0)\n      (effects (font (size 1.27 1.27)) hide)\n    )\n",
        k = quoted(key),
        v = quoted(value),
        x = n(x),
        y = n(y)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{FootprintIr, IrPad, IrPin, IrRect, PartMeta, SymbolIr};

    #[test]
    fn symbol_lib_contains_pins_and_lcsc() {
        let symbol = SymbolIr {
            name: "RES".into(),
            description: "test".into(),
            meta: PartMeta {
                lcsc: "C2040".into(),
                mpn: "RES".into(),
                ..Default::default()
            },
            pins: vec![IrPin {
                number: "1".into(),
                name: "1".into(),
                x: 0.0,
                y: 2.54,
                length: 2.54,
                rotation: 270.0,
                pin_type: "PASSIVE".into(),
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
        let text = symbol_lib_text_many(std::slice::from_ref(&symbol));
        assert!(text.contains("(kicad_symbol_lib"));
        assert!(text.contains("LCSC"));
        assert!(text.contains("C2040"));
        assert!(text.contains("passive"));
    }

    #[test]
    fn merged_symbol_lib_contains_two_entries() {
        let a = SymbolIr {
            name: "AAA".into(),
            description: String::new(),
            meta: PartMeta::default(),
            pins: vec![],
            rects: vec![],
            polys: vec![],
            ellipses: vec![],
        };
        let mut b = a.clone();
        b.name = "BBB".into();
        let text = symbol_lib_text_many(&[a, b]);
        assert!(text.contains("(symbol \"AAA\""));
        assert!(text.contains("(symbol \"BBB\""));
        assert_eq!(text.matches("(kicad_symbol_lib").count(), 1);
    }

    #[test]
    fn footprint_mod_contains_pads() {
        let fp = FootprintIr {
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
        };
        let text = footprint_mod_text(&fp, None);
        assert!(text.contains("(footprint"));
        assert!(text.contains("smd"));
        assert!(text.contains("\"1\""));
        assert!(!text.contains("(model "));
    }

    #[test]
    fn footprint_mod_model_block_matches_npnp_sexp() {
        let fp = FootprintIr {
            name: "R0402".into(),
            description: "0402".into(),
            meta: PartMeta::default(),
            pads: vec![],
            tracks: vec![],
            circles: vec![],
            arcs: vec![],
            regions: vec![],
            model: Default::default(),
        };
        let text = footprint_mod_text(&fp, Some("../R0402.3dshapes/R0402.step"));
        assert!(text.contains("  (model \"../R0402.3dshapes/R0402.step\"\n"));
        assert!(text.contains("    (offset (xyz 0 0 0))\n"));
        assert!(text.contains("    (scale (xyz 1 1 1))\n"));
        assert!(text.contains("    (rotate (xyz 0 0 0))\n"));
        let mut rotated = fp;
        rotated.model.rot_z = 90.0;
        rotated.model.off_y = 0.1;
        let text = footprint_mod_text(&rotated, Some("../R0402.3dshapes/R0402.step"));
        assert!(text.contains("    (offset (xyz 0 0.1 0))\n"));
        assert!(text.contains("    (rotate (xyz 0 0 90))\n"));
    }
}
