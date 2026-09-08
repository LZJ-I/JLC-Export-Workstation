//! 收款码只从 [LZJ-I/sponsor](https://github.com/LZJ-I/sponsor) 读取。
//! GitHub 仓库的 FUNDING.yml 也只挂这一处，避免各项目各放一份码。

use crate::update;
use serde_json::Value;

pub const REPO: &str = "LZJ-I/sponsor";
pub const PAGE_URL: &str = "https://github.com/LZJ-I/sponsor";
const MANIFEST_URL: &str = "https://raw.githubusercontent.com/LZJ-I/sponsor/main/sponsor.json";

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: String,
    pub title_zh: String,
    pub title_en: String,
    pub url: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Pack {
    #[allow(dead_code)]
    pub hint_zh: String,
    #[allow(dead_code)]
    pub hint_en: String,
    #[allow(dead_code)]
    pub page: String,
    pub channels: Vec<Channel>,
}

pub fn load() -> Result<Pack, String> {
    let bytes = get_bytes(MANIFEST_URL).ok_or_else(|| "无法读取赞助配置".to_string())?;
    let json: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("赞助配置不是 JSON: {e}"))?;
    parse_pack(&json)
}

pub fn title(channel: &Channel, zh: bool) -> &str {
    if zh {
        &channel.title_zh
    } else {
        &channel.title_en
    }
}

pub fn fetch_image(rel: &str) -> Option<Vec<u8>> {
    let url = resolve_asset(rel);
    get_bytes(&url).filter(|b| b.len() > 32 && b.len() < 2 * 1024 * 1024)
}

fn parse_pack(json: &Value) -> Result<Pack, String> {
    let channels = json
        .get("channels")
        .and_then(Value::as_array)
        .ok_or_else(|| "赞助配置缺少 channels".to_string())?;
    let mut out = Vec::new();
    for ch in channels {
        let id = json_str(ch, "id").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        out.push(Channel {
            id,
            title_zh: json_str(ch, "title_zh").unwrap_or_else(|| "赞助".into()),
            title_en: json_str(ch, "title_en").unwrap_or_else(|| "Sponsor".into()),
            url: json_str(ch, "url").filter(|s| s.starts_with("http")),
            image: json_str(ch, "image"),
        });
    }
    if out.is_empty() {
        return Err("赞助配置是空的".into());
    }
    Ok(Pack {
        hint_zh: json_str(json, "hint_zh")
            .unwrap_or_else(|| "自愿赞助，不是购买软件。软件始终免费。".into()),
        hint_en: json_str(json, "hint_en")
            .unwrap_or_else(|| "Optional tip. The software stays free.".into()),
        page: json_str(json, "page").unwrap_or_else(|| PAGE_URL.into()),
        channels: out,
    })
}

fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn resolve_asset(rel: &str) -> String {
    let rel = rel.trim();
    if rel.starts_with("http://") || rel.starts_with("https://") {
        rel.to_string()
    } else {
        format!(
            "https://raw.githubusercontent.com/{REPO}/main/{}",
            rel.trim_start_matches('/')
        )
    }
}

fn get_bytes(url: &str) -> Option<Vec<u8>> {
    let proxied = update::via_proxy(url);
    update::fetch_bytes(&proxied).or_else(|| update::fetch_bytes(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_relative_and_absolute() {
        assert_eq!(
            resolve_asset("wechat.webp"),
            "https://raw.githubusercontent.com/LZJ-I/sponsor/main/wechat.webp"
        );
        assert_eq!(
            resolve_asset("https://example.com/a.webp"),
            "https://example.com/a.webp"
        );
    }

    #[test]
    fn parse_three_channels() {
        let json = serde_json::json!({
            "hint_zh": "自愿",
            "hint_en": "optional",
            "page": "https://github.com/LZJ-I/sponsor",
            "channels": [
                { "id": "wechat", "title_zh": "微信", "title_en": "WeChat", "image": "wechat.webp" },
                { "id": "alipay", "title_zh": "支付宝", "title_en": "Alipay", "image": "alipay.webp" },
                { "id": "afdian", "title_zh": "爱发电", "title_en": "Afdian", "url": "https://afdian.com/a/demo" }
            ]
        });
        let pack = parse_pack(&json).unwrap();
        assert_eq!(pack.channels.len(), 3);
        assert_eq!(pack.channels[0].image.as_deref(), Some("wechat.webp"));
        assert_eq!(pack.channels[2].url.as_deref(), Some("https://afdian.com/a/demo"));
        assert!(pack.channels[2].image.is_none());
    }
}
