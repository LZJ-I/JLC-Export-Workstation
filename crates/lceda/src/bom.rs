//! BOM import and explicit candidate confirmation. Importing never silently chooses a model.
use crate::library::{package_from_item, Library, SavedPart, UNCAT};
use calamine::{open_workbook, open_workbook_auto, Reader, Xlsx};
use lceda_core::{client::LcedaClient, models::{looks_like_lcsc_token, SearchItem}};
use std::{collections::{HashMap, HashSet}, path::Path, sync::atomic::{AtomicBool, Ordering}};

const MAX_ROWS: usize = 10_000;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct BomRow {
    pub line: usize,
    pub lcsc: String,
    pub model: String,
    pub manufacturer: String,
    pub package: String,
    pub quantity: u32,
    pub designators: String,
    pub candidates: Vec<SearchItem>,
    /// An exact C-number may be suggested, but remains unconfirmed until the user acts.
    pub selected: Option<usize>,
    pub confirmed: bool,
    pub included: bool,
    pub error: Option<String>,
    /// Source-table errors need source correction; transient lookup errors can be retried.
    pub validation_error: bool,
}

impl BomRow {
    pub fn query(&self) -> &str {
        if !self.lcsc.is_empty() { &self.lcsc } else { &self.model }
    }

    pub fn chosen(&self) -> Option<&SearchItem> {
        self.selected.and_then(|index| self.candidates.get(index))
    }

    pub fn choose(&mut self, index: usize) -> bool {
        if self.validation_error { return false; }
        if self.candidates.get(index).and_then(SearchItem::lcsc_id).is_none() { return false; }
        self.selected = Some(index);
        self.confirmed = true;
        self.error = None;
        true
    }
}

#[derive(Debug, Clone, Default)]
pub struct BomDocument {
    pub source: String,
    pub rows: Vec<BomRow>,
    pub warnings: Vec<String>,
}

impl BomDocument {
    pub fn load(path: &Path) -> Result<Self, String> {
        if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_FILE_BYTES {
            return Err("BOM 文件超过 32 MiB，请先缩减表格".into());
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        if matches!(ext.as_str(), "xlsx" | "xlsb" | "ods") { check_archive_size(path)?; }
        let table = match ext.as_str() {
            "csv" | "tsv" | "txt" => {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                let text = decode_text(&bytes)?;
                parse_delimited(&text)?
            }
            "xlsx" => read_xlsx(path)?,
            "xls" | "xlsb" | "ods" => {
                let mut workbook = open_workbook_auto(path).map_err(|e| format!("无法读取表格: {e}"))?;
                // A cover sheet is common: use the first worksheet with identifiable BOM columns.
                let mut found = None;
                for name in workbook.sheet_names().to_vec() {
                    let range = workbook.worksheet_range(&name).map_err(|e| format!("工作表 {name}: {e}"))?;
                    if range.get_size().0 > MAX_ROWS + 50 || range.get_size().1 > 256 {
                        continue;
                    }
                    let rows: Vec<Vec<String>> = range.rows().map(|row| row.iter().map(ToString::to_string).collect()).collect();
                    if find_header(&rows).is_some() { found = Some(rows); break; }
                }
                found.ok_or("没有找到含立创编号或型号列的 BOM 工作表")?
            }
            _ => return Err("支持 CSV、TSV、XLSX、XLS、XLSB 和 ODS 文件".into()),
        };
        Self::from_table(path.file_name().unwrap_or_default().to_string_lossy().into_owned(), table)
    }

    pub fn from_table(source: String, table: Vec<Vec<String>>) -> Result<Self, String> {
        let (header_index, columns) = find_header(&table).ok_or("没有识别到表头，请使用“立创编号 / LCSC Part Number”或“型号 / MPN”列")?;
        let mut doc = Self { source, ..Default::default() };
        for (index, cells) in table.iter().enumerate().skip(header_index + 1) {
            if doc.rows.len() >= MAX_ROWS { return Err("BOM 超过 10000 行，请分批导入".into()); }
            let cell = |col: Option<usize>| col.and_then(|i| cells.get(i)).map(|s| s.trim()).unwrap_or("");
            if cells.iter().all(|s| s.trim().is_empty()) { continue; }
            let raw_lcsc = cell(columns.lcsc);
            let lcsc = normalize_id(raw_lcsc);
            let model = cell(columns.model).to_string();
            if lcsc.is_empty() && model.is_empty() {
                doc.warnings.push(format!("第 {} 行没有编号或型号，已跳过", index + 1));
                continue;
            }
            let qty = cell(columns.quantity);
            let designators = cell(columns.designators).to_string();
            let inferred = designators.split([',', ';', '，', '；', ' ']).filter(|s| !s.trim().is_empty()).count().max(1) as u32;
            let (quantity, mut error) = if qty.is_empty() { (inferred, None) } else {
                match qty.parse::<f64>() {
                    Ok(n) if n.is_finite() && n > 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 => (n as u32, None),
                    _ => (0, Some(format!("数量“{qty}”不是正整数，请修改源表"))),
                }
            };
            if !raw_lcsc.is_empty() && lcsc.is_empty() {
                error = Some(format!("立创编号“{raw_lcsc}”无效，请修改源表"));
            }
            doc.rows.push(BomRow {
                line: index + 1, lcsc, model,
                manufacturer: cell(columns.manufacturer).into(), package: cell(columns.package).into(),
                quantity, designators, validation_error: error.is_some(), error, included: true, ..Default::default()
            });
        }
        if doc.rows.is_empty() { return Err("BOM 中没有可导入的器件行".into()); }
        Ok(doc)
    }

    /// Run on a worker thread. Local snapshots make exact-ID lookup available offline.
    #[cfg(test)]
    pub fn resolve(&mut self, client: &LcedaClient, library: &Library) {
        self.resolve_with_control(client, library, &AtomicBool::new(false), |_, _| {});
    }

    pub fn resolve_with_control(
        &mut self, client: &LcedaClient, library: &Library,
        cancelled: &AtomicBool, progress: impl FnMut(usize, usize),
    ) {
        self.resolve_using(library, cancelled, progress, |query| client.search(query).map_err(|e| e.to_string()));
    }

    fn resolve_using(
        &mut self, library: &Library, cancelled: &AtomicBool,
        mut progress: impl FnMut(usize, usize),
        mut search: impl FnMut(&str) -> Result<Vec<SearchItem>, String>,
    ) {
        let total = self.rows.len();
        let mut lookups: HashMap<String, Result<Vec<SearchItem>, String>> = HashMap::new();
        progress(0, total);
        for (index, row) in self.rows.iter_mut().enumerate() {
            if cancelled.load(Ordering::Relaxed) { break; }
            if row.validation_error || !row.included || row.confirmed {
                progress(index + 1, total);
                continue;
            }
            // A retry clears lookup failure only; imported validation errors remain visible.
            row.error = None;
            let local = if row.lcsc.is_empty() {
                library.records.values().filter_map(|r| r.snapshot.clone())
                    .filter(|item| item.name().eq_ignore_ascii_case(&row.model)).collect::<Vec<_>>()
            } else { library.cached_item(&row.lcsc).into_iter().collect() };
            let mut candidates = local;
            if row.lcsc.is_empty() || candidates.is_empty() {
                let query = row.query().to_string();
                let result = lookups.entry(query.clone()).or_insert_with(|| search(&query));
                match result {
                    Ok(found) => candidates.extend(found.clone()),
                    Err(e) if candidates.is_empty() => {
                        row.error = Some(e.clone());
                        progress(index + 1, total);
                        continue;
                    }
                    Err(_) => {}
                }
            }
            apply_candidates(row, candidates);
            progress(index + 1, total);
        }
    }

    /// Confirm remaining exact supplier IDs without overriding explicit user choices.
    pub fn confirm_exact_ids(&mut self) -> usize {
        let mut confirmed = 0;
        for row in &mut self.rows {
            if row.confirmed || !row.included || row.error.is_some() || row.lcsc.is_empty() {
                continue;
            }
            if let Some(index) = row.candidates.iter()
                .position(|item| item.lcsc_id().as_deref() == Some(row.lcsc.as_str())) {
                if row.choose(index) { confirmed += 1; }
            }
        }
        confirmed
    }

    pub fn confirmed_parts(&self) -> Result<Vec<SavedPart>, String> {
        let mut parts = Vec::new();
        let mut ids = HashSet::new();
        for row in self.rows.iter().filter(|row| row.included) {
            if row.error.is_some() || !row.confirmed { return Err(format!("第 {} 行尚未确认匹配，请选择器件或排除此行", row.line)); }
            let item = row.chosen().ok_or_else(|| format!("第 {} 行缺少匹配器件", row.line))?;
            let lcsc = item.lcsc_id().ok_or_else(|| format!("第 {} 行没有立创编号", row.line))?;
            if ids.insert(lcsc.clone()) {
                parts.push(SavedPart { lcsc, name: item.name().into(), manufacturer: item.manufacturer_label(), cat_id: UNCAT.into() });
            }
        }
        if parts.is_empty() { return Err("请至少确认一个器件".into()); }
        Ok(parts)
    }
}

fn check_archive_size(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("表格压缩结构无效: {e}"))?;
    if archive.len() > 4096 { return Err("工作簿附件过多，请另存为仅包含 BOM 的表格".into()); }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        total = total.saturating_add(entry.size());
        if total > 128 * 1024 * 1024 { return Err("工作簿展开后超过 128 MiB，请另存为仅包含 BOM 的表格".into()); }
    }
    Ok(())
}

fn read_xlsx(path: &Path) -> Result<Vec<Vec<String>>, String> {
    let mut workbook: Xlsx<_> = open_workbook(path).map_err(|e| format!("无法读取 XLSX: {e}"))?;
    for name in workbook.sheet_names().to_vec() {
        // Stream XLSX cells rather than allocating the entire advertised sparse worksheet range.
        let mut reader = workbook.worksheet_cells_reader(&name).map_err(|e| e.to_string())?;
        let mut table = Vec::<Vec<String>>::new();
        let mut oversized = false;
        while let Some(cell) = reader.next_cell().map_err(|e| format!("工作表 {name}: {e}"))? {
            let value = calamine::Data::from(cell.get_value().clone()).to_string();
            if value.trim().is_empty() { continue; }
            let (row, column) = cell.get_position();
            if row as usize >= MAX_ROWS + 50 || column >= 256 {
                oversized = true;
                break;
            }
            table.resize_with((row as usize + 1).max(table.len()), Vec::new);
            let target = &mut table[row as usize];
            target.resize((column as usize + 1).max(target.len()), String::new());
            target[column as usize] = value;
        }
        if !oversized && find_header(&table).is_some() { return Ok(table); }
    }
    Err("没有找到含立创编号或型号列的 BOM 工作表（限 10000 行、256 列）".into())
}

fn apply_candidates(row: &mut BomRow, mut candidates: Vec<SearchItem>) {
    let mut seen = HashSet::new();
    candidates.retain(|item| item.lcsc_id().is_some_and(|id| seen.insert(id)));
    candidates.sort_by_key(|item| {
        let model = !row.model.is_empty() && item.name().eq_ignore_ascii_case(&row.model);
        let manufacturer = !row.manufacturer.is_empty() && item.manufacturer_label().eq_ignore_ascii_case(&row.manufacturer);
        let package = !row.package.is_empty() && package_from_item(item).eq_ignore_ascii_case(&row.package);
        std::cmp::Reverse((model as u8) * 4 + (manufacturer as u8) * 2 + package as u8)
    });
    row.selected = if row.lcsc.is_empty() { None } else {
        candidates.iter().position(|item| item.lcsc_id().as_deref() == Some(&row.lcsc))
    };
    row.confirmed = false;
    row.error = None;
    row.candidates = candidates;
    if row.candidates.is_empty() { row.error = Some("没有找到可导出的器件，请检查型号或编号".into()); }
    else if !row.lcsc.is_empty() && row.selected.is_none() {
        row.error = Some("搜索结果没有对应的立创编号，请检查源表".into());
    }
}

#[derive(Default)]
struct Columns { lcsc: Option<usize>, model: Option<usize>, manufacturer: Option<usize>, package: Option<usize>, quantity: Option<usize>, designators: Option<usize> }

fn find_header(table: &[Vec<String>]) -> Option<(usize, Columns)> {
    table.iter().take(50).enumerate().filter_map(|(row, cells)| {
        let mut columns = Columns::default();
        for (index, cell) in cells.iter().enumerate() {
            let key: String = cell.trim_start_matches('\u{feff}').to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
            let slot = match key.as_str() {
                "lcsc" | "lcscpart" | "lcscpartnumber" | "lcscpartno" | "supplierpart" | "supplierpartnumber" | "jlcpcbpart" | "jlcpcbpartnumber" | "productcode" | "立创编号" | "立创编码" | "立创料号" | "嘉立创编号" | "嘉立创料号" | "商品编号" | "商品编码" | "供应商编号" | "供应商料号" => &mut columns.lcsc,
                "mpn" | "manufacturerpartnumber" | "manufacturerpart" | "partnumber" | "partno" | "model" | "型号" | "器件型号" | "制造商型号" | "厂家型号" | "物料型号" | "comment" | "value" => &mut columns.model,
                "manufacturer" | "mfr" | "brand" | "厂商" | "厂家" | "制造商" | "品牌" | "厂牌" => &mut columns.manufacturer,
                "package" | "footprint" | "封装" | "封装名称" | "封装规格" => &mut columns.package,
                "quantity" | "qty" | "count" | "数量" | "用量" | "总数量" => &mut columns.quantity,
                "designator" | "designators" | "reference" | "references" | "refdes" | "位号" | "元件位号" | "器件位号" => &mut columns.designators,
                _ => continue,
            };
            if slot.is_none() || matches!(key.as_str(), "mpn" | "manufacturerpartnumber" | "型号" | "器件型号" | "制造商型号" | "厂家型号") {
                *slot = Some(index);
            }
        }
        let score = [columns.lcsc, columns.model, columns.manufacturer, columns.package, columns.quantity, columns.designators]
            .iter().filter(|c| c.is_some()).count();
        (columns.lcsc.is_some() || columns.model.is_some()).then_some((row, columns, score))
    }).max_by_key(|(row, _, score)| (*score, std::cmp::Reverse(*row)))
        .map(|(row, columns, _)| (row, columns))
}

fn normalize_id(text: &str) -> String {
    let value = text.trim().to_ascii_uppercase();
    if looks_like_lcsc_token(&value) { value }
    else if value.len() >= 3 && value.bytes().all(|b| b.is_ascii_digit()) { format!("C{value}") }
    else { String::new() }
}

fn decode_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) { return Ok(encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned()); }
    if bytes.starts_with(&[0xfe, 0xff]) { return Ok(encoding_rs::UTF_16BE.decode(&bytes[2..]).0.into_owned()); }
    if let Ok(text) = std::str::from_utf8(bytes) { return Ok(text.trim_start_matches('\u{feff}').to_string()); }
    let (decoded, _, errors) = encoding_rs::GBK.decode(bytes);
    if errors { Err("无法识别文件编码，请另存为 UTF-8 CSV".into()) } else { Ok(decoded.into_owned()) }
}

fn parse_delimited(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut best = None;
    for delimiter in [b',', b'\t', b';'] {
        let mut reader = csv::ReaderBuilder::new().delimiter(delimiter).has_headers(false).flexible(true).from_reader(text.as_bytes());
        let records: Result<Vec<Vec<String>>, _> = reader.records().take(MAX_ROWS + 51).map(|r| r.map(|r| r.iter().map(str::to_string).collect())).collect();
        if let Ok(rows) = records {
            if let Some((_, cols)) = find_header(&rows) {
                let score = [cols.lcsc, cols.model, cols.manufacturer, cols.package, cols.quantity, cols.designators].iter().filter(|c| c.is_some()).count();
                if best.as_ref().is_none_or(|(previous, _)| score > *previous) { best = Some((score, rows)); }
            }
        }
    }
    best.map(|(_, rows)| rows).ok_or_else(|| "无法识别 CSV 表头或分隔符，请检查文件".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;


    fn candidate() -> SearchItem {
        SearchItem { index: 1, display_title: "BAT54".into(), title: "BAT54".into(), manufacturer: "maker".into(), model_uuid: None, raw: json!({"product_code":"C2040"}) }
    }

    #[test]
    fn confirm_exact_ids_preserves_manual_substitutes_and_skips_ineligible_rows() {
        let exact = candidate();
        let mut substitute = exact.clone();
        substitute.raw["product_code"] = json!("C9999");
        let mut manual = BomRow {
            lcsc: "C2040".into(), included: true,
            candidates: vec![exact.clone(), substitute], ..Default::default()
        };
        assert!(manual.choose(1));
        let pending = BomRow {
            lcsc: "C2040".into(), included: true,
            candidates: vec![exact], ..Default::default()
        };
        let mut excluded = pending.clone();
        excluded.included = false;
        let mut error = pending.clone();
        error.error = Some("lookup failed".into());
        let mut model_only = pending.clone();
        model_only.lcsc.clear();
        let mut doc = BomDocument {
            rows: vec![manual, pending, excluded, error, model_only], ..Default::default()
        };
        assert_eq!(doc.confirm_exact_ids(), 1);
        assert_eq!(doc.rows[0].chosen().unwrap().lcsc_id().as_deref(), Some("C9999"));
        assert!(doc.rows[0].confirmed);
        assert!(doc.rows[1].confirmed);
        assert_eq!(doc.rows[1].chosen().unwrap().lcsc_id().as_deref(), Some("C2040"));
        assert!(doc.rows[2..].iter().all(|row| !row.confirmed));
        assert_eq!(doc.confirm_exact_ids(), 0, "repeated confirmation must be idempotent");
    }

    #[test]
    fn explicit_candidate_choice_clears_lookup_error_but_never_source_validation() {
        let mut row = BomRow {
            candidates: vec![candidate()], error: Some("lookup timeout".into()), included: true,
            ..Default::default()
        };
        assert!(!row.choose(99));
        assert_eq!(row.error.as_deref(), Some("lookup timeout"));
        assert!(row.choose(0));
        assert!(row.confirmed);
        assert!(row.error.is_none());
        row.validation_error = true;
        row.error = Some("invalid quantity".into());
        row.confirmed = false;
        assert!(!row.choose(0));
        assert!(!row.confirmed);
        assert_eq!(row.error.as_deref(), Some("invalid quantity"));
    }

    #[test]
    fn retry_transient_failure_preserves_source_errors_and_user_confirmation() {
        let mut doc = BomDocument::from_table("retry.csv".into(), parse_delimited("LCSC,MPN,Qty\nC2040,BAT54,1\nC2040,BAT54,1.5\n").unwrap()).unwrap();
        let lib = Library::default();
        let cancelled = AtomicBool::new(false);
        doc.resolve_using(&lib, &cancelled, |_, _| {}, |_| Err("offline".into()));
        assert_eq!(doc.rows[0].error.as_deref(), Some("offline"));
        assert!(!doc.rows[0].validation_error);
        doc.resolve_using(&lib, &cancelled, |_, _| {}, |_| Ok(vec![candidate()]));
        assert!(doc.rows[0].error.is_none());
        assert!(doc.rows[0].choose(0));
        assert!(doc.rows[1].validation_error);
        doc.resolve_using(&lib, &cancelled, |_, _| {}, |_| panic!("confirmed and invalid rows must never search"));
        assert!(doc.rows[0].confirmed);
        assert_eq!(doc.rows[0].chosen().unwrap().lcsc_id().as_deref(), Some("C2040"));
        assert!(doc.rows[1].error.is_some());
    }

    #[test]
    fn duplicate_queries_share_lookup_and_cancellation_keeps_completed_rows() {
        let mut doc = BomDocument::from_table("repeat.csv".into(), parse_delimited("LCSC,Qty\nC2040,1\nC2040,2\nC9999,1\n").unwrap()).unwrap();
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        doc.resolve_using(&Library::default(), &cancelled, |done, _| {
            if done == 2 { cancelled.store(true, Ordering::Relaxed); }
        }, |_| { calls += 1; Ok(vec![candidate()]) });
        assert_eq!(calls, 1);
        assert_eq!(doc.rows[0].candidates.len(), 1);
        assert_eq!(doc.rows[1].candidates.len(), 1);
        assert!(doc.rows[2].candidates.is_empty());
    }

    #[test]
    fn xlsx_import_reads_inline_strings_and_skips_cover_sheet() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("lceda-bom-{}.xlsx", std::process::id()));
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let files = [
            ("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#),
            ("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#),
            ("xl/workbook.xml", r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Cover" sheetId="1" r:id="rId1"/><sheet name="BOM" sheetId="2" r:id="rId2"/></sheets></workbook>"#),
            ("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#),
            ("xl/worksheets/sheet1.xml", r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Project A</t></is></c></row></sheetData></worksheet>"#),
            ("xl/worksheets/sheet2.xml", r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>LCSC</t></is></c><c r="B1" t="inlineStr"><is><t>MPN</t></is></c><c r="C1" t="inlineStr"><is><t>Qty</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>C2040</t></is></c><c r="B2" t="inlineStr"><is><t>BAT54</t></is></c><c r="C2"><v>3</v></c></row></sheetData></worksheet>"#),
        ];
        for (name, data) in files {
            archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(data.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        let doc = BomDocument::load(&path).unwrap();
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].model, "BAT54");
        assert_eq!(doc.rows[0].quantity, 3);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn explicit_mpn_column_wins_over_generic_value() {
        let doc = BomDocument::from_table("part.csv".into(), parse_delimited("Value,MPN,LCSC
Diode,BAT54,C2040
").unwrap()).unwrap();
        assert_eq!(doc.rows[0].model, "BAT54");
    }

    #[test]
    fn chinese_csv_quoted_designators_and_quantities() {
        let table = parse_delimited("物料清单\n立创编号,型号,厂商,封装,数量,位号\nc2040,BAT54,Diodes,SOT-23,2,\"D1,D2\"\n,STM32,ST,LQFP,1,U1\n").unwrap();
        let doc = BomDocument::from_table("test.csv".into(), table).unwrap();
        assert_eq!(doc.rows[0].lcsc, "C2040");
        assert_eq!(doc.rows[0].designators, "D1,D2");
        assert_eq!(doc.rows[0].quantity, 2);
        assert!(!doc.rows[0].confirmed);
        assert!(doc.confirmed_parts().is_err());
    }

    #[test]
    fn semicolon_and_bom_utf8_are_supported() {
        let table = parse_delimited("\u{feff}MPN;LCSC Part Number;Qty;Footprint\nBAT54;2040;1.0;SOT-23\n").unwrap();
        let doc = BomDocument::from_table("bom.csv".into(), table).unwrap();
        assert_eq!(doc.rows[0].lcsc, "C2040");
        assert_eq!(doc.rows[0].package, "SOT-23");
    }

    #[test]
    fn invalid_id_and_quantity_cannot_be_silently_exported() {
        let table = parse_delimited("LCSC,MPN,Qty\nC2040,BAT54,1.5\nC12garbage,STM32,1\n").unwrap();
        let doc = BomDocument::from_table("bad.csv".into(), table).unwrap();
        assert!(doc.rows.iter().all(|r| r.error.is_some()));
        assert!(doc.confirmed_parts().is_err());
    }

    #[test]
    fn local_exact_match_still_requires_explicit_confirmation() {
        let mut lib = Library::default();
        lib.enrich_from_item(&SearchItem { index: 1, display_title: "BAT54".into(), title: "BAT54".into(), manufacturer: "maker".into(), model_uuid: None, raw: json!({"product_code":"C2040"}) });
        let mut doc = BomDocument::from_table("local.csv".into(), parse_delimited("LCSC,Qty\nC2040,2\n").unwrap()).unwrap();
        doc.resolve(&LcedaClient::new(), &lib);
        assert_eq!(doc.rows[0].selected, Some(0));
        assert!(!doc.rows[0].confirmed);
        assert!(doc.confirmed_parts().is_err());
        assert!(doc.rows[0].choose(0));
        assert_eq!(doc.confirmed_parts().unwrap()[0].lcsc, "C2040");
    }
}
