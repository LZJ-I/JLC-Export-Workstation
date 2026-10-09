//! A display list decoded from the exporter's actual SchLib streams.
//! Coordinates are mm; text sizes are Altium points. No EasyEDA geometry or
//! separate preview labels are involved. Native AD can apply user-specific
//! pin margins; the preview uses the standard inside-name/above-number layout.

use crate::error::{Error, Result};
use encoding_rs::{GBK, WINDOWS_1252};
use std::collections::BTreeMap;
use std::io::Read;

// SchLib's schematic size is a document unit: size 10 is about 100 mils,
// rather than a printed typographic 10 pt. See AltiumSharp SchComponentRenderer.
pub const MM_PER_POINT: f64 = 0.254;
type Point = (f64, f64);
type Params = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub struct SchFont {
    pub name: String,
    pub points: f64,
    pub color: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnchor { LeftBottom, LeftCenter, RightCenter, CenterBottom }

#[derive(Debug, Clone, PartialEq)]
pub struct SchText {
    pub role: String,
    pub text: String,
    pub position: Point,
    /// Counterclockwise in the document's upward Y coordinate system.
    pub rotation: f64,
    pub anchor: TextAnchor,
    pub font: SchFont,
    pub color: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchStroke {
    pub points: Vec<Point>,
    pub color: i32,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchEllipse {
    pub center: Point,
    pub radii: Point,
    pub color: i32,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchPreview {
    pub background: i32,
    pub strokes: Vec<SchStroke>,
    pub ellipses: Vec<SchEllipse>,
    pub texts: Vec<SchText>,
}

impl SchPreview {
    pub fn from_streams(header: &[u8], data: &[u8], pin_text: Option<&[u8]>) -> Result<Self> {
        let mut offset = 0;
        let (_, bytes) = block(header, &mut offset)?;
        let header = params(bytes);
        let mut fonts = BTreeMap::new();
        for id in 1..=number(&header, "FONTIDCOUNT", 1.0).clamp(1.0, 1024.0) as usize {
            fonts.insert(id, SchFont {
                name: text(&header, &format!("FONTNAME{id}")).to_string(),
                points: number(&header, &format!("SIZE{id}"), 10.0).clamp(1.0, 256.0),
                color: header.get(&format!("COLOR{id}")).and_then(|value| value.parse().ok()),
            });
        }
        let overrides = read_pin_text(pin_text)?;
        let mut scene = Self { background: number(&header, "AREACOLOR", 16317695.0) as i32,
            strokes: Vec::new(), ellipses: Vec::new(), texts: Vec::new() };
        let mut offset = 0;
        let mut pin_index = 0;
        while offset < data.len() {
            let (flags, bytes) = block(data, &mut offset)?;
            if flags == 1 {
                scene.add_pin(binary_pin(bytes)?, overrides.get(&pin_index), &fonts)?;
                pin_index += 1;
                continue;
            }
            let p = params(bytes);
            let color = number(&p, "COLOR", 0.0) as i32;
            let width = match number(&p, "LINEWIDTH", 1.0) as i32 {
                0 => 0.0254, 2 => 0.762, 3 => 1.27, _ => 0.254,
            };
            match text(&p, "RECORD") {
                "2" => {
                    scene.add_pin(Pin { position: point(&p, "LOCATION"),
                        length: number(&p, "PINLENGTH", 10.0) * 0.254,
                        flags: number(&p, "PINCONGLOMERATE", 0.0) as u8,
                        color, name: text(&p, "NAME").to_string(), number: text(&p, "DESIGNATOR").to_string(),
                        name_font: number(&p, "NAME_CUSTOMFONTID", 1.0) as usize,
                        number_font: number(&p, "DESIGNATOR_CUSTOMFONTID", 1.0) as usize,
                    }, overrides.get(&pin_index), &fonts)?;
                    pin_index += 1;
                }
                "14" => {
                    let (x1, y1) = point(&p, "LOCATION");
                    let (x2, y2) = point(&p, "CORNER");
                    scene.strokes.push(SchStroke { points: vec![(x1,y1), (x2,y1), (x2,y2), (x1,y2), (x1,y1)], color, width });
                }
                "6" => {
                    let count = number(&p, "LOCATIONCOUNT", 0.0).clamp(0.0, 100_000.0) as usize;
                    let points = (1..=count).map(|i| (coord(&p, &format!("X{i}")), coord(&p, &format!("Y{i}")))).collect();
                    scene.strokes.push(SchStroke { points, color, width });
                }
                "8" => scene.ellipses.push(SchEllipse { center: point(&p, "LOCATION"),
                    radii: (coord(&p, "RADIUS"), coord(&p, "SECONDARYRADIUS")), color, width }),
                "34" | "41" if text(&p, "ISHIDDEN") != "T" => {
                    let font = font(&fonts, number(&p, "FONTID", 1.0) as usize)?;
                    scene.texts.push(SchText { role: text(&p, "NAME").to_string(), text: text(&p, "TEXT").to_string(),
                        position: point(&p, "LOCATION"), rotation: number(&p, "ORIENTATION", 0.0) * 90.0,
                        anchor: TextAnchor::LeftBottom, font, color });
                }
                _ => {} // Component metadata and footprint implementations are not visible graphics.
            }
        }
        Ok(scene)
    }

    fn add_pin(&mut self, pin: Pin, attrs: Option<&PinText>, fonts: &BTreeMap<usize, SchFont>) -> Result<()> {
        let orientation = pin.flags & 3;
        let (dx, dy) = [(1.0,0.0), (0.0,1.0), (-1.0,0.0), (0.0,-1.0)][orientation as usize];
        let (x, y) = pin.position;
        self.strokes.push(SchStroke { points: vec![(x,y), (x+dx*pin.length, y+dy*pin.length)], color: pin.color, width: 0.1524 });
        let mut name_font = font(fonts, attrs.map(|a| a.name_font).unwrap_or(pin.name_font))?;
        let mut number_font = font(fonts, attrs.map(|a| a.number_font).unwrap_or(pin.number_font))?;
        if let Some(attrs) = attrs { name_font.color = Some(attrs.name_color); number_font.color = Some(attrs.number_color); }
        // AD names are inside the body; designators run along the pin shaft.
        // All margins are document units, so font size and spacing scale together.
        let margin = 0.508;
        let rotation = if orientation % 2 == 0 { 0.0 } else { 90.0 };
        if pin.flags & 8 != 0 && !pin.name.is_empty() {
            self.texts.push(SchText { role: "PinName".into(), text: pin.name,
                position: (x-dx*margin, y-dy*margin), rotation,
                anchor: if orientation == 2 || orientation == 3 { TextAnchor::LeftCenter } else { TextAnchor::RightCenter },
                color: name_font.color.unwrap_or(pin.color), font: name_font });
        }
        if pin.flags & 16 != 0 && !pin.number.is_empty() {
            self.texts.push(SchText { role: "PinDesignator".into(), text: pin.number,
                position: (x+dx*pin.length*0.5-if orientation%2 == 1 { margin } else { 0.0 },
                    y+dy*pin.length*0.5+if orientation%2 == 0 { margin } else { 0.0 }),
                rotation, anchor: TextAnchor::CenterBottom,
                color: number_font.color.unwrap_or(pin.color), font: number_font });
        }
        Ok(())
    }
}

struct Pin {
    position: Point, length: f64, flags: u8, color: i32, name: String, number: String,
    name_font: usize, number_font: usize,
}
struct PinText { name_font: usize, name_color: i32, number_font: usize, number_color: i32 }

fn invalid() -> Error { Error::Altium("原理图预览记录不完整".into()) }
fn block<'a>(data: &'a [u8], offset: &mut usize) -> Result<(u8, &'a [u8])> {
    let header = data.get(*offset..*offset+4).ok_or_else(invalid)?;
    let size = u32::from_le_bytes(header.try_into().unwrap());
    *offset += 4;
    let end = offset.checked_add((size & 0x00ff_ffff) as usize).ok_or_else(invalid)?;
    let bytes = data.get(*offset..end).ok_or_else(invalid)?;
    *offset = end;
    Ok(((size >> 24) as u8, bytes))
}
fn params(bytes: &[u8]) -> Params {
    let decoded = GBK.decode(bytes.strip_suffix(&[0]).unwrap_or(bytes)).0;
    decoded.split('|').filter_map(|s| s.split_once('='))
        .map(|(k,v)| (k.to_ascii_uppercase(), v.to_string())).collect()
}
fn text<'a>(p: &'a Params, key: &str) -> &'a str { p.get(key).map(String::as_str).unwrap_or("") }
fn number(p: &Params, key: &str, default: f64) -> f64 {
    p.get(key).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(default)
}
fn coord(p: &Params, key: &str) -> f64 {
    (number(p, key, 0.0) + number(p, &format!("{key}_FRAC"), 0.0)/100_000.0)*0.254
}
fn point(p: &Params, key: &str) -> Point { (coord(p, &format!("{key}.X")), coord(p, &format!("{key}.Y"))) }
fn font(fonts: &BTreeMap<usize, SchFont>, id: usize) -> Result<SchFont> {
    fonts.get(&id).or_else(|| fonts.get(&1)).cloned().ok_or_else(invalid)
}
fn short(bytes: &[u8], offset: &mut usize) -> Result<String> {
    let len = *bytes.get(*offset).ok_or_else(invalid)? as usize;
    *offset += 1;
    let text = bytes.get(*offset..*offset+len).ok_or_else(invalid)?;
    *offset += len;
    Ok(WINDOWS_1252.decode(text).0.into_owned())
}
fn binary_pin(bytes: &[u8]) -> Result<Pin> {
    if bytes.get(..4) != Some(&2_i32.to_le_bytes()) { return Err(invalid()); }
    let mut offset = 12;
    short(bytes, &mut offset)?; // Description.
    let fields = bytes.get(offset..offset+13).ok_or_else(invalid)?;
    let i16_at = |at| i16::from_le_bytes(fields[at..at+2].try_into().unwrap()) as f64;
    let flags = fields[2];
    let length = i16_at(3)*0.254;
    let position = (i16_at(5)*0.254, i16_at(7)*0.254);
    let color = i32::from_le_bytes(fields[9..13].try_into().unwrap());
    offset += 13;
    let name = short(bytes, &mut offset)?;
    let number = short(bytes, &mut offset)?;
    Ok(Pin { position, length, flags, color, name, number, name_font: 1, number_font: 1 })
}
fn read_pin_text(stream: Option<&[u8]>) -> Result<BTreeMap<usize, PinText>> {
    let mut entries = BTreeMap::new();
    let Some(stream) = stream else { return Ok(entries); };
    let mut offset = 0;
    block(stream, &mut offset)?;
    while offset < stream.len() {
        let (_, bytes) = block(stream, &mut offset)?;
        if bytes.first() != Some(&0xd0) { return Err(invalid()); }
        let mut cursor = 1;
        let index = short(bytes, &mut cursor)?.parse::<usize>().map_err(|_| invalid())?;
        let size_bytes = bytes.get(cursor..cursor+4).ok_or_else(invalid)?;
        let size = u32::from_le_bytes(size_bytes.try_into().unwrap()) as usize;
        cursor += 4;
        let compressed = bytes.get(cursor..cursor+size).ok_or_else(invalid)?;
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(compressed).take(15).read_to_end(&mut decoded)?;
        if decoded.len() != 14 || decoded[0] != 0x10 || decoded[7] != 0x10 { return Err(invalid()); }
        entries.insert(index, PinText {
            name_font: u16::from_le_bytes(decoded[1..3].try_into().unwrap()) as usize,
            name_color: i32::from_le_bytes(decoded[3..7].try_into().unwrap()),
            number_font: u16::from_le_bytes(decoded[8..10].try_into().unwrap()) as usize,
            number_color: i32::from_le_bytes(decoded[10..14].try_into().unwrap()),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::altium::{SchColors, schlib};
    use crate::ir::{IrPin, IrRect, PartMeta, SymbolIr};

    fn symbol() -> SymbolIr {
        SymbolIr { name: "RES".into(), description: "10kΩ 电阻".into(), designator: "R?".into(),
            meta: PartMeta { mpn: "0603WAF1002T5E|10k".into(), ..Default::default() },
            pins: vec![IrPin { number: "1".into(), name: "VCC".into(), x: -1.27, y: 0.0,
                rotation: 180.0, pin_type: "power".into(), ..Default::default() },
                IrPin { number: "2".into(), name: "GND".into(), x: 1.27, y: 0.0,
                    show_name: false, show_number: false, ..Default::default() }],
            rects: vec![IrRect { x1: -1.27, y1: -0.61, x2: 1.27, y2: 0.61 }],
            polys: vec![], ellipses: vec![] }
    }

    #[test]
    fn preview_matches_actual_schlib_streams_and_visible_exported_properties() {
        let dir = std::env::temp_dir().join(format!("lceda-sch-preview-{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("RES.SchLib");
        let symbol = symbol();
        let mut custom = SchColors::altium_classic();
        custom.pin_font_size = 13;
        custom.pin_name = SchColors::rgb(24, 120, 80);
        for colors in [SchColors::altium_classic(), SchColors::easyeda(), SchColors::mono(), custom] {
            schlib::write_with_colors(&path, &symbol, colors).unwrap();
            let mut file = cfb::open(&path).unwrap();
            let mut read = |name: &str| {
                let mut bytes = Vec::new();
                file.open_stream(name).unwrap().read_to_end(&mut bytes).unwrap();
                bytes
            };
            let header = read("FileHeader");
            let data = read("RES/Data");
            let pin_text = if colors.follows_easyeda_pins() { Some(read("RES/PinTextData")) } else { None };
            let from_file = SchPreview::from_streams(&header, &data, pin_text.as_deref()).unwrap();
            assert_eq!(from_file, schlib::preview(&symbol, colors).unwrap());
            assert_eq!(from_file.background, colors.background);
            let label = |role| from_file.texts.iter().find(|t| t.role == role).unwrap();
            assert_eq!(label("Designator").text, "R?");
            assert_eq!(label("Designator").color, colors.designator);
            assert_eq!(label("Comment").text, "0603WAF1002T5E_10k");
            assert_eq!(label("Comment").font.name, "Times New Roman");
            assert_eq!(label("Comment").color, colors.comment);
            assert!(label("Designator").position.1 > 0.61);
            assert!(label("Comment").position.1 < -0.61);
            let pin = label("PinName");
            assert_eq!(pin.text, "VCC");
            assert_eq!(pin.font.points, colors.clamped_pin_font_size() as f64);
            assert_eq!(pin.color, colors.pin_style("power", "VCC").name);
            assert_eq!(from_file.texts.iter().filter(|t| t.role.starts_with("Pin")).count(), 2,
                "hidden pin name and number must stay hidden");
            let rect = from_file.strokes.last().unwrap();
            assert!((rect.points[0].1 - (-0.61)).abs() < 0.000003,
                "fractional AD coordinates must survive decoding");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn monochrome_font_size_is_written_and_previewed_and_truncated_streams_are_rejected() {
        let mut colors = SchColors::mono();
        colors.pin_font_size = 14;
        let scene = schlib::preview(&symbol(), colors).unwrap();
        assert_eq!(scene.texts.iter().find(|text| text.role == "PinName").unwrap().font.points, 14.0);
        assert!(SchPreview::from_streams(&[0xff,0xff,0xff,0], &[], None).is_err());
        assert!(SchPreview::from_streams(&[], &[], None).is_err());
    }
}
