//! Altium 原理图库颜色。数值是 Windows COLORREF（`0x00BBGGRR`）。
//!
//! 规范依据：
//! - Altium / Protel 原理图库默认图元色为 **128**（暗红 RGB 128,0,0），
//!   位号与型号为 **16711680**（蓝 RGB 0,0,255），管脚名/号默认黑。
//!   这是 AD 自带库和多数手工建库的样子。
//! - 立创 / EasyEDA 导出常见为外形蓝、管脚与管脚字红。
//! - IEEE / 印刷图常用黑白。

/// 一套写入 SchLib 的颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchColors {
    pub body: i32,
    pub pin: i32,
    pub pin_name: i32,
    pub pin_number: i32,
    pub designator: i32,
    pub comment: i32,
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
        }
    }

    /// 立创 / EasyEDA 导出常见蓝框红脚。
    pub const fn easyeda() -> Self {
        Self {
            body: Self::rgb(0, 0, 255),
            pin: Self::rgb(255, 0, 0),
            pin_name: Self::rgb(255, 0, 0),
            pin_number: Self::rgb(255, 0, 0),
            designator: Self::rgb(0, 0, 128),
            comment: Self::rgb(0, 0, 128),
        }
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
        }
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
            "easyeda" | "lceda" | "lcsc" => Self::EasyEda,
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
        assert_eq!(SchColors::to_rgb(128), [128, 0, 0]);
        assert_eq!(SchColors::to_rgb(16_711_680), [0, 0, 255]);
    }

    #[test]
    fn easyeda_matches_blue_body_red_pin() {
        let c = SchColors::easyeda();
        assert_eq!(c.body, 0x00FF_0000);
        assert_eq!(c.pin, 0x0000_00FF);
        assert_eq!(SchColors::to_rgb(c.body), [0, 0, 255]);
        assert_eq!(SchColors::to_rgb(c.pin), [255, 0, 0]);
    }

    #[test]
    fn hex_roundtrip_and_scheme_parse() {
        let c = SchColors::rgb(128, 0, 0);
        assert_eq!(SchColors::as_hex(c), "#800000");
        assert_eq!(SchColors::parse_hex("#800000"), Some(128));
        assert_eq!(SchColors::parse_hex("0000FF"), Some(16_711_680));
        assert_eq!(SchColorScheme::parse("easyeda"), SchColorScheme::EasyEda);
        assert_eq!(SchColorScheme::parse("ieee"), SchColorScheme::Mono);
        assert_eq!(SchColorScheme::parse(""), SchColorScheme::AltiumClassic);
        assert_eq!(
            SchColorScheme::Custom.colors(SchColors::mono()),
            SchColors::mono()
        );
    }

    #[test]
    fn default_scheme_is_altium() {
        assert_eq!(SchColors::default(), SchColors::altium_classic());
        assert_eq!(SchColorScheme::default(), SchColorScheme::AltiumClassic);
    }
}
