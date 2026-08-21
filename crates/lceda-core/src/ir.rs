//! EasyEDA 单位 → 中间表示（毫米）。

use crate::easyeda::{EasyedaFootprint, EasyedaSymbol};

/// EasyEDA 原理图：1 单位 = 10 mil = 0.254 mm
pub const SYMBOL_UNIT_MM: f64 = 0.254;
/// EasyEDA 封装：1 单位 = 1 mil = 0.0254 mm
pub const FOOTPRINT_UNIT_MM: f64 = 0.0254;

#[derive(Debug, Clone, Default)]
pub struct PartMeta {
    pub lcsc: String,
    pub mpn: String,
    pub manufacturer: String,
    pub datasheet: String,
    pub footprint_lib: String,
}

impl PartMeta {
    pub fn describe(&self, fallback: &str) -> String {
        let mut parts = Vec::new();
        if !fallback.is_empty() {
            parts.push(fallback.to_string());
        }
        if !self.lcsc.is_empty() {
            parts.push(format!("LCSC {}", self.lcsc));
        }
        if !self.manufacturer.is_empty() {
            parts.push(self.manufacturer.clone());
        }
        if parts.is_empty() {
            fallback.to_string()
        } else {
            parts.join(" | ")
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolIr {
    pub name: String,
    pub description: String,
    pub meta: PartMeta,
    pub pins: Vec<IrPin>,
    pub rects: Vec<IrRect>,
    pub polys: Vec<Vec<(f64, f64)>>,
    pub ellipses: Vec<IrEllipse>,
}

#[derive(Debug, Clone)]
pub struct IrPin {
    pub number: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub length: f64,
    pub rotation: f64,
    pub pin_type: String,
}

#[derive(Debug, Clone)]
pub struct IrRect {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Debug, Clone)]
pub struct IrEllipse {
    pub x: f64,
    pub y: f64,
    pub rx: f64,
    pub ry: f64,
}

#[derive(Debug, Clone)]
pub struct FootprintIr {
    pub name: String,
    pub description: String,
    pub meta: PartMeta,
    pub pads: Vec<IrPad>,
    pub tracks: Vec<IrTrack>,
    pub circles: Vec<IrCircle>,
    pub arcs: Vec<IrArc>,
    pub regions: Vec<IrRegion>,
}

#[derive(Debug, Clone)]
pub struct IrPad {
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
    /// Absolute outline in mm; set for irregular POLY pads.
    pub polygon: Option<Vec<(f64, f64)>>,
}

impl IrPad {
    pub fn is_custom_poly(&self) -> bool {
        self.hole <= 1e-6
            && self.shape.eq_ignore_ascii_case("POLY")
            && self.polygon.as_ref().is_some_and(|p| p.len() >= 3)
    }
}

#[derive(Debug, Clone)]
pub struct IrTrack {
    pub layer: i32,
    pub width: f64,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone)]
pub struct IrCircle {
    pub layer: i32,
    pub width: f64,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

#[derive(Debug, Clone)]
pub struct IrArc {
    pub layer: i32,
    pub width: f64,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone)]
pub struct IrRegion {
    pub layer: i32,
    pub points: Vec<(f64, f64)>,
}

pub fn symbol_ir(name: &str, description: &str, src: EasyedaSymbol, meta: PartMeta) -> SymbolIr {
    let mut rects: Vec<IrRect> = src
        .rects
        .into_iter()
        .map(|r| IrRect {
            x1: r.x1 * SYMBOL_UNIT_MM,
            y1: r.y1 * SYMBOL_UNIT_MM,
            x2: r.x2 * SYMBOL_UNIT_MM,
            y2: r.y2 * SYMBOL_UNIT_MM,
        })
        .collect();
    if rects.is_empty() {
        if let Some((x1, y1, x2, y2)) = src.part_box {
            rects.push(IrRect {
                x1: x1 * SYMBOL_UNIT_MM,
                y1: y1 * SYMBOL_UNIT_MM,
                x2: x2 * SYMBOL_UNIT_MM,
                y2: y2 * SYMBOL_UNIT_MM,
            });
        }
    }
    SymbolIr {
        name: name.to_string(),
        description: meta.describe(description),
        meta,
        pins: src
            .pins
            .into_iter()
            .map(|p| IrPin {
                number: p.number,
                name: p.name,
                x: p.x * SYMBOL_UNIT_MM,
                y: p.y * SYMBOL_UNIT_MM,
                length: p.length.max(10.0) * SYMBOL_UNIT_MM,
                rotation: p.rotation,
                pin_type: p.pin_type,
            })
            .collect(),
        rects,
        polys: src
            .polys
            .into_iter()
            .map(|poly| poly.into_iter().map(|(x, y)| (x * SYMBOL_UNIT_MM, y * SYMBOL_UNIT_MM)).collect())
            .collect(),
        ellipses: src
            .ellipses
            .into_iter()
            .map(|e| IrEllipse {
                x: e.x * SYMBOL_UNIT_MM,
                y: e.y * SYMBOL_UNIT_MM,
                rx: e.rx * SYMBOL_UNIT_MM,
                ry: e.ry * SYMBOL_UNIT_MM,
            })
            .collect(),
    }
}

pub fn footprint_ir(name: &str, description: &str, src: EasyedaFootprint, meta: PartMeta) -> FootprintIr {
    FootprintIr {
        name: name.to_string(),
        description: meta.describe(description),
        meta,
        pads: src
            .pads
            .into_iter()
            .map(|p| IrPad {
                designator: p.designator,
                x: p.x * FOOTPRINT_UNIT_MM,
                y: p.y * FOOTPRINT_UNIT_MM,
                width: p.width * FOOTPRINT_UNIT_MM,
                height: p.height * FOOTPRINT_UNIT_MM,
                hole: p.hole.max(0.0) * FOOTPRINT_UNIT_MM,
                hole_slot: p.hole_slot.max(0.0) * FOOTPRINT_UNIT_MM,
                hole_shape: p.hole_shape,
                rotation: p.rotation,
                layer: p.layer,
                shape: p.shape,
                polygon: p.polygon.map(|pts| {
                    pts.into_iter()
                        .map(|(x, y)| (x * FOOTPRINT_UNIT_MM, y * FOOTPRINT_UNIT_MM))
                        .collect()
                }),
            })
            .collect(),
        tracks: src
            .tracks
            .into_iter()
            .map(|t| IrTrack {
                layer: t.layer,
                width: t.width * FOOTPRINT_UNIT_MM,
                points: t
                    .points
                    .into_iter()
                    .map(|(x, y)| (x * FOOTPRINT_UNIT_MM, y * FOOTPRINT_UNIT_MM))
                    .collect(),
            })
            .collect(),
        circles: src
            .circles
            .into_iter()
            .map(|c| IrCircle {
                layer: c.layer,
                width: c.width * FOOTPRINT_UNIT_MM,
                x: c.x * FOOTPRINT_UNIT_MM,
                y: c.y * FOOTPRINT_UNIT_MM,
                radius: c.radius * FOOTPRINT_UNIT_MM,
            })
            .collect(),
        arcs: src
            .arcs
            .into_iter()
            .map(|a| IrArc {
                layer: a.layer,
                width: a.width * FOOTPRINT_UNIT_MM,
                x: a.x * FOOTPRINT_UNIT_MM,
                y: a.y * FOOTPRINT_UNIT_MM,
                radius: a.radius * FOOTPRINT_UNIT_MM,
                start: a.start,
                end: a.end,
            })
            .collect(),
        regions: src
            .regions
            .into_iter()
            .map(|r| IrRegion {
                layer: r.layer,
                points: r
                    .points
                    .into_iter()
                    .map(|(x, y)| (x * FOOTPRINT_UNIT_MM, y * FOOTPRINT_UNIT_MM))
                    .collect(),
            })
            .collect(),
    }
}
