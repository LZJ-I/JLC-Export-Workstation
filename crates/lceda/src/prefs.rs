use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Prefs {
    pub ad_embed_3d: bool,
    pub kicad_attach_3d: bool,
    pub batch_merge: bool,
    pub lang: Option<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            ad_embed_3d: true,
            kicad_attach_3d: true,
            batch_merge: false,
            lang: None,
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
        batch_merge: v.get("batch_merge").and_then(Value::as_bool).unwrap_or(false),
        lang: v
            .get("lang")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
    }
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
        "batch_merge": prefs.batch_merge,
    });
    if let Some(lang) = &prefs.lang {
        body["lang"] = json!(lang);
    }
    let _ = fs::write(path, body.to_string());
}

fn prefs_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant")
        .map(|d| d.config_dir().join("prefs.json"))
}
