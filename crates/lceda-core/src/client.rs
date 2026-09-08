use crate::error::{Error, Result};
use crate::models::SearchItem;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

const SEARCH_API: &str = "https://pro.lceda.cn/api/szlcsc/eda/product/list?wd=";
const COMPONENT_API: &str = "https://pro.lceda.cn/api/components/{uuid}?uuid={uuid}";
pub const STEP_API: &str = "https://modules.lceda.cn/qAxj6KHrDKw4blvCG8QJPs7Y/{uuid}";
pub const OBJ_API: &str = "https://modules.lceda.cn/3dmodel/{uuid}";

#[derive(Clone)]
pub struct LcedaClient {
    agent: ureq::Agent,
}

impl Default for LcedaClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LcedaClient {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(35))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
            .build();
        Self { agent }
    }

    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        self.get_bytes_referer(url, "https://pro.lceda.cn/")
    }

    fn get_bytes_referer(&self, url: &str, referer: &str) -> Result<Vec<u8>> {
        let mut last_err = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(400 * attempt as u64));
            }
            match self
                .agent
                .get(url)
                .set("Accept", "application/json,text/plain,*/*")
                .set("Referer", referer)
                .call()
            {
                Ok(resp) => {
                    let mut buf = Vec::new();
                    resp.into_reader()
                        .take(80 * 1024 * 1024)
                        .read_to_end(&mut buf)
                        .map_err(|e| Error::Http(e.to_string()))?;
                    return Ok(buf);
                }
                Err(err) => last_err = err.to_string(),
            }
        }
        Err(Error::Http(last_err))
    }

    /// 立创「数据手册」常是 HTML 商品页，真正的 PDF 在 atta.szlcsc.com。
    pub fn download_datasheet_pdf(&self, page_url: &str) -> Result<Vec<u8>> {
        let bytes = self.get_bytes_referer(page_url, "https://item.szlcsc.com/")?;
        if bytes.starts_with(b"%PDF") {
            return Ok(bytes);
        }
        let html = String::from_utf8_lossy(&bytes);
        let Some(pdf_url) = crate::models::extract_pdf_url(&html) else {
            return Err(Error::msg("数据手册页里没有 PDF 链接"));
        };
        let pdf = self.get_bytes_referer(&pdf_url, page_url)?;
        if pdf.starts_with(b"%PDF") {
            Ok(pdf)
        } else {
            Err(Error::msg("解析到的数据手册链接不是 PDF"))
        }
    }

    pub fn get_json(&self, url: &str) -> Result<Value> {
        let bytes = self.get_bytes(url)?;
        serde_json::from_slice(&bytes).map_err(Error::from)
    }

    pub fn search(&self, keyword: &str) -> Result<Vec<SearchItem>> {
        let url = format!("{SEARCH_API}{}", urlencoding::encode(keyword));
        let data = self.get_json(&url)?;
        let Some(results) = data.get("result").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        let items = results
            .iter()
            .enumerate()
            .map(|(idx, raw)| SearchItem {
                index: idx + 1,
                display_title: string_field(raw, "display_title"),
                title: string_field(raw, "title"),
                manufacturer: crate::models::manufacturer_from_raw(raw),
                model_uuid: raw
                    .get("attributes")
                    .and_then(|a| a.get("3D Model"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                raw: raw.clone(),
            })
            .collect();
        Ok(rank_search(keyword, items))
    }

    pub fn select(&self, keyword: &str, index: usize) -> Result<SearchItem> {
        let items = self.search(keyword)?;
        if items.is_empty() {
            return Err(Error::NotFound(keyword.to_string()));
        }
        items
            .into_iter()
            .find(|i| i.index == index)
            .ok_or_else(|| Error::msg(format!("无效序号 {index}")))
    }

    pub fn component_json(&self, uuid: &str) -> Result<Value> {
        let url = COMPONENT_API.replace("{uuid}", uuid);
        self.get_json(&url)
    }

    pub fn resolve_model_uuid(&self, item: &SearchItem) -> Result<String> {
        let Some(uuid) = item.model_uuid.as_deref() else {
            return Err(Error::No3dModel);
        };
        let detail = self.component_json(uuid)?;
        if detail.get("code").and_then(Value::as_i64) == Some(0) {
            if let Some(model) = detail
                .pointer("/result/3d_model_uuid")
                .and_then(Value::as_str)
            {
                return Ok(model.to_string());
            }
        }
        Ok(uuid.to_string())
    }

    pub fn download_step_bytes(&self, item: &SearchItem) -> Result<Vec<u8>> {
        let model_uuid = self.resolve_model_uuid(item)?;
        let url = STEP_API.replace("{uuid}", &urlencoding::encode(&model_uuid));
        self.get_bytes(&url)
    }

    pub fn download_obj_bytes(&self, item: &SearchItem) -> Result<Vec<u8>> {
        let model_uuid = self.resolve_model_uuid(item)?;
        let url = OBJ_API.replace("{uuid}", &urlencoding::encode(&model_uuid));
        self.get_bytes(&url)
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn rank_search(keyword: &str, mut items: Vec<crate::models::SearchItem>) -> Vec<crate::models::SearchItem> {
    let kw = keyword.trim();
    if crate::models::looks_like_lcsc_token(kw) {
        let target = crate::models::extract_lcsc(kw);
        items.sort_by_key(|it| {
            let id = it.lcsc_id();
            if id.as_ref() == target.as_ref() {
                0u8
            } else if it.name().eq_ignore_ascii_case(kw) {
                1
            } else {
                2
            }
        });
        for (i, it) in items.iter_mut().enumerate() {
            it.index = i + 1;
        }
    }
    items
}
