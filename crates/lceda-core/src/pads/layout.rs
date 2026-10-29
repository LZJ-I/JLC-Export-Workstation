//! PADS-only geometry adaptation. No device names or pin-number special cases.
//! Keep source topology, pin order and orientation; expand cramped geometry while
//! retaining readable text sizes and external pin lengths on the 10 mil grid.
use super::{clean_attr, pin_outward, sch_pin_tip, sch_pin_length, snap10, SCH_NAME_HEIGHT,
    SCH_NUMBER_HEIGHT, SCH_TEXT_GAP, SchematicGrid};
use super::grid::AxisMap;
use crate::error::{Error, Result};
use crate::ir::{IrPin, SymbolIr};

const CLEARANCE: f64 = 20.0;

fn same_anchor(a: &IrPin, b: &IrPin, grid: SchematicGrid) -> bool {
    grid.coordinate(a.x) == grid.coordinate(b.x) && grid.coordinate(a.y) == grid.coordinate(b.y)
        && pin_outward(a.rotation, 1.0) == pin_outward(b.rotation, 1.0)
}

/// Stacked electrical terminals stay stacked. Only their visible labels merge:
/// repeated VDD names become one VDD, while every distinct pin number remains.
pub(super) fn display_pins_on_grid(symbol: &SymbolIr, grid: SchematicGrid) -> Vec<IrPin> {
    let mut groups: Vec<Vec<&IrPin>> = Vec::new();
    for pin in &symbol.pins {
        if let Some(group) = groups.iter_mut().find(|g| same_anchor(g[0], pin, grid)) { group.push(pin); }
        else { groups.push(vec![pin]); }
    }
    groups.into_iter().map(|group| {
        if group.len() == 1 { return group[0].clone(); }
        let mut pin = group[0].clone();
        let join = |name: bool| {
            let mut labels = Vec::new();
            for p in &group {
                let (visible, text) = if name { (p.show_name, &p.name) } else { (p.show_number, &p.number) };
                if visible && !text.is_empty() && !labels.contains(&text) { labels.push(text); }
            }
            labels.into_iter().cloned().collect::<Vec<_>>().join(if name { "/" } else { "," })
        };
        pin.name = join(true); pin.number = join(false);
        pin.show_name = !pin.name.is_empty(); pin.show_number = !pin.number.is_empty();
        pin.length = group.iter().map(|p| p.length).fold(0.0, f64::max);
        pin
    }).collect()
}

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

fn name_bounds(pin: &IrPin, grid: SchematicGrid) -> Bounds {
    let (dx, dy) = pin_outward(pin.rotation, 1.0);
    let x = grid.coordinate(pin.x) - dx * SCH_TEXT_GAP;
    let y = grid.coordinate(pin.y) - dy * SCH_TEXT_GAP;
    let w = text_width(&pin.name, SCH_NAME_HEIGHT);
    // Font height is nominal: underscores and glyph overshoots can extend well
    // beyond the capital height. Reserve their full vertical ink bounds too.
    let h = SCH_NAME_HEIGHT * 1.4 / 2.0;
    if dx < 0.0 { Bounds { x0: x, x1: x + w, y0: y - h, y1: y + h } }
    else if dx > 0.0 { Bounds { x0: x - w, x1: x, y0: y - h, y1: y + h } }
    else if dy > 0.0 { Bounds { x0: x - h, x1: x + h, y0: y - w, y1: y } }
    else { Bounds { x0: x - h, x1: x + h, y0: y, y1: y + w } }
}

fn number_bounds(pin: &IrPin, grid: SchematicGrid) -> Bounds {
    let (x, y) = sch_pin_tip(pin, grid);
    let (dx, dy) = pin_outward(pin.rotation, sch_pin_length(pin, grid) / 2.0);
    let (x, y) = (x - snap10(dx), y - snap10(dy));
    let w = text_width(&pin.number, SCH_NUMBER_HEIGHT) / 2.0;
    if dy.abs() > 0.0 {
        Bounds { x0: x - 15.0 - SCH_NUMBER_HEIGHT, x1: x - 15.0, y0: y - w, y1: y + w }
    } else {
        Bounds { x0: x - w, x1: x + w, y0: y + 15.0, y1: y + 15.0 + SCH_NUMBER_HEIGHT }
    }
}

fn crowded_axes(symbol: &SymbolIr, grid: SchematicGrid) -> (bool, bool) {
    let mut boxes = Vec::new();
    let (mut grow_x, mut grow_y) = (false, false);
    for pin in &display_pins_on_grid(symbol, grid) {
        let anchor = (grid.coordinate(pin.x), grid.coordinate(pin.y));
        if pin.show_name && !pin.name.is_empty() {
            let b = name_bounds(pin, grid);
            boxes.push((b, anchor));
            let (dx, dy) = pin_outward(pin.rotation, 1.0);
            // Check text depth inside a rectangular body attached to this pin.
            // Nonrectangular symbols retain their original topology as well.
            for r in &symbol.rects {
                let (x0, x1) = (grid.coordinate(r.x1.min(r.x2)), grid.coordinate(r.x1.max(r.x2)));
                let (y0, y1) = (grid.coordinate(r.y1.min(r.y2)), grid.coordinate(r.y1.max(r.y2)));
                let (x, y) = (grid.coordinate(pin.x), grid.coordinate(pin.y));
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
        if pin.show_number { boxes.push((number_bounds(pin, grid), anchor)); }
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
            // Scaling cannot separate labels sharing the same anchor.
            if !x.is_finite() && !y.is_finite() { continue; }
            if x <= y { grow_x = true; } else { grow_y = true; }
        }
    }
    (grow_x, grow_y)
}

pub(super) fn arrange_with_scale(source: &SymbolIr) -> (SymbolIr, Scale) {
    let grid = SchematicGrid::default();
    let mut candidate = source.clone();
    // Long alphanumeric numbers also need enough shaft space outside the body.
    let displayed = display_pins_on_grid(&candidate, grid);
    for pin in &mut candidate.pins {
        let label = displayed.iter().find(|p| same_anchor(p, pin, grid)).unwrap();
        let required = text_width(&label.number, SCH_NUMBER_HEIGHT) + 10.0;
        pin.length = label.length;
        if label.show_number {
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
        let (mut grow_x, mut grow_y) = crowded_axes(&candidate, grid);
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

/// Apply the same geometry transform to source annotations and body artwork.
pub(super) struct Placement {
    scale: Scale,
    axes: Option<(AxisMap, AxisMap)>,
}

impl Placement {
    pub fn x(&self, mils: f64) -> f64 {
        let v = mils * self.scale.x;
        self.axes.as_ref().map_or(v, |(x, _)| x.map(v))
    }
    pub fn y(&self, mils: f64) -> f64 {
        let v = mils * self.scale.y;
        self.axes.as_ref().map_or(v, |(_, y)| y.map(v))
    }

    fn poly(&self, points: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let map = |(x, y)| (self.x(super::mils(x)) * 0.0254, self.y(super::mils(y)) * 0.0254);
        let Some((x_axis, y_axis)) = &self.axes else { return points.iter().copied().map(map).collect(); };
        let mut out = Vec::new();
        for edge in points.windows(2) {
            let ((x0, y0), (x1, y1)) = (edge[0], edge[1]);
            let mut cuts: Vec<_> = x_axis.cuts(super::mils(x0) * self.scale.x, super::mils(x1) * self.scale.x)
                .chain(y_axis.cuts(super::mils(y0) * self.scale.y, super::mils(y1) * self.scale.y)).collect();
            cuts.push(0.0); cuts.sort_by(f64::total_cmp); cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
            out.extend(cuts.into_iter().map(|t| map((x0 + (x1 - x0) * t, y0 + (y1 - y0) * t))));
        }
        if let Some(&last) = points.last() { out.push(map(last)); }
        out
    }
}

fn project(source: &SymbolIr, scale: Scale, grid: SchematicGrid) -> (SymbolIr, Placement) {
    let visible = |p: &&IrPin| p.show_name || p.show_number;
    let columns = source.pins.iter().filter(visible).filter(|p| pin_outward(p.rotation, 1.0).1 != 0.0)
        .map(|p| super::mils(p.x) * scale.x).collect();
    let rows = source.pins.iter().filter(visible).filter(|p| pin_outward(p.rotation, 1.0).0 != 0.0)
        .map(|p| super::mils(p.y) * scale.y).collect();
    let min_pitch = SCH_NAME_HEIGHT.max(SCH_NUMBER_HEIGHT) * 1.4 + CLEARANCE;
    let placement = Placement { scale, axes: Some((
        AxisMap::new(source.pins.iter().map(|p| super::mils(p.x) * scale.x), columns, min_pitch, grid),
        AxisMap::new(source.pins.iter().map(|p| super::mils(p.y) * scale.y), rows, min_pitch, grid),
    )) };
    let x = |mm| placement.x(super::mils(mm)) * 0.0254;
    let y = |mm| placement.y(super::mils(mm)) * 0.0254;
    let mut out = source.clone();
    for pin in &mut out.pins { pin.x = x(pin.x); pin.y = y(pin.y); }
    for r in &mut out.rects { r.x1 = x(r.x1); r.x2 = x(r.x2); r.y1 = y(r.y1); r.y2 = y(r.y2); }
    // A nonlinear grid warp must subdivide sloped segments at its breakpoints.
    // Otherwise a pin on an op-amp edge can become detached from the outline.
    for poly in &mut out.polys { *poly = placement.poly(poly); }
    out.ellipses.clear();
    for e in &source.ellipses {
        let radius = e.rx.max(e.ry); // Same circular outline as the PADS writer.
        let attached: Vec<_> = source.pins.iter().filter(|p|
            ((p.x - e.x).hypot(p.y - e.y) - radius).abs() < 1e-6).collect();
        if !attached.is_empty() && radius > 0.0 {
            // Include every attached pin exactly, in addition to the sampled
            // outline. A warped circle must not leave its terminals behind.
            let mut angles: Vec<_> = (0..64).map(|i| f64::from(i) * std::f64::consts::TAU / 64.0)
                .chain(attached.iter().map(|p| (p.y - e.y).atan2(p.x - e.x).rem_euclid(std::f64::consts::TAU))).collect();
            angles.sort_by(f64::total_cmp); angles.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
            let mut outline: Vec<_> = angles.into_iter().map(|a| (e.x + radius * a.cos(), e.y + radius * a.sin())).collect();
            outline.push(outline[0]);
            out.polys.push(placement.poly(&outline));
        } else {
            // Small polarity dots/annotations retain their round shape.
            let mut e = e.clone();
            e.x = x(e.x); e.y = y(e.y);
            e.rx *= scale.x.min(scale.y); e.ry *= scale.x.min(scale.y);
            out.ellipses.push(e);
        }
    }
    (out, placement)
}

pub(super) fn arrange_with_grid(source: &SymbolIr, grid: SchematicGrid) -> Result<(SymbolIr, Placement)> {
    if grid == SchematicGrid::default() {
        let (symbol, scale) = arrange_with_scale(source);
        return Ok((symbol, Placement { scale, axes: None }));
    }
    if source.pins.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || !p.length.is_finite()) {
        return Err(Error::msg("PADS 原理图含无效引脚坐标"));
    }
    let mut scale = Scale { x: 1.0, y: 1.0 };
    let mut template = source.clone();
    // Project before grouping, so distinct source anchors never become stacks
    // merely because a coarser target grid would round them to the same point.
    let (initial, _) = project(&template, scale, grid);
    let displayed = display_pins_on_grid(&initial, grid);
    for (pin, projected) in template.pins.iter_mut().zip(&initial.pins) {
        let label = displayed.iter().find(|p| same_anchor(p, projected, grid)).unwrap();
        let required = if label.show_number { text_width(&label.number, SCH_NUMBER_HEIGHT) + 10.0 } else { 0.0 };
        pin.length = grid.ceil(super::mils(label.length).max(required).max(super::SCH_PIN_LENGTH)) * 0.0254;
    }
    let independent_axes = !source.rects.is_empty() && source.polys.is_empty();
    loop {
        let (candidate, placement) = project(&template, scale, grid);
        let (mut grow_x, mut grow_y) = crowded_axes(&candidate, grid);
        // Different bodies/directions can point toward each other. Also reject
        // accidental coincident electrical endpoints, not just label collisions.
        for (i, a) in candidate.pins.iter().enumerate() {
            for b in &candidate.pins[i + 1..] {
                if !same_anchor(a, b, grid) && sch_pin_tip(a, grid) == sch_pin_tip(b, grid) {
                    grow_x = true; grow_y = true;
                }
            }
        }
        if !grow_x && !grow_y { return Ok((candidate, placement)); }
        if !independent_axes { grow_x = true; grow_y = true; }
        let next = Scale {
            x: if grow_x { (scale.x * 1.25).min(32.0) } else { scale.x },
            y: if grow_y { (scale.y * 1.25).min(32.0) } else { scale.y },
        };
        if next.x == scale.x && next.y == scale.y {
            return Err(Error::msg(format!("PADS 符号 {} 无法在 {} mil 栅格上完成无重叠排版", source.name, grid.mils())));
        }
        scale = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrRect, PartMeta};

    #[test]
    fn configurable_grid_preserves_distinct_rows_stacks_and_pin_metadata() {
        for rotation in [0.0, 90.0, 180.0, 270.0] {
            let vertical = rotation == 90.0 || rotation == 270.0;
            let (x, y) = pin_outward(rotation, 5.08);
            let mut source = SymbolIr { name: "mixed-grid".into(), description: String::new(),
                designator: "U?".into(), meta: PartMeta::default(), rects: vec![], polys: vec![], ellipses: vec![],
                pins: [37.5, 38.5, 162.5, 287.5].iter().enumerate().map(|(i, offset)| IrPin {
                    x: x + if vertical { offset * 0.0254 } else { 0.0 },
                    y: y + if vertical { 0.0 } else { offset * 0.0254 },
                    rotation, length: 6.4135, number: format!("A{i}"), name: format!("SIGNAL_{i}"),
                    ..Default::default()
                }).collect() };
            let mut stacked = source.pins[0].clone(); stacked.number = "EP".into();
            source.pins.push(stacked);
            let original = serde_json::to_value(&source).unwrap();
            for mils in [1, 7, 25, 50, 100, 250, 1000] {
                let grid = SchematicGrid::new(mils).unwrap();
                let (arranged, _) = arrange_with_grid(&source, grid).unwrap();
                assert_eq!(arranged.pins.len(), 5);
                assert_eq!(display_pins_on_grid(&arranged, grid).len(), 4);
                assert_eq!(crowded_axes(&arranged, grid), (false, false));
                let tips: Vec<_> = arranged.pins.iter().map(|pin| sch_pin_tip(pin, grid)).collect();
                assert_eq!(tips[0], tips[4], "original stack stays connected");
                for (i, &(x, y)) in tips[..4].iter().enumerate() {
                    assert_eq!(x.rem_euclid(f64::from(mils)), 0.0);
                    assert_eq!(y.rem_euclid(f64::from(mils)), 0.0);
                    assert!(!tips[..i].contains(&(x, y)), "independent pins must not merge");
                }
                let rows: Vec<_> = arranged.pins[..4].iter().map(|p| if vertical { p.x } else { p.y }).collect();
                assert!(rows.windows(2).all(|r| r[1] > r[0]), "source order is retained");
                for (before, after) in source.pins.iter().zip(&arranged.pins) {
                    assert_eq!((&before.number, &before.name, before.rotation, &before.pin_type, before.show_name, before.show_number),
                        (&after.number, &after.name, after.rotation, &after.pin_type, after.show_name, after.show_number));
                }
                // A second layout must neither inflate the body nor shift pins.
                assert_eq!(super::super::sch_library_with_grid(&[&arranged], grid).unwrap(),
                    super::super::sch_library_with_grid(&[&source], grid).unwrap());
            }
            assert_eq!(serde_json::to_value(source).unwrap(), original);
        }
    }

    #[test]
    fn rectangular_bodies_and_annotations_follow_the_grid_transform() {
        let raw = serde_json::from_str(include_str!("../../tests/fixtures/BTS7008-1EPP_symbol_easyeda.json")).unwrap();
        let source = crate::ir::symbol_ir("any-name", "", crate::easyeda::parse_symbol(&raw).unwrap(), PartMeta::default());
        for mils in [25, 50, 100] {
            let grid = SchematicGrid::new(mils).unwrap();
            let (arranged, placement) = arrange_with_grid(&source, grid).unwrap();
            assert_eq!(crowded_axes(&arranged, grid), (false, false));
            assert_eq!((placement.x(0.0), placement.y(0.0)), (0.0, 0.0));
            for (before, after) in source.pins.iter().zip(&arranged.pins) {
                assert!((placement.x(super::super::mils(before.x)) - super::super::mils(after.x)).abs() < 1e-6);
                assert!((placement.y(super::super::mils(before.y)) - super::super::mils(after.y)).abs() < 1e-6);
                let body = &arranged.rects[0];
                let (dx, dy) = pin_outward(after.rotation, 1.0);
                let edge = if dx < 0.0 { body.x1.min(body.x2) } else if dx > 0.0 { body.x1.max(body.x2) }
                    else if dy < 0.0 { body.y1.min(body.y2) } else { body.y1.max(body.y2) };
                assert!((edge - if dx == 0.0 { after.y } else { after.x }).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn sloped_and_round_outlines_keep_their_pin_attachment_points() {
        let mut source = SymbolIr { name: "outline".into(), description: String::new(), designator: "U?".into(),
            meta: PartMeta::default(), rects: vec![], polys: vec![vec![(-5.08, -5.08), (5.08, 0.0), (-5.08, 5.08)]],
            ellipses: vec![], pins: vec![IrPin { x: 0.0, y: 2.54, rotation: 90.0, name: "VCC".into(), number: "1".into(), ..Default::default() }] };
        let grid = SchematicGrid::new(250).unwrap();
        let check = |source: &SymbolIr| {
            let (arranged, _) = arrange_with_grid(source, grid).unwrap();
            for pin in &arranged.pins {
                assert!(arranged.polys.iter().flatten().any(|&(x, y)|
                    grid.coordinate(x) == grid.coordinate(pin.x) && grid.coordinate(y) == grid.coordinate(pin.y)),
                    "terminal must remain attached to the transformed body");
            }
        };
        check(&source);
        source.polys.clear();
        source.ellipses.push(crate::ir::IrEllipse { x: 0.0, y: 0.0, rx: 2.54, ry: 2.54 });
        check(&source);
    }

    #[test]
    fn stacked_power_pins_keep_connections_and_fit_without_growing_the_body() {
        for rotation in [0.0, 90.0, 180.0, 270.0] {
            let (x, y) = pin_outward(rotation, 5.08);
            let source = SymbolIr { name: "StackedPower".into(), description: String::new(), designator: "U?".into(),
                meta: PartMeta::default(), pins: (1..=3).map(|i| IrPin {
                    number: i.to_string(), name: "VDD".into(), x, y, rotation, ..Default::default()
                }).collect(), rects: vec![IrRect { x1: -5.08, y1: -5.08, x2: 5.08, y2: 5.08 }], polys: vec![], ellipses: vec![] };
            let (arranged, scale) = arrange_with_scale(&source);
            assert_eq!((scale.x, scale.y), (1.0, 1.0));
            assert_eq!(crowded_axes(&arranged, SchematicGrid::default()), (false, false));
            assert_eq!(arranged.pins.len(), 3);
            for (before, after) in source.pins.iter().zip(&arranged.pins) {
                assert_eq!((&before.number, &before.name, before.x, before.y, before.rotation),
                           (&after.number, &after.name, after.x, after.y, after.rotation));
                assert_eq!(sch_pin_tip(after, SchematicGrid::default()), sch_pin_tip(&arranged.pins[0], SchematicGrid::default()));
            }
            let displayed = display_pins_on_grid(&arranged, SchematicGrid::default());
            assert_eq!(displayed.len(), 1); assert_eq!(displayed[0].name, "VDD"); assert_eq!(displayed[0].number, "1,2,3");
            assert_eq!(serde_json::to_value(arrange_with_scale(&arranged).0).unwrap(), serde_json::to_value(arranged).unwrap());
        }
    }

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
        assert_ne!(crowded_axes(&source, SchematicGrid::default()), (false, false));
        let (arranged, scale) = arrange_with_scale(&source);
        assert_eq!(crowded_axes(&arranged, SchematicGrid::default()), (false, false));
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
