//! Altium Description / 详细信息共用模板。

use crate::models::{self, SearchItem};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescField {
    PartName,
    Description,
    Mpn,
    Lcsc,
    Manufacturer,
    Package,
}

impl DescField {
    pub const ALL: [Self; 6] = [
        Self::PartName,
        Self::Description,
        Self::Mpn,
        Self::Lcsc,
        Self::Manufacturer,
        Self::Package,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::PartName => "part_name",
            Self::Description => "description",
            Self::Mpn => "mpn",
            Self::Lcsc => "lcsc",
            Self::Manufacturer => "manufacturer",
            Self::Package => "package",
        }
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::PartName => "desc_field_part_name",
            Self::Description => "desc_field_description",
            Self::Mpn => "desc_field_mpn",
            Self::Lcsc => "desc_field_lcsc",
            Self::Manufacturer => "desc_field_manufacturer",
            Self::Package => "desc_field_package",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "part_name" => Some(Self::PartName),
            "description" => Some(Self::Description),
            "mpn" => Some(Self::Mpn),
            "lcsc" => Some(Self::Lcsc),
            "manufacturer" => Some(Self::Manufacturer),
            "package" => Some(Self::Package),
            _ => None,
        }
    }

    pub fn library() -> Vec<Self> {
        vec![
            Self::PartName,
            Self::Description,
            Self::Mpn,
            Self::Lcsc,
            Self::Manufacturer,
        ]
    }

    pub fn specs_only() -> Vec<Self> {
        vec![Self::Description]
    }

    pub fn ids_only() -> Vec<Self> {
        vec![Self::Mpn, Self::Lcsc, Self::Manufacturer]
    }
}

pub fn parse_fields(ids: &[String]) -> Vec<DescField> {
    let mut out = Vec::new();
    for id in ids {
        if let Some(f) = DescField::parse(id) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    if out.is_empty() {
        DescField::library()
    } else {
        out
    }
}

pub fn field_ids(fields: &[DescField]) -> Vec<String> {
    fields.iter().map(|f| f.id().to_string()).collect()
}

pub fn preset_of(fields: &[DescField]) -> Option<&'static str> {
    if fields == DescField::library() {
        Some("library")
    } else if fields == DescField::specs_only() {
        Some("specs")
    } else if fields == DescField::ids_only() {
        Some("ids")
    } else {
        None
    }
}

impl SearchItem {
    pub fn part_short_name(&self) -> String {
        crate::models::attr_string(&self.raw, "LCSC Part Name").unwrap_or_default()
    }

    pub fn package(&self) -> String {
        crate::models::attr_string(&self.raw, "Supplier Footprint").unwrap_or_default()
    }

    pub fn spec_description(&self) -> String {
        let raw = crate::models::string_or_num(&self.raw, "description").unwrap_or_default();
        crate::models::format_product_description(&raw)
    }

    /// 立创 `Designator` / `pre`，没有则 `U?`。
    pub fn sch_designator(&self) -> String {
        first_designator(&[
            crate::models::attr_string(&self.raw, "Designator"),
            crate::models::attr_string(&self.raw, "pre"),
        ])
    }

    pub fn render_description(&self, fields: &[DescField]) -> String {
        let mut lines = Vec::new();
        for field in fields {
            let text = self.field_text(*field);
            if !text.is_empty() {
                lines.push(text);
            }
        }
        if lines.is_empty() {
            self.name().to_string()
        } else {
            lines.join("\n")
        }
    }

    fn field_text(&self, field: DescField) -> String {
        match field {
            DescField::PartName => self.part_short_name(),
            DescField::Description => self.spec_description(),
            DescField::Mpn => {
                let n = self.name();
                if n.is_empty() || n == "component" {
                    String::new()
                } else {
                    format!("型号:{n}")
                }
            }
            DescField::Lcsc => self
                .lcsc_id()
                .map(|id| format!("LCSC {id}"))
                .unwrap_or_default(),
            DescField::Manufacturer => {
                let m = self.manufacturer_label();
                if m.is_empty() {
                    String::new()
                } else {
                    format!("厂牌:{m}")
                }
            }
            DescField::Package => {
                let p = self.package();
                if p.is_empty() {
                    String::new()
                } else {
                    format!("封装:{p}")
                }
            }
        }
    }

    pub fn example_ch343p() -> Self {
        Self {
            index: 1,
            display_title: "CH343P".into(),
            title: "CH343P".into(),
            manufacturer: "WCH(南京沁恒)".into(),
            model_uuid: None,
            raw: serde_json::json!({
                "product_code": "C2846043",
                "description": "应用功能:USB转UART;USB协议版本:USB 2.0;通道数:-;数据速率:6Mbps;",
                "attributes": {
                    "LCSC Part Name": "USB转高速串口芯片",
                    "Supplier Footprint": "TQFN-16-EP(3x3)",
                    "Designator": "U?",
                    "Manufacturer Part": "CH343P"
                }
            }),
        }
    }
}

pub fn parse_designator(raw: &str) -> Option<String> {
    let s = raw.trim().replace('？', "?");
    let core = s.trim_end_matches('?').trim();
    if core.is_empty()
        || core.len() > 8
        || !core.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(format!("{core}?"))
}

pub fn normalize_designator(raw: &str) -> String {
    parse_designator(raw).unwrap_or_else(|| "U?".into())
}

/// 第一个合法位号前缀；型号或空值跳过，最后兜底 `U?`。
pub fn first_designator(candidates: &[Option<String>]) -> String {
    for raw in candidates {
        if let Some(s) = raw.as_deref().and_then(parse_designator) {
            return s;
        }
    }
    "U?".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropGroup {
    General,
    Identity,
    Specs,
    Extra,
}

impl PropGroup {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::General => "prop_group_general",
            Self::Identity => "prop_group_identity",
            Self::Specs => "prop_group_specs",
            Self::Extra => "prop_group_extra",
        }
    }

    pub fn hint_key(self) -> &'static str {
        match self {
            Self::General => "prop_group_general_hint",
            Self::Identity => "prop_group_identity_hint",
            Self::Specs => "prop_group_specs_hint",
            Self::Extra => "prop_group_extra_hint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropRow {
    pub group: PropGroup,
    pub name: String,
    pub value: String,
}

const SKIP_ATTRS: &[&str] = &[
    "Symbol",
    "Footprint",
    "3D Model",
    "3D Model Title",
    "3D Model Transform",
    "Add into BOM",
    "Convert to PCB",
    "Name",
    "Designator",
    "Supplier",
    "Supplier Part",
    "Manufacturer",
    "Manufacturer Part",
    "Supplier Footprint",
    "LCSC Part Name",
    "Datasheet",
];

const SPEC_ATTRS: &[(&str, &str)] = &[
    ("Pins Structure", "插针结构"),
    ("Pitch", "间距"),
    ("Mounting Type", "安装方式"),
    ("Reference Series", "参考系列"),
    ("Number of Pins", "总PIN数"),
    ("Number of Rows", "排数"),
    ("Number of PINs Per Row", "每排PIN数"),
    ("Row Spacing", "行距"),
    ("Current Rating", "额定电流"),
    ("Contact Material", "触头材质"),
    ("Contact Plating", "触头镀层"),
    ("Operating Temperature", "工作温度"),
    ("Flame Retardant Rating", "阻燃等级"),
    ("Color", "颜色"),
];

impl SearchItem {
    pub fn spec_pairs(&self) -> Vec<(String, String)> {
        parse_spec_pairs(&self.spec_description())
    }

    pub fn category_label(&self) -> String {
        let tags = self.raw.get("tags");
        let parent = tags
            .and_then(|t| t.get("parent_tag"))
            .map(tag_name)
            .unwrap_or_default();
        let child = tags
            .and_then(|t| t.get("child_tag"))
            .map(tag_name)
            .unwrap_or_default();
        match (parent.is_empty(), child.is_empty()) {
            (true, true) => String::new(),
            (false, true) => parent,
            (true, false) => child,
            (false, false) => format!("{parent} / {child}"),
        }
    }

    /// AD 属性面板行。预览只给常规三栏；`full` 再带编号 / 规格 / 其它。
    pub fn ad_properties(&self, fields: &[DescField], full: bool) -> Vec<PropRow> {
        let mut rows = Vec::new();
        push_row(
            &mut rows,
            PropGroup::General,
            "Designator",
            self.sch_designator(),
        );
        let comment = self.name();
        if !comment.is_empty() && comment != "component" {
            push_row(&mut rows, PropGroup::General, "Comment", comment);
        }
        push_row(
            &mut rows,
            PropGroup::General,
            "Description",
            self.render_description(fields),
        );

        if !full {
            return rows;
        }

        push_row(&mut rows, PropGroup::Identity, "短名", self.part_short_name());
        if !comment.is_empty() && comment != "component" {
            push_row(&mut rows, PropGroup::Identity, "型号", comment);
        }
        if let Some(id) = self.lcsc_id() {
            push_row(&mut rows, PropGroup::Identity, "LCSC", id);
        }
        push_row(
            &mut rows,
            PropGroup::Identity,
            "厂牌",
            self.manufacturer_label(),
        );
        push_row(&mut rows, PropGroup::Identity, "封装", self.package());
        push_row(&mut rows, PropGroup::Identity, "分类", self.category_label());

        let pairs = self.spec_pairs();
        if pairs.is_empty() {
            for (key, label) in SPEC_ATTRS {
                if let Some(v) = models::attr_string(&self.raw, key) {
                    push_row(&mut rows, PropGroup::Specs, *label, v);
                }
            }
        } else {
            for (k, v) in pairs {
                push_row(&mut rows, PropGroup::Specs, k, v);
            }
        }

        let seen: Vec<String> = rows.iter().map(|r| r.value.clone()).collect();
        for (key, label) in SPEC_ATTRS {
            if let Some(v) = models::attr_string(&self.raw, key) {
                if !seen.iter().any(|s| s == &v) && !rows.iter().any(|r| r.name == *label) {
                    push_row(&mut rows, PropGroup::Extra, *label, v);
                }
            }
        }
        if let Some(class) = models::attr_string(&self.raw, "JLCPCB Part Class") {
            push_row(&mut rows, PropGroup::Extra, "嘉立创分类", class);
        }
        for (k, v) in iter_attrs(&self.raw) {
            if SKIP_ATTRS.contains(&k.as_str())
                || SPEC_ATTRS.iter().any(|(en, _)| *en == k)
                || k == "JLCPCB Part Class"
                || looks_like_uuid(&v)
            {
                continue;
            }
            if rows.iter().any(|r| r.name == k || r.value == v) {
                continue;
            }
            push_row(&mut rows, PropGroup::Extra, k, v);
        }
        if let Some(url) = self.datasheet_url() {
            push_row(&mut rows, PropGroup::Extra, "数据手册", url);
        }
        rows
    }
}

pub fn parse_spec_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        if k.is_empty() || k.chars().count() > 24 || k.contains(' ') {
            continue;
        }
        out.push((k.to_string(), v.to_string()));
    }
    out
}

fn push_row(rows: &mut Vec<PropRow>, group: PropGroup, name: impl Into<String>, value: impl Into<String>) {
    let value = value.into();
    let name = name.into();
    if value.trim().is_empty() {
        return;
    }
    if rows.iter().any(|r| r.group == group && r.name == name && r.value == value) {
        return;
    }
    rows.push(PropRow { group, name, value });
}

fn tag_name(v: &Value) -> String {
    v.get("name_cn")
        .and_then(Value::as_str)
        .or_else(|| v.get("name").and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn iter_attrs(raw: &Value) -> Vec<(String, String)> {
    let Some(obj) = raw.get("attributes").and_then(Value::as_object) else {
        return Vec::new();
    };
    obj.iter()
        .filter_map(|(k, v)| {
            let s = v
                .as_str()
                .map(str::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
                .or_else(|| v.as_f64().map(|n| n.to_string()))?;
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            Some((k.clone(), s.to_string()))
        })
        .collect()
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> SearchItem {
        SearchItem::example_ch343p()
    }

    #[test]
    fn field_ids_roundtrip_keeps_order() {
        let ids = ["lcsc", "package", "mpn", "description"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let fields = parse_fields(&ids);
        assert_eq!(field_ids(&fields), ids);
        assert_eq!(
            parse_fields(&["bogus".into(), "mpn".into(), "mpn".into(), "lcsc".into()]),
            vec![DescField::Mpn, DescField::Lcsc]
        );
    }

    #[test]
    fn library_template_skips_unchecked_package() {
        let text = item().render_description(&DescField::library());
        assert_eq!(
            text,
            "USB转高速串口芯片\n应用功能:USB转UART\nUSB协议版本:USB 2.0\n通道数:-\n数据速率:6Mbps\n型号:CH343P\nLCSC C2846043\n厂牌:WCH(南京沁恒)"
        );
        assert!(!text.contains("封装"));
    }

    #[test]
    fn specs_and_ids_presets() {
        assert_eq!(
            item().render_description(&DescField::specs_only()),
            "应用功能:USB转UART\nUSB协议版本:USB 2.0\n通道数:-\n数据速率:6Mbps"
        );
        assert_eq!(
            item().render_description(&DescField::ids_only()),
            "型号:CH343P\nLCSC C2846043\n厂牌:WCH(南京沁恒)"
        );
    }

    #[test]
    fn empty_fields_are_skipped() {
        let bare = SearchItem {
            index: 1,
            display_title: "RS2227XN".into(),
            title: String::new(),
            manufacturer: String::new(),
            model_uuid: None,
            raw: serde_json::json!({}),
        };
        assert_eq!(
            bare.render_description(&DescField::library()),
            "型号:RS2227XN"
        );
    }

    #[test]
    fn designator_from_lceda_or_fallback() {
        assert_eq!(item().sch_designator(), "U?");
        let sw = SearchItem {
            index: 1,
            display_title: "TS-1088".into(),
            title: String::new(),
            manufacturer: String::new(),
            model_uuid: None,
            raw: serde_json::json!({"attributes": {"Designator": "SW?"}}),
        };
        assert_eq!(sw.sch_designator(), "SW?");
        assert_eq!(normalize_designator("CN"), "CN?");
        assert_eq!(normalize_designator("KEY？"), "KEY?");
        assert_eq!(normalize_designator("RS2227XN"), "U?");
        assert_eq!(normalize_designator(""), "U?");
        assert_eq!(
            first_designator(&[Some("RS2227XN".into()), Some("CN?".into())]),
            "CN?"
        );
    }

    #[test]
    fn ad_properties_groups_and_skips_cad_uuids() {
        let item = SearchItem {
            index: 1,
            display_title: "1-1827875-5".into(),
            title: String::new(),
            manufacturer: "TE Connectivity(泰科电子)".into(),
            model_uuid: None,
            raw: serde_json::json!({
                "product_code": "C498523",
                "description": "插针结构:2x5P;间距:2.5mm;安装方式:直插;额定电流:5A;",
                "tags": {
                    "parent_tag": {"name_cn": "连接器"},
                    "child_tag": {"name_cn": "排针排母"}
                },
                "attributes": {
                    "LCSC Part Name": "2x5P 间距:2.5mm 直插",
                    "Supplier Footprint": "插件,P=2.5mm",
                    "Designator": "CN?",
                    "Manufacturer Part": "1-1827875-5",
                    "Symbol": "6993259b583542ad9fca4b00896cdbde",
                    "Flame Retardant Rating": "UL94V-0",
                    "Color": "黑色"
                }
            }),
        };
        let rows = item.ad_properties(&DescField::library(), true);
        assert!(rows.iter().any(|r| r.group == PropGroup::General && r.name == "Designator" && r.value == "CN?"));
        assert!(rows.iter().any(|r| r.group == PropGroup::General && r.name == "Comment" && r.value == "1-1827875-5"));
        assert!(rows.iter().any(|r| {
            r.group == PropGroup::General
                && r.name == "Description"
                && r.value.contains("2x5P 间距:2.5mm 直插")
                && r.value.contains("LCSC C498523")
        }));
        assert!(rows.iter().any(|r| r.group == PropGroup::Identity && r.name == "LCSC" && r.value == "C498523"));
        assert!(rows.iter().any(|r| r.group == PropGroup::Identity && r.name == "分类" && r.value.contains("连接器")));
        assert!(rows.iter().any(|r| r.group == PropGroup::Specs && r.name == "插针结构" && r.value == "2x5P"));
        assert!(rows.iter().any(|r| r.group == PropGroup::Extra && r.name == "阻燃等级" && r.value == "UL94V-0"));
        assert!(!rows.iter().any(|r| r.value.contains("6993259b")));
        let preview = item.ad_properties(&DescField::ids_only(), false);
        assert!(preview.iter().any(|r| r.name == "Description" && r.value.contains("型号:1-1827875-5")));
        assert_eq!(preview.len(), 3);
        assert!(!preview.iter().any(|r| r.name == "插针结构"));
        assert!(!preview.iter().any(|r| r.name == "阻燃等级"));
    }
}
