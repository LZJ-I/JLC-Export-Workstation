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
        if fallback.contains('\n') {
            return fallback.to_string();
        }
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
    /// 原理图位号前缀，如 `U?` / `R?` / `SW?`。
    pub designator: String,
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
    pub show_name: bool,
    pub show_number: bool,
}

impl Default for IrPin {
    fn default() -> Self {
        Self {
            number: String::new(),
            name: String::new(),
            x: 0.0,
            y: 0.0,
            length: 2.54,
            rotation: 0.0,
            pin_type: String::new(),
            show_name: true,
            show_number: true,
        }
    }
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
    pub model: Model3dPlacement,
}

/// STEP 相对封装原点的姿态（毫米 / 度），来自立创 `model_3d.transform`。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Model3dPlacement {
    pub rot_x: f64,
    pub rot_y: f64,
    pub rot_z: f64,
    pub off_x: f64,
    pub off_y: f64,
    pub off_z: f64,
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
    let rects: Vec<IrRect> = src
        .rects
        .into_iter()
        .map(|r| IrRect {
            x1: r.x1 * SYMBOL_UNIT_MM,
            y1: r.y1 * SYMBOL_UNIT_MM,
            x2: r.x2 * SYMBOL_UNIT_MM,
            y2: r.y2 * SYMBOL_UNIT_MM,
        })
        .collect();
    // 立创 PART.BBOX 只是点选包围盒，官方预览 / KiCad 导入都不画。
    let description = meta.describe(description);
    let mut symbol = SymbolIr {
        name: name.to_string(),
        description,
        designator: "U?".into(),
        meta,
        pins: src
            .pins
            .into_iter()
            .map(|p| IrPin {
                number: p.number,
                name: p.name,
                x: p.x * SYMBOL_UNIT_MM,
                y: p.y * SYMBOL_UNIT_MM,
                length: p.length.max(0.0) * SYMBOL_UNIT_MM,
                rotation: p.rotation,
                pin_type: p.pin_type,
                show_name: p.show_name,
                show_number: p.show_number,
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
    };
    let _ = src.part_box;
    attach_pins_from_length(&mut symbol);
    symbol
}

const MIN_PIN_LENGTH_MM: f64 = 2.54;

/// 立创 PIN 坐标是电气端点，长度指向本体。Altium 坐标是本体端，向外画线。
/// 按库里的长度贴上，不要用图形包围盒：LED 箭头会把包围盒撑出阴极，右边留缝。
fn attach_pins_from_length(symbol: &mut SymbolIr) {
    for pin in &mut symbol.pins {
        let length = pin.length;
        if length.is_finite() && length > 1e-6 {
            let toward = pin_quadrant(pin.rotation);
            match toward {
                0 => pin.x += length,
                1 => pin.y += length,
                2 => pin.x -= length,
                3 => pin.y -= length,
                _ => {}
            }
            pin.rotation = f64::from((toward + 2) % 4) * 90.0;
        }
        pin.length = length.max(MIN_PIN_LENGTH_MM);
    }
}

fn pin_quadrant(rotation_deg: f64) -> u8 {
    let a = crate::easyeda::normalize_angle(rotation_deg);
    ((a / 90.0).round() as i32).rem_euclid(4) as u8
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
        model: Model3dPlacement {
            rot_x: src.model.rot_x,
            rot_y: src.model.rot_y,
            rot_z: src.model.rot_z,
            off_x: src.model.off_x * FOOTPRINT_UNIT_MM,
            off_y: src.model.off_y * FOOTPRINT_UNIT_MM,
            off_z: src.model.off_z * FOOTPRINT_UNIT_MM,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::easyeda::{EasyedaSymbol, SymbolPin, SymbolRect};

    #[test]
    fn describe_keeps_multiline_product_intro() {
        let meta = PartMeta {
            lcsc: "C2846043".into(),
            manufacturer: "WCH".into(),
            ..Default::default()
        };
        assert_eq!(
            meta.describe("应用功能:USB转UART\nUSB协议版本:USB 2.0"),
            "应用功能:USB转UART\nUSB协议版本:USB 2.0"
        );
        assert_eq!(
            meta.describe("CH343P"),
            "CH343P | LCSC C2846043 | WCH"
        );
        let src = EasyedaSymbol {
            pins: vec![],
            rects: vec![],
            polys: vec![],
            ellipses: vec![],
            part_box: None,
        };
        let one_line = symbol_ir("CH343P", "CH343P", src.clone(), meta.clone());
        assert_eq!(one_line.description, "CH343P | LCSC C2846043 | WCH");
        let multi = symbol_ir(
            "CH343P",
            "应用功能:USB转UART\nUSB协议版本:USB 2.0",
            src,
            meta,
        );
        assert_eq!(
            multi.description,
            "应用功能:USB转UART\nUSB协议版本:USB 2.0"
        );
    }

    #[test]
    fn snaps_side_pins_to_body_edge() {
        let src = EasyedaSymbol {
            pins: vec![
                SymbolPin {
                    id: "a".into(),
                    x: -20.0,
                    y: 0.0,
                    length: 3.0,
                    rotation: 0.0,
                    number: "1".into(),
                    name: "1".into(),
                    pin_type: String::new(),
                    show_name: true,
                    show_number: true,
                },
                SymbolPin {
                    id: "b".into(),
                    x: 20.0,
                    y: 0.0,
                    length: 3.0,
                    rotation: 180.0,
                    number: "2".into(),
                    name: "2".into(),
                    pin_type: String::new(),
                    show_name: true,
                    show_number: true,
                },
            ],
            rects: vec![SymbolRect {
                x1: -10.0,
                y1: -4.0,
                x2: 10.0,
                y2: 4.0,
            }],
            polys: vec![],
            ellipses: vec![],
            part_box: None,
        };
        let sym = symbol_ir("L", "", src, PartMeta::default());
        let left = &sym.pins[0];
        let right = &sym.pins[1];
        assert!((left.x - (-17.0 * SYMBOL_UNIT_MM)).abs() < 1e-9);
        assert!((right.x - (17.0 * SYMBOL_UNIT_MM)).abs() < 1e-9);
        assert!((left.rotation - 180.0).abs() < 1e-9);
        assert!(right.rotation.abs() < 1e-9);
        assert!(left.length >= MIN_PIN_LENGTH_MM - 1e-9);
        assert!(right.length >= MIN_PIN_LENGTH_MM - 1e-9);
    }

    #[test]
    fn part_box_snaps_pins_but_is_not_drawn() {
        let src = EasyedaSymbol {
            pins: vec![SymbolPin {
                id: "a".into(),
                x: -20.0,
                y: 0.0,
                length: 3.0,
                rotation: 0.0,
                number: "1".into(),
                name: "KA1".into(),
                pin_type: String::new(),
                show_name: false,
                show_number: true,
            }],
            rects: vec![],
            polys: vec![],
            ellipses: vec![],
            part_box: Some((-10.0, -4.0, 10.0, 4.0)),
        };
        let sym = symbol_ir("LED", "", src, PartMeta::default());
        assert!(sym.rects.is_empty(), "EasyEDA BBOX must not become a rectangle");
        assert!((sym.pins[0].x - (-17.0 * SYMBOL_UNIT_MM)).abs() < 1e-9);
        assert!(!sym.pins[0].show_name);
        assert!(sym.pins[0].show_number);
    }

    #[test]
    fn led_arrows_do_not_leave_cathode_gap() {
        // C93880：阴极在 x=5，箭头伸到 x=9；脚长 15，应从 20 收到 5。
        let src = EasyedaSymbol {
            pins: vec![SymbolPin {
                id: "k".into(),
                x: 20.0,
                y: -10.0,
                length: 15.0,
                rotation: 180.0,
                number: "2".into(),
                name: "K1".into(),
                pin_type: String::new(),
                show_name: false,
                show_number: true,
            }],
            rects: vec![],
            polys: vec![
                vec![(5.0, -16.0), (5.0, -4.0)],
                vec![(2.0, -18.0), (9.0, -25.0)],
            ],
            ellipses: vec![],
            part_box: Some((-5.5, -29.5, 9.5, 29.5)),
        };
        let sym = symbol_ir("LED", "", src, PartMeta::default());
        assert!((sym.pins[0].x - (5.0 * SYMBOL_UNIT_MM)).abs() < 1e-9);
        assert!(sym.pins[0].rotation.abs() < 1e-9);
    }
}
