use lceda_core::altium::{SchColorScheme, SchColors};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Prefs {
    pub ad_embed_3d: bool,
    pub kicad_attach_3d: bool,
    pub rename_footprint: bool,
    pub batch_merge: bool,
    pub hide_welcome: bool,
    pub lang: Option<String>,
    pub theme: ThemeMode,
    pub always_on_top: bool,
    pub out_dir: Option<String>,
    pub export_step: bool,
    pub export_obj: bool,
    pub export_ad: bool,
    pub export_kicad: bool,
    pub export_pads: bool,
    pub export_datasheet: bool,
    pub export_source: bool,
    pub win_x: Option<f32>,
    pub win_y: Option<f32>,
    pub win_w: f32,
    pub win_h: f32,
    pub win_max: bool,
    pub parts_width: f32,
    pub photo_frac: Option<f32>,
    pub top_frac: Option<f32>,
    pub sch_scheme: SchColorScheme,
    pub sch_custom: SchColors,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            ad_embed_3d: true,
            kicad_attach_3d: true,
            rename_footprint: false,
            batch_merge: false,
            hide_welcome: false,
            lang: None,
            theme: ThemeMode::Auto,
            always_on_top: false,
            out_dir: None,
            export_step: true,
            export_obj: false,
            export_ad: true,
            export_kicad: true,
            export_pads: false,
            export_datasheet: false,
            export_source: false,
            win_x: None,
            win_y: None,
            win_w: 1180.0,
            win_h: 760.0,
            win_max: false,
            parts_width: 340.0,
            photo_frac: None,
            top_frac: None,
            sch_scheme: SchColorScheme::default(),
            sch_custom: SchColors::altium_classic(),
        }
    }
}

pub fn load() -> Prefs {
    let Some(path) = prefs_path() else {
        return Prefs::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Prefs::default();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Prefs::default();
    };
    Prefs {
        ad_embed_3d: v.get("ad_embed_3d").and_then(Value::as_bool).unwrap_or(true),
        kicad_attach_3d: v.get("kicad_attach_3d").and_then(Value::as_bool).unwrap_or(true),
        rename_footprint: v
            .get("rename_footprint")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        batch_merge: v.get("batch_merge").and_then(Value::as_bool).unwrap_or(false),
        hide_welcome: v.get("hide_welcome").and_then(Value::as_bool).unwrap_or(false),
        lang: v
            .get("lang")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        theme: v
            .get("theme")
            .and_then(Value::as_str)
            .map(ThemeMode::parse)
            .unwrap_or(ThemeMode::Auto),
        always_on_top: v
            .get("always_on_top")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        out_dir: v
            .get("out_dir")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        export_step: v.get("export_step").and_then(Value::as_bool).unwrap_or(true),
        export_obj: v.get("export_obj").and_then(Value::as_bool).unwrap_or(false),
        export_ad: v.get("export_ad").and_then(Value::as_bool).unwrap_or(true),
        export_kicad: v.get("export_kicad").and_then(Value::as_bool).unwrap_or(true),
        export_pads: v.get("export_pads").and_then(Value::as_bool).unwrap_or(false),
        export_datasheet: v
            .get("export_datasheet")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        export_source: v.get("export_source").and_then(Value::as_bool).unwrap_or(false),
        win_x: json_f32(&v, "win_x"),
        win_y: json_f32(&v, "win_y"),
        win_w: json_f32(&v, "win_w").unwrap_or(1180.0).clamp(960.0, 4000.0),
        win_h: json_f32(&v, "win_h").unwrap_or(760.0).clamp(620.0, 3000.0),
        win_max: v.get("win_max").and_then(Value::as_bool).unwrap_or(false),
        parts_width: json_f32(&v, "parts_width").unwrap_or(340.0).clamp(200.0, 480.0),
        photo_frac: json_f32(&v, "photo_frac").map(|n| n.clamp(0.15, 0.75)),
        top_frac: json_f32(&v, "top_frac").map(|n| n.clamp(0.20, 0.80)),
        sch_scheme: v
            .get("sch_color")
            .and_then(Value::as_str)
            .map(SchColorScheme::parse)
            .unwrap_or_default(),
        sch_custom: json_sch_colors(v.get("sch_custom")),
    }
}

fn json_sch_colors(v: Option<&Value>) -> SchColors {
    let mut c = SchColors::altium_classic();
    let Some(v) = v else {
        return c;
    };
    if let Some(n) = hex_or_int(v.get("body")) {
        c.body = n;
    }
    if let Some(n) = hex_or_int(v.get("pin")) {
        c.pin = n;
    }
    if let Some(n) = hex_or_int(v.get("pin_name")) {
        c.pin_name = n;
    }
    if let Some(n) = hex_or_int(v.get("pin_number")) {
        c.pin_number = n;
    }
    if let Some(n) = hex_or_int(v.get("designator")) {
        c.designator = n;
    }
    if let Some(n) = hex_or_int(v.get("comment")) {
        c.comment = n;
    }
    c
}

fn hex_or_int(v: Option<&Value>) -> Option<i32> {
    let v = v?;
    if let Some(n) = v.as_i64() {
        return Some(n as i32);
    }
    v.as_str().and_then(SchColors::parse_hex)
}

fn json_f32(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|n| n as f32)
}

pub fn save(prefs: &Prefs) {
    let Some(path) = prefs_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut body = json!({
        "ad_embed_3d": prefs.ad_embed_3d,
        "kicad_attach_3d": prefs.kicad_attach_3d,
        "rename_footprint": prefs.rename_footprint,
        "batch_merge": prefs.batch_merge,
        "hide_welcome": prefs.hide_welcome,
        "theme": prefs.theme.as_str(),
        "always_on_top": prefs.always_on_top,
        "export_step": prefs.export_step,
        "export_obj": prefs.export_obj,
        "export_ad": prefs.export_ad,
        "export_kicad": prefs.export_kicad,
        "export_pads": prefs.export_pads,
        "export_datasheet": prefs.export_datasheet,
        "export_source": prefs.export_source,
        "win_w": prefs.win_w,
        "win_h": prefs.win_h,
        "win_max": prefs.win_max,
        "parts_width": prefs.parts_width,
        "sch_color": prefs.sch_scheme.as_str(),
        "sch_custom": {
            "body": SchColors::as_hex(prefs.sch_custom.body),
            "pin": SchColors::as_hex(prefs.sch_custom.pin),
            "pin_name": SchColors::as_hex(prefs.sch_custom.pin_name),
            "pin_number": SchColors::as_hex(prefs.sch_custom.pin_number),
            "designator": SchColors::as_hex(prefs.sch_custom.designator),
            "comment": SchColors::as_hex(prefs.sch_custom.comment),
        },
    });
    if let Some(f) = prefs.photo_frac {
        body["photo_frac"] = json!(f);
    }
    if let Some(f) = prefs.top_frac {
        body["top_frac"] = json!(f);
    }
    if let Some(lang) = &prefs.lang {
        body["lang"] = json!(lang);
    }
    if let Some(dir) = &prefs.out_dir {
        body["out_dir"] = json!(dir);
    }
    if let Some(x) = prefs.win_x {
        body["win_x"] = json!(x);
    }
    if let Some(y) = prefs.win_y {
        body["win_y"] = json!(y);
    }
    let _ = fs::write(path, body.to_string());
}

fn prefs_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant")
        .map(|d| d.config_dir().join("prefs.json"))
}
