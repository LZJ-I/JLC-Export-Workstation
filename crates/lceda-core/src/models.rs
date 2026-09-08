use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SearchItem {
    pub index: usize,
    pub display_title: String,
    pub title: String,
    pub manufacturer: String,
    pub model_uuid: Option<String>,
    pub raw: Value,
}

impl SearchItem {
    pub fn manufacturer_label(&self) -> String {
        if !self.manufacturer.is_empty() {
            return self.manufacturer.clone();
        }
        manufacturer_from_raw(&self.raw)
    }

    pub fn name(&self) -> &str {
        if !self.display_title.is_empty() {
            &self.display_title
        } else if !self.title.is_empty() {
            &self.title
        } else {
            "component"
        }
    }

    pub fn uuid(&self) -> Option<&str> {
        self.raw.get("uuid").and_then(Value::as_str)
    }

    pub fn image_url(&self) -> Option<String> {
        self.image_urls().into_iter().next()
    }

    /// 预览优先用立创原图，失败再回退搜索接口给的缩略图。
    pub fn image_urls(&self) -> Vec<String> {
        let raw = if let Some(images) = self.raw.get("images").and_then(Value::as_array) {
            images.first().and_then(Value::as_str)
        } else {
            None
        }
        .or_else(|| {
            self.raw
                .get("creator")
                .and_then(|c| c.get("avatar"))
                .and_then(Value::as_str)
        });
        let Some(url) = raw.and_then(normalize_url) else {
            return Vec::new();
        };
        let hd = prefer_source_image_url(&url);
        if hd != url {
            vec![hd, url]
        } else {
            vec![url]
        }
    }

    pub fn symbol_uuid(&self) -> Option<String> {
        nested_uuid(&self.raw, "symbol").or_else(|| attr_string(&self.raw, "Symbol"))
    }

    pub fn footprint_uuid(&self) -> Option<String> {
        nested_uuid(&self.raw, "footprint").or_else(|| attr_string(&self.raw, "Footprint"))
    }

    pub fn has_symbol_or_footprint(&self) -> bool {
        self.symbol_uuid().is_some() || self.footprint_uuid().is_some()
    }

    pub fn lcsc_id(&self) -> Option<String> {
        if let Some(id) = string_or_num(&self.raw, "product_code").and_then(|s| extract_lcsc(&s)) {
            return Some(id);
        }
        const KEYS: &[&str] = &[
            "Supplier Part",
            "LCSC Part",
            "LCSC Part Number",
            "LCSC",
            "Supplier Part Number",
            "JLCPCB Part Number",
            "JLCPCB Part",
        ];
        for key in KEYS {
            if let Some(id) = attr_string(&self.raw, key).and_then(|s| extract_lcsc(&s)) {
                return Some(id);
            }
        }
        None
    }

    /// 器件目录名：`STM32F030C8T6_C23922_ST(意法半导体)`
    pub fn export_stem(&self) -> String {
        crate::util::part_stem(self.name(), self.lcsc_id().as_deref(), &self.manufacturer)
    }

    pub fn datasheet_url(&self) -> Option<String> {
        const KEYS: &[&str] = &["Datasheet", "datasheet", "PDF", "Supplier Datasheet"];
        for key in KEYS {
            if let Some(url) = attr_string(&self.raw, key).and_then(|s| normalize_url(&s)) {
                if url.starts_with("http") {
                    return Some(url);
                }
            }
        }
        None
    }

    pub fn product_url(&self) -> String {
        if let Some(id) = self.datasheet_url().as_deref().and_then(mall_id_from_url) {
            return szlcsc_item_page_url(&id);
        }
        let q = self
            .lcsc_id()
            .unwrap_or_else(|| self.name().to_string());
        szlcsc_search_url(&q)
    }

    pub fn meta(&self) -> crate::ir::PartMeta {
        crate::ir::PartMeta {
            lcsc: self.lcsc_id().unwrap_or_default(),
            mpn: self.name().to_string(),
            manufacturer: self.manufacturer.clone(),
            datasheet: self.datasheet_url().unwrap_or_default(),
            footprint_lib: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DownloadPaths {
    pub folder: Option<PathBuf>,
    pub step: Option<PathBuf>,
    pub obj: Option<PathBuf>,
    pub mtl: Option<PathBuf>,
    pub symbol_json: Option<PathBuf>,
    pub footprint_json: Option<PathBuf>,
    pub schlib: Option<PathBuf>,
    pub pcblib: Option<PathBuf>,
    pub kicad_sym: Option<PathBuf>,
    pub kicad_mod: Option<PathBuf>,
    pub pads_c: Option<PathBuf>,
    pub pads_d: Option<PathBuf>,
    pub pads_p: Option<PathBuf>,
    pub datasheet: Option<PathBuf>,
}

impl DownloadPaths {
    pub fn has_files(&self) -> bool {
        self.step.is_some()
            || self.obj.is_some()
            || self.mtl.is_some()
            || self.datasheet.is_some()
            || self.symbol_json.is_some()
            || self.footprint_json.is_some()
            || self.schlib.is_some()
            || self.pcblib.is_some()
            || self.kicad_sym.is_some()
            || self.kicad_mod.is_some()
            || self.pads_c.is_some()
            || self.pads_d.is_some()
            || self.pads_p.is_some()
    }
}

fn nested_uuid(raw: &Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(|v| v.get("uuid"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn manufacturer_from_raw(raw: &Value) -> String {
    const ATTR_KEYS: &[&str] = &[
        "Manufacturer",
        "manufacturer",
        "Brand",
        "brand",
        "厂牌",
        "品牌",
        "Manufacturer Name",
    ];
    for key in ATTR_KEYS {
        if let Some(s) = attr_string(raw, key) {
            return s;
        }
    }
    for key in ["manufacturer", "brand"] {
        if let Some(s) = string_or_num(raw, key) {
            return s;
        }
    }
    String::new()
}

fn attr_string(raw: &Value, key: &str) -> Option<String> {
    raw.get("attributes")
        .and_then(|a| a.get(key))
        .and_then(json_to_string)
}

fn string_or_num(raw: &Value, key: &str) -> Option<String> {
    raw.get(key).and_then(json_to_string)
}

fn json_to_string(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::to_string)
        .or_else(|| v.as_i64().map(|n| n.to_string()))
        .filter(|s| !s.is_empty())
}

pub fn normalize_url(url: &str) -> Option<String> {
    let value = url.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    Some(value.to_string())
}

/// 搜索接口 `images` 是 `/product/middle/`（约 224px），立创原图在同路径 `/source/`。
pub fn prefer_source_image_url(url: &str) -> String {
    url.replace("/product/middle/", "/product/source/")
}

pub fn join_dir(dir: impl AsRef<Path>, name: &str) -> PathBuf {
    dir.as_ref().join(name)
}

/// 立创商城商品页。`/front/product/view` 是内部接口，浏览器会 403。
/// C 编号不是 `item.szlcsc.com/{数字}.html` 里的数字；那个数字来自数据手册 URL。
pub fn szlcsc_item_page_url(mall_id: &str) -> String {
    format!("https://item.szlcsc.com/{mall_id}.html")
}

pub fn szlcsc_search_url(query: &str) -> String {
    format!(
        "https://so.szlcsc.com/global.html?k={}",
        urlencoding::encode(query.trim())
    )
}

pub fn lcsc_item_page_url(lcsc: &str) -> String {
    szlcsc_search_url(&extract_lcsc(lcsc).unwrap_or_else(|| lcsc.trim().to_string()))
}

/// 立创数据手册页是 HTML。优先取预览 iframe / 「下载PDF」，不要页脚 ISO 或认证附件。
pub fn extract_pdf_url(html: &str) -> Option<String> {
    for (id, attr) in [("myPdfIframe", "src"), ("item-pdf-down", "href")] {
        if let Some(url) = attr_near_id(html, id, attr).and_then(|u| normalize_pdf_url(&u)) {
            return Some(url);
        }
    }
    if let Some(url) = pdf_property_file_url(html).and_then(|u| normalize_pdf_url(&u)) {
        return Some(url);
    }
    scan_atta_pdf(html)
}

fn normalize_pdf_url(raw: &str) -> Option<String> {
    let mut url = raw.replace("&amp;", "&");
    url = url.trim().to_string();
    if let Some(hash) = url.find('#') {
        url.truncate(hash);
    }
    if url.starts_with("//") {
        url = format!("https:{url}");
    } else if url.starts_with("/upload/") {
        url = format!("https://atta.szlcsc.com{url}");
    }
    let lower = url.to_ascii_lowercase();
    if url.starts_with("http") && lower.contains(".pdf") {
        Some(url)
    } else {
        None
    }
}

fn attr_near_id(html: &str, id: &str, attr: &str) -> Option<String> {
    let needles = [format!("id=\"{id}\""), format!("id='{id}'")];
    let pos = needles.iter().find_map(|n| html.find(n))?;
    quoted_attr(char_window(html, pos, 300, 900), attr)
}

fn char_window(s: &str, pos: usize, before: usize, after: usize) -> &str {
    let start = s.floor_char_boundary(pos.saturating_sub(before));
    let end = s.ceil_char_boundary(pos.saturating_add(after).min(s.len()));
    &s[start..end]
}

fn quoted_attr(s: &str, attr: &str) -> Option<String> {
    for q in ['"', '\''] {
        let pat = format!("{attr}={q}");
        if let Some(i) = s.find(&pat) {
            let rest = &s[i + pat.len()..];
            if let Some(j) = rest.find(q) {
                return Some(rest[..j].to_string());
            }
        }
    }
    None
}

fn pdf_property_file_url(html: &str) -> Option<String> {
    let marker = "\"fileType\":\"pdf_property\"";
    let pos = html.find(marker)?;
    let window = char_window(html, pos, 500, 0);
    let key = "\"fileUrl\":\"";
    let i = window.rfind(key)?;
    let rest = &window[i + key.len()..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

fn scan_atta_pdf(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(".pdf") {
        let pdf_end = from + rel + 4;
        let bytes = html.as_bytes();
        let mut url_end = pdf_end;
        if url_end < bytes.len() && matches!(bytes[url_end], b'?' | b'#') {
            let stop = bytes[url_end];
            url_end += 1;
            if stop == b'?' {
                while url_end < bytes.len() {
                    match bytes[url_end] {
                        b'"' | b'\'' | b' ' | b'<' | b'>' | b')' | b'#' | 0..=0x1f => break,
                        _ => url_end += 1,
                    }
                }
            }
        }
        url_end = html.floor_char_boundary(url_end);
        let prefix = &html[..url_end];
        if let Some(start) = ["https://", "http://", "//"]
            .iter()
            .filter_map(|p| prefix.rfind(p))
            .max()
        {
            if url_end - start <= 500 {
                if let Some(url) = normalize_pdf_url(&html[start..url_end]) {
                    let l = url.to_ascii_lowercase();
                    if l.contains("atta.szlcsc.com") && !l.contains("iso_iec") {
                        return Some(url);
                    }
                }
            }
        }
        from = pdf_end;
    }
    None
}

/// 从 `item.szlcsc.com/datasheet/型号/3198300.html` 取出商城数字 ID。
pub fn mall_id_from_url(url: &str) -> Option<String> {
    let after = url.split("item.szlcsc.com/").nth(1)?;
    let path = after.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    let id = last.strip_suffix(".html").unwrap_or(last);
    if id.len() >= 3 && id.bytes().all(|c| c.is_ascii_digit()) {
        Some(id.to_string())
    } else {
        None
    }
}

pub fn extract_lcsc(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'C' || c == b'c' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - i >= 4 {
                let digits = &text[i + 1..j];
                return Some(format!("C{digits}"));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

pub fn looks_like_lcsc_token(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 4 {
        return false;
    }
    let b = t.as_bytes();
    (b[0] == b'C' || b[0] == b'c') && t[1..].bytes().all(|c| c.is_ascii_digit())
}

pub fn parse_id_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_urls_prefer_source_over_middle_thumb() {
        let item = SearchItem {
            index: 1,
            display_title: "BAT54WS".into(),
            title: String::new(),
            manufacturer: String::new(),
            model_uuid: None,
            raw: serde_json::json!({
                "images": [
                    "https://alimg.szlcsc.com/upload/public/product/middle/20230822/3A57432B09985736ABAADF64C6CD16FA.jpg"
                ]
            }),
        };
        assert_eq!(
            item.image_urls(),
            [
                "https://alimg.szlcsc.com/upload/public/product/source/20230822/3A57432B09985736ABAADF64C6CD16FA.jpg",
                "https://alimg.szlcsc.com/upload/public/product/middle/20230822/3A57432B09985736ABAADF64C6CD16FA.jpg",
            ]
        );
        assert_eq!(
            item.image_url().as_deref(),
            Some("https://alimg.szlcsc.com/upload/public/product/source/20230822/3A57432B09985736ABAADF64C6CD16FA.jpg")
        );
    }

    #[test]
    fn extracts_lcsc_numbers() {
        assert_eq!(extract_lcsc("C2040").as_deref(), Some("C2040"));
        assert_eq!(extract_lcsc("part C8755 SOP").as_deref(), Some("C8755"));
        assert!(extract_lcsc("TYPE-C").is_none());
    }

    #[test]
    fn product_page_uses_szlcsc_mall_id_from_datasheet() {
        let item = SearchItem {
            index: 1,
            display_title: "ESP32-S3-WROOM-1-N16R8".into(),
            title: String::new(),
            manufacturer: String::new(),
            model_uuid: None,
            raw: serde_json::json!({
                "product_code": "C2913202",
                "attributes": {
                    "Supplier Part": "C2913202",
                    "Datasheet": "https://item.szlcsc.com/datasheet/ESP32-S3-WROOM-1-N16R8/3198300.html"
                }
            }),
        };
        assert_eq!(
            item.product_url(),
            "https://item.szlcsc.com/3198300.html"
        );
        assert!(!item.product_url().contains("/front/product/view"));
        assert!(!item.product_url().contains("www.lcsc.com"));
        assert!(!item.product_url().contains("item.szlcsc.com/2913202"));
    }

    #[test]
    fn product_page_without_datasheet_uses_cn_search() {
        let item = SearchItem {
            index: 1,
            display_title: "STM32G070RBT6".into(),
            title: String::new(),
            manufacturer: String::new(),
            model_uuid: None,
            raw: serde_json::json!({"attributes": {"Supplier Part": "C529340"}}),
        };
        assert_eq!(
            item.product_url(),
            "https://so.szlcsc.com/global.html?k=C529340"
        );
        assert!(!item.product_url().contains("/front/product/view"));
        assert!(!item.product_url().contains("item.szlcsc.com/529340"));
        assert!(!item.product_url().contains("www.lcsc.com"));
    }

    #[test]
    fn extract_pdf_prefers_iframe_not_iso_or_certs() {
        let html = r#"
<a href="https://static.szlcsc.com/doc/isoiec/last/iso_iec_doc.pdf">ISO/IEC</a>
<iframe id="myPdfIframe" src="https://atta.szlcsc.com/upload/public/pdf/source/20241030/6A412B13087E1327DF6279F5735253CA.pdf#page=1&amp;view=fitH"></iframe>
{"fileName":"乐鑫模组_REACH认证.pdf","fileUrl":"/upload/public/pdf/source/20241029/AB1100BB.pdf"}
"#;
        assert_eq!(
            extract_pdf_url(html).as_deref(),
            Some("https://atta.szlcsc.com/upload/public/pdf/source/20241030/6A412B13087E1327DF6279F5735253CA.pdf")
        );
    }

    #[test]
    fn extract_pdf_iframe_survives_cjk_around_tag() {
        let prefix = "规格书截至目前已更新，请以页面预览为准。".repeat(40);
        let html = format!(
            "{prefix}<iframe id=\"myPdfIframe\" src=\"https://atta.szlcsc.com/upload/public/pdf/source/20220812/8363ACEC51AD55ECEB84799464F7CB85.pdf#page=1&amp;view=fitH\"></iframe>"
        );
        assert_eq!(
            extract_pdf_url(&html).as_deref(),
            Some("https://atta.szlcsc.com/upload/public/pdf/source/20220812/8363ACEC51AD55ECEB84799464F7CB85.pdf")
        );
    }

    #[test]
    fn extract_pdf_from_download_button() {
        let html = r#"<a id="item-pdf-down" href="https://atta.szlcsc.com/upload/public/pdf/source/20220812/8363ACEC51AD55ECEB84799464F7CB85.pdf?x=1&amp;y=2">下载PDF</a>"#;
        assert_eq!(
            extract_pdf_url(html).as_deref(),
            Some("https://atta.szlcsc.com/upload/public/pdf/source/20220812/8363ACEC51AD55ECEB84799464F7CB85.pdf?x=1&y=2")
        );
    }

    #[test]
    fn extract_pdf_from_property_file_url() {
        let html = r#"{"detailVOList":[{"fileName":"规格书.pdf","fileUrl":"/upload/public/pdf/source/20220812/8363ACEC51AD55ECEB84799464F7CB85.pdf"}],"fileType":"pdf_property"}"#;
        assert_eq!(
            extract_pdf_url(html).as_deref(),
            Some("https://atta.szlcsc.com/upload/public/pdf/source/20220812/8363ACEC51AD55ECEB84799464F7CB85.pdf")
        );
    }

    #[test]
    fn mall_id_from_datasheet_and_item_url() {
        assert_eq!(
            mall_id_from_url(
                "https://item.szlcsc.com/datasheet/ESP32-S3-WROOM-1-N16R8/3198300.html"
            )
            .as_deref(),
            Some("3198300")
        );
        assert_eq!(
            mall_id_from_url("https://item.szlcsc.com/3198300.html?fromZone=s_s")
                .as_deref(),
            Some("3198300")
        );
    }

    #[test]
    fn manufacturer_from_raw_reads_brand_and_cjk() {
        let brand = serde_json::json!({"attributes": {"Brand": "STC"}});
        assert_eq!(manufacturer_from_raw(&brand), "STC");
        let cjk = serde_json::json!({"attributes": {"厂牌": "意法半导体"}});
        assert_eq!(manufacturer_from_raw(&cjk), "意法半导体");
        let top = serde_json::json!({"manufacturer": "TI"});
        assert_eq!(manufacturer_from_raw(&top), "TI");
    }

    #[test]
    fn export_stem_uses_mpn_lcsc_mfr() {
        let item = SearchItem {
            index: 1,
            display_title: "STM32F030C8T6".into(),
            title: String::new(),
            manufacturer: "ST(意法半导体)".into(),
            model_uuid: None,
            raw: serde_json::json!({"product_code": "C23922"}),
        };
        assert_eq!(item.lcsc_id().as_deref(), Some("C23922"));
        assert_eq!(
            item.export_stem(),
            "STM32F030C8T6_C23922_ST(意法半导体)"
        );
    }
}
