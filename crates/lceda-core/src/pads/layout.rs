//! PADS-only geometry adaptation. No device names or pin-number special cases.
//! Keep source topology, pin order and orientation; expand cramped geometry while
//! retaining readable text sizes and external pin lengths on the 10 mil grid.
use super::{clean_attr, mils_snap, pin_outward, sch_pin_tip, sch_pin_length, snap10, SCH_NAME_HEIGHT,
    SCH_NUMBER_HEIGHT, SCH_TEXT_GAP};
use crate::ir::{IrPin, SymbolIr};

const CLEARANCE: f64 = 20.0;

#[derive(Clone, Copy, Debug)]
struct Bounds { x0: f64, y0: f64, x1: f64, y1: f64 }

#[derive(Clone, Copy, Debug)]
pub(super) struct Scale { pub x: f64, pub y: f64 }

impl Bounds {
    fn overlaps(self, other: Self) -> bool {
        self.x0 < other.x1 + CLEARANCE && self.x1 + CLEARANCE > other.x0
            && self.y0 < other.y1 + CLEARANCE && self.y1 + CLEARANCE > other.y0
    }
}

// Conservative cap-height-based advances, including room for Default Font
// variations. This is deliberately larger than the preview font's glyph bounds.
fn text_width(text: &str, height: f64) -> f64 {
    clean_attr(text).to_ascii_uppercase().chars().map(|c| match c {
        'W' => 1.5, 'M' | '@' => 1.4,
        'I' | '1' | '.' | ',' | ':' | ';' | '!' | '|' => 0.75,
        _ => 1.15,
    }).sum::<f64>() * height
}

fn name_bounds(pin: &IrPin) -> Bounds {
    let (dx, dy) = pin_outward(pin.rotation, 1.0);
    let x = mils_snap(pin.x) - dx * SCH_TEXT_GAP;
    let y = mils_snap(pin.y) - dy * SCH_TEXT_GAP;
    let w = text_width(&pin.name, SCH_NAME_HEIGHT);
    // Font height is nominal: underscores and glyph overshoots can extend well
    // beyond the capital height. Reserve their full vertical ink bounds too.
    let h = SCH_NAME_HEIGHT * 1.4 / 2.0;
    if dx < 0.0 { Bounds { x0: x, x1: x + w, y0: y - h, y1: y + h } }
    else if dx > 0.0 { Bounds { x0: x - w, x1: x, y0: y - h, y1: y + h } }
    else if dy > 0.0 { Bounds { x0: x - h, x1: x + h, y0: y - w, y1: y } }
    else { Bounds { x0: x - h, x1: x + h, y0: y, y1: y + w } }
}

fn number_bounds(pin: &IrPin) -> Bounds {
    let (x, y) = sch_pin_tip(pin);
    let (dx, dy) = pin_outward(pin.rotation, sch_pin_length(pin) / 2.0);
    let (x, y) = (x - snap10(dx), y - snap10(dy));
    let w = text_width(&pin.number, SCH_NUMBER_HEIGHT) / 2.0;
    if dy.abs() > 0.0 {
        Bounds { x0: x - 15.0 - SCH_NUMBER_HEIGHT, x1: x - 15.0, y0: y - w, y1: y + w }
    } else {
        Bounds { x0: x - w, x1: x + w, y0: y + 15.0, y1: y + 15.0 + SCH_NUMBER_HEIGHT }
    }
}

fn crowded_axes(symbol: &SymbolIr) -> (bool, bool) {
    let mut boxes = Vec::new();
    let (mut grow_x, mut grow_y) = (false, false);
    for pin in &symbol.pins {
        let anchor = (mils_snap(pin.x), mils_snap(pin.y));
        if pin.show_name && !pin.name.is_empty() {
            let b = name_bounds(pin);
            boxes.push((b, anchor));
            let (dx, dy) = pin_outward(pin.rotation, 1.0);
            // Check text depth inside a rectangular body attached to this pin.
            // Nonrectangular symbols retain their original topology as well.
            for r in &symbol.rects {
                let (x0, x1) = (mils_snap(r.x1.min(r.x2)), mils_snap(r.x1.max(r.x2)));
                let (y0, y1) = (mils_snap(r.y1.min(r.y2)), mils_snap(r.y1.max(r.y2)));
                let (x, y) = (mils_snap(pin.x), mils_snap(pin.y));
                let on_side = (dx < 0.0 && (x - x0).abs() < 1.0)
                    || (dx > 0.0 && (x - x1).abs() < 1.0);
                let on_end = (dy < 0.0 && (y - y0).abs() < 1.0)
                    || (dy > 0.0 && (y - y1).abs() < 1.0);
                if on_side && y >= y0 && y <= y1 && (b.x0 < x0 + CLEARANCE || b.x1 > x1 - CLEARANCE) {
                    grow_x = true;
                }
                if on_end && x >= x0 && x <= x1 && (b.y0 < y0 + CLEARANCE || b.y1 > y1 - CLEARANCE) {
                    grow_y = true;
                }
            }
        }
        if pin.show_number { boxes.push((number_bounds(pin), anchor)); }
    }
    for (i, (a, (ax, ay))) in boxes.iter().enumerate() {
        for (b, (bx, by)) in &boxes[i + 1..] {
            if !a.overlaps(*b) { continue; }
            // Expand the axis that separates these anchors with less growth.
            // Long opposite names widen the body without stretching every row;
            // names on the same side increase the row/column pitch instead.
            let growth = |d: f64, positive: f64, negative: f64| {
                if d.abs() < 1.0 { f64::INFINITY }
                else { (if d > 0.0 { positive } else { negative }) / d.abs() }
            };
            let x = growth(bx - ax, a.x1 - b.x0 + CLEARANCE, b.x1 - a.x0 + CLEARANCE);
            let y = growth(by - ay, a.y1 - b.y0 + CLEARANCE, b.y1 - a.y0 + CLEARANCE);
            if x <= y { grow_x = true; } else { grow_y = true; }
        }
    }
    (grow_x, grow_y)
}

pub(super) fn arrange_with_scale(source: &SymbolIr) -> (SymbolIr, Scale) {
    let mut candidate = source.clone();
    // Long alphanumeric numbers also need enough shaft space outside the body.
    for pin in &mut candidate.pins {
        let required = text_width(&pin.number, SCH_NUMBER_HEIGHT) + 10.0;
        if pin.show_number {
            pin.length = pin.length.max((required / 10.0).ceil() * 10.0 * 0.0254);
        }
    }
    let template = candidate.clone();
    let mut scale = Scale { x: 1.0, y: 1.0 };
    // Preserve the aspect ratio of nonrectangular symbols (op-amp triangles,
    // transistor artwork, etc.). Rectangular IC bodies can grow independently.
    let independent_axes = !source.rects.is_empty() && source.polys.is_empty();
    // A bounded pass avoids runaway growth for intentionally stacked terminals.
    // Ordinary separated pins converge by growing the graphic around the origin.
    loop {
        let (mut grow_x, mut grow_y) = crowded_axes(&candidate);
        if !grow_x && !grow_y { break; }
        if !independent_axes { grow_x = true; grow_y = true; }
        let next = Scale {
            x: if grow_x { (scale.x * 1.25).min(32.0) } else { scale.x },
            y: if grow_y { (scale.y * 1.25).min(32.0) } else { scale.y },
        };
        if next.x == scale.x && next.y == scale.y { break; }
        scale = next;
        candidate = template.clone();
        let coordinate = |value: f64, factor: f64| snap10(value * factor / 0.0254) * 0.0254;
        for pin in &mut candidate.pins { pin.x = coordinate(pin.x, scale.x); pin.y = coordinate(pin.y, scale.y); }
        for r in &mut candidate.rects {
            r.x1 = coordinate(r.x1, scale.x); r.x2 = coordinate(r.x2, scale.x);
            r.y1 = coordinate(r.y1, scale.y); r.y2 = coordinate(r.y2, scale.y);
        }
        for poly in &mut candidate.polys {
            for (x, y) in poly { *x = coordinate(*x, scale.x); *y = coordinate(*y, scale.y); }
        }
        for e in &mut candidate.ellipses {
            e.x = coordinate(e.x, scale.x); e.y = coordinate(e.y, scale.y);
            e.rx *= scale.x.min(scale.y); e.ry *= scale.x.min(scale.y);
        }
    }
    (candidate, scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrRect, PartMeta};

    #[test]
    fn dense_rows_and_long_opposite_names_expand_without_changing_connections() {
        let mut source = SymbolIr { name: "arbitrary-name".into(), description: String::new(),
            designator: "U?".into(), meta: PartMeta::default(), pins: vec![],
            rects: vec![IrRect { x1: -5.08, y1: -3.81, x2: 5.08, y2: 3.81 }],
            polys: vec![], ellipses: vec![] };
        for side in [0, 1] {
            for row in 0..4 {
                source.pins.push(IrPin { x: if side == 0 { -5.08 } else { 5.08 },
                    y: 2.54 - f64::from(row) * 1.27, rotation: if side == 0 { 180.0 } else { 0.0 },
                    name: "LONG_CONTROL_SIGNAL".into(), number: format!("{}", side * 4 + row + 1),
                    ..Default::default() });
            }
        }
        assert_ne!(crowded_axes(&source), (false, false));
        let (arranged, scale) = arrange_with_scale(&source);
        assert_eq!(crowded_axes(&arranged), (false, false));
        assert!(scale.x > scale.y, "long names must not stretch all rows equally");
        assert!(arranged.rects[0].x2 > source.rects[0].x2);
        for (a, b) in source.pins.iter().zip(&arranged.pins) {
            assert_eq!((&a.name, &a.number, a.rotation, &a.pin_type), (&b.name, &b.number, b.rotation, &b.pin_type));
        }
        let (again, _) = arrange_with_scale(&arranged);
        assert_eq!(serde_json::to_value(&arranged).unwrap(), serde_json::to_value(again).unwrap());
        assert_eq!(source.rects[0].x2, 5.08, "other exporters' input must stay unchanged");
    }
}
