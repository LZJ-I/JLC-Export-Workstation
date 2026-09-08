//! Altium 原理图库颜色。数值是 Windows COLORREF（`0x00BBGGRR`）。
//!
//! 规范依据（立创商城 `lceda.cn/api/products/{id}/svgs` 官方预览，抽样 IC / LDO / MCU）：
//! - **默认 Altium 经典**：外形/管脚暗红 128，管脚字黑，位号/型号蓝。
//! - **立创官方**：外壳 `#880000`；普通脚（输入/输出/IO/未定义）线暗红、字蓝 `#0000FF`；
//!   电源线+字 `#FF0000`；地线+字 `#000000`。输入和输出没有第三套色。
//! - IEEE / 印刷图常用黑白。

/// 一套写入 SchLib 的底色。立创官方方案还会按管脚电气类型改线/字色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchColors {
    pub body: i32,
    pub pin: i32,
    pub pin_name: i32,
    pub pin_number: i32,
    pub designator: i32,
    pub comment: i32,
    /// 管脚名/号字号（Altium 点）。立创官方 7，经典 10。
    pub pin_font_size: i32,
}

pub const PIN_FONT_SIZE_MIN: i32 = 6;
pub const PIN_FONT_SIZE_MAX: i32 = 16;

/// 单只管脚的线色、名色、号色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinStyle {
    pub line: i32,
    pub name: i32,
    pub number: i32,
}

impl PinStyle {
    pub const fn solid(c: i32) -> Self {
        Self {
            line: c,
            name: c,
            number: c,
        }
    }

    pub fn uses_local_font(self) -> bool {
        self.name != self.line || self.number != self.line
    }
}

/// 立创官方预览里真正分色的只有电源 / 地；其余电气类型同一套。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EasyedaPinKind {
    Signal,
    Power,
    Ground,
}

impl Default for SchColors {
    fn default() -> Self {
        Self::altium_classic()
    }
}

impl SchColors {
    /// `r + (g << 8) + (b << 16)`
    pub const fn rgb(r: u8, g: u8, b: u8) -> i32 {
        r as i32 | ((g as i32) << 8) | ((b as i32) << 16)
    }

    pub const fn to_rgb(c: i32) -> [u8; 3] {
        [
            (c & 0xFF) as u8,
            ((c >> 8) & 0xFF) as u8,
            ((c >> 16) & 0xFF) as u8,
        ]
    }

    pub const fn from_rgb(rgb: [u8; 3]) -> i32 {
        Self::rgb(rgb[0], rgb[1], rgb[2])
    }

    /// Altium 经典：外形/管脚暗红，位号/型号蓝，管脚字黑。
    pub const fn altium_classic() -> Self {
        Self {
            body: Self::rgb(128, 0, 0),
            pin: Self::rgb(128, 0, 0),
            pin_name: Self::rgb(0, 0, 0),
            pin_number: Self::rgb(0, 0, 0),
            designator: Self::rgb(0, 0, 255),
            comment: Self::rgb(0, 0, 255),
            pin_font_size: 10,
        }
    }

    /// 立创商城官方符号预览。普通脚线 `#880000`、字 `#0000FF`（SchLib 走 PinTextData）；
    /// 电源整脚红、地整脚黑。外壳 `#880000`。管脚字统一 Verdana 7。
    pub const fn easyeda() -> Self {
        Self {
            body: Self::rgb(0x88, 0, 0),
            pin: Self::rgb(0x88, 0, 0),
            pin_name: Self::rgb(0, 0, 255),
            pin_number: Self::rgb(0, 0, 255),
            designator: Self::rgb(0, 0, 128),
            comment: Self::rgb(0, 0, 128),
            pin_font_size: 7,
        }
    }

    pub const fn easyeda_power() -> i32 {
        Self::rgb(255, 0, 0)
    }

    pub const fn easyeda_ground() -> i32 {
        0
    }

    /// 黑白，适合打印或 IEEE 风格图纸。
    pub const fn mono() -> Self {
        Self {
            body: 0,
            pin: 0,
            pin_name: 0,
            pin_number: 0,
            designator: 0,
            comment: 0,
            pin_font_size: 10,
        }
    }

    pub fn clamped_pin_font_size(self) -> i32 {
        self.pin_font_size.clamp(PIN_FONT_SIZE_MIN, PIN_FONT_SIZE_MAX)
    }

    pub fn pin_font_name(self) -> &'static str {
        if self.follows_easyeda_pins() {
            "Verdana"
        } else {
            "Times New Roman"
        }
    }

    /// 脚线/脚字是立创官方那套时，按电源/地分色。字号不参与比较。
    pub fn follows_easyeda_pins(self) -> bool {
        let e = Self::easyeda();
        self.pin == e.pin && self.pin_name == e.pin_name && self.pin_number == e.pin_number
    }

    pub fn as_hex(c: i32) -> String {
        let [r, g, b] = Self::to_rgb(c);
        format!("#{r:02X}{g:02X}{b:02X}")
    }

    pub fn parse_hex(s: &str) -> Option<i32> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let n = u32::from_str_radix(s, 16).ok()?;
        Some(Self::rgb(
            ((n >> 16) & 0xFF) as u8,
            ((n >> 8) & 0xFF) as u8,
            (n & 0xFF) as u8,
        ))
    }

    /// 立创官方方案按电气类型改色；其它方案用这一套固定色。
    pub fn pin_style(self, pin_type: &str, pin_name: &str) -> PinStyle {
        if self.follows_easyeda_pins() {
            return easyeda_pin_style(pin_type, pin_name);
        }
        PinStyle {
            line: self.pin,
            name: self.pin_name,
            number: self.pin_number,
        }
    }

    /// FileHeader 里 FONT2 起的管脚字颜色（顺序即 FONTID）。同一字号，颜色分槽。
    pub fn pin_text_fonts(self) -> Vec<i32> {
        let mut fonts = Vec::new();
        let mut push = |c: i32| {
            if !fonts.contains(&c) {
                fonts.push(c);
            }
        };
        push(self.pin_name);
        push(self.pin_number);
        if self.follows_easyeda_pins() {
            push(Self::easyeda_power());
            push(Self::easyeda_ground());
        }
        fonts
    }

    pub fn font_id_for(self, color: i32) -> i32 {
        self.pin_text_fonts()
            .iter()
            .position(|&c| c == color)
            .map(|i| (i + 2) as i32)
            .unwrap_or(1)
    }
}

pub fn easyeda_pin_style(pin_type: &str, pin_name: &str) -> PinStyle {
    match classify_easyeda_pin(pin_type, pin_name) {
        EasyedaPinKind::Power => PinStyle::solid(SchColors::easyeda_power()),
        EasyedaPinKind::Ground => PinStyle::solid(SchColors::easyeda_ground()),
        EasyedaPinKind::Signal => {
            let c = SchColors::easyeda();
            PinStyle {
                line: c.pin,
                name: c.pin_name,
                number: c.pin_number,
            }
        }
    }
}

/// 对照官方 SVG：IN/OUT/IO/Passive/NC/Undefined 都是暗红线+蓝字。
/// 电源/地优先看库里的 Pin Type；多数立创库写成 Undefined，再按管脚名补。
pub fn classify_easyeda_pin(pin_type: &str, pin_name: &str) -> EasyedaPinKind {
    let t = pin_type.trim().to_ascii_uppercase();
    if is_power_type(&t) {
        return EasyedaPinKind::Power;
    }
    if is_ground_type(&t) {
        return EasyedaPinKind::Ground;
    }
    if !t.is_empty() && t != "UNDEFINED" && t != "UNSPECIFIED" {
        return EasyedaPinKind::Signal;
    }
    infer_kind_from_name(pin_name)
}

fn is_power_type(t: &str) -> bool {
    matches!(
        t,
        "POWER" | "PWR" | "POWER_IN" | "POWER_OUT" | "POWERIN" | "POWEROUT"
    )
}

fn is_ground_type(t: &str) -> bool {
    matches!(t, "GROUND" | "GND" | "EARTH")
}

fn infer_kind_from_name(name: &str) -> EasyedaPinKind {
    let n = name
        .trim()
        .trim_start_matches('#')
        .to_ascii_uppercase()
        .replace([' ', '-'], "_");
    let stem = n.split('_').next().unwrap_or(&n);
    // 官方红：VCC / VDD / VDDA / V+（VDD_1 这类）。VIN/VOUT/VBUS/IOVDD/DVDD 官方常当信号。
    if matches!(stem, "VCC" | "VDD" | "VDDA" | "AVCC" | "V+" | "VBAT") || n == "V+" {
        return EasyedaPinKind::Power;
    }
    if matches!(stem, "GND" | "VSS" | "VSSA" | "AGND" | "DGND" | "PGND") {
        return EasyedaPinKind::Ground;
    }
    EasyedaPinKind::Signal
}

/// 设置里的配色方案。`Custom` 使用用户保存的 `SchColors`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchColorScheme {
    AltiumClassic,
    EasyEda,
    Mono,
    Custom,
}

impl Default for SchColorScheme {
    fn default() -> Self {
        Self::AltiumClassic
    }
}

impl SchColorScheme {
    pub const ALL: [Self; 4] = [Self::AltiumClassic, Self::EasyEda, Self::Mono, Self::Custom];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AltiumClassic => "altium",
            Self::EasyEda => "easyeda",
            Self::Mono => "mono",
            Self::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "easyeda" | "jlc" | "lceda" | "official" => Self::EasyEda,
            "mono" | "bw" | "ieee" | "print" => Self::Mono,
            "custom" => Self::Custom,
            _ => Self::AltiumClassic,
        }
    }

    pub fn colors(self, custom: SchColors) -> SchColors {
        match self {
            Self::AltiumClassic => SchColors::altium_classic(),
            Self::EasyEda => SchColors::easyeda(),
            Self::Mono => SchColors::mono(),
            Self::Custom => custom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altium_classic_matches_ad_defaults() {
        let c = SchColors::altium_classic();
        assert_eq!(c.body, 128, "AD 默认图元色 COLOR=128");
        assert_eq!(c.pin, 128);
        assert_eq!(c.pin_name, 0);
        assert_eq!(c.pin_number, 0);
        assert_eq!(c.designator, 16_711_680);
        assert_eq!(c.comment, 16_711_680);
        assert_eq!(c.pin_font_size, 10);
        assert_eq!(c.pin_font_name(), "Times New Roman");
        assert_eq!(SchColors::to_rgb(128), [128, 0, 0]);
        assert_eq!(SchColors::to_rgb(16_711_680), [0, 0, 255]);
    }

    #[test]
    fn easyeda_matches_official_svg() {
        let c = SchColors::easyeda();
        assert_eq!(c.body, 0x88);
        assert_eq!(c.pin, 0x88);
        assert_eq!(c.pin_name, 16_711_680);
        assert_eq!(SchColors::to_rgb(c.body), [0x88, 0, 0]);
        assert_eq!(SchColors::as_hex(c.body), "#880000");
        assert_eq!(SchColors::as_hex(c.pin_name), "#0000FF");
        assert_eq!(SchColors::as_hex(SchColors::easyeda_power()), "#FF0000");
        assert_eq!(c.pin_font_size, 7);
        assert_eq!(c.pin_font_name(), "Verdana");
        assert_eq!(
            c.pin_text_fonts(),
            vec![16_711_680, 255, 0],
            "蓝 / 电源红 / 地黑，同一字号"
        );
        assert!(c.pin_style("", "S").uses_local_font());
        let mut sized = c;
        sized.pin_font_size = 12;
        assert!(sized.follows_easyeda_pins());
        assert_eq!(sized.clamped_pin_font_size(), 12);
    }

    #[test]
    fn official_svg_only_power_and_ground_differ() {
        let sig = easyeda_pin_style("", "S");
        assert_eq!(
            sig,
            PinStyle {
                line: 0x88,
                name: 16_711_680,
                number: 16_711_680,
            }
        );
        assert_eq!(easyeda_pin_style("IN", "GPIO0").name, 16_711_680);
        assert_eq!(easyeda_pin_style("OUT", "TXD").name, 16_711_680);
        assert_eq!(easyeda_pin_style("BI", "D+").name, 16_711_680);
        assert_eq!(easyeda_pin_style("PASSIVE", "NC").name, 16_711_680);
        assert_eq!(easyeda_pin_style("", "VIN").name, 16_711_680);
        assert_eq!(easyeda_pin_style("", "VOUT").name, 16_711_680);
        assert_eq!(easyeda_pin_style("", "VBUS").name, 16_711_680);
        assert_eq!(easyeda_pin_style("", "IOVDD").name, 16_711_680);
        assert_eq!(easyeda_pin_style("", "V+"), PinStyle::solid(255));
        assert_eq!(easyeda_pin_style("Power", "VIN"), PinStyle::solid(255));
        assert_eq!(easyeda_pin_style("", "VDD_1"), PinStyle::solid(255));
        assert_eq!(easyeda_pin_style("", "GND"), PinStyle::solid(0));
        assert_eq!(easyeda_pin_style("", "VSSA"), PinStyle::solid(0));
        assert_eq!(easyeda_pin_style("Ground", "EP"), PinStyle::solid(0));
    }

    #[test]
    fn hex_roundtrip_and_scheme_parse() {
        let c = SchColors::rgb(128, 0, 0);
        assert_eq!(SchColors::as_hex(c), "#800000");
        assert_eq!(SchColors::parse_hex("#800000"), Some(128));
        assert_eq!(SchColors::parse_hex("0000FF"), Some(16_711_680));
        assert_eq!(SchColorScheme::parse("easyeda"), SchColorScheme::EasyEda);
        assert_eq!(SchColorScheme::parse("jlc"), SchColorScheme::EasyEda);
        assert_eq!(SchColorScheme::parse("ieee"), SchColorScheme::Mono);
        assert_eq!(SchColorScheme::parse("altium"), SchColorScheme::AltiumClassic);
        assert_eq!(SchColorScheme::parse(""), SchColorScheme::AltiumClassic);
        assert_eq!(
            SchColorScheme::Custom.colors(SchColors::mono()),
            SchColors::mono()
        );
    }

    #[test]
    fn default_scheme_is_altium_classic() {
        assert_eq!(SchColors::default(), SchColors::altium_classic());
        assert_eq!(SchColorScheme::default(), SchColorScheme::AltiumClassic);
    }
}
