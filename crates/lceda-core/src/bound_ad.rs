//! Add components to a user-selected AD library while retaining unrelated CFB streams.
//! Unsupported directory/header layouts are rejected before either original is changed.
use crate::{altium::{self, binary::BinWriter, parameters, SchColors}, managed_ad::Entry, Error, Result};
use encoding_rs::{GBK, WINDOWS_1252};
use std::{collections::{HashMap, HashSet}, fs::{self, File, OpenOptions}, io::{Cursor, Read, Seek, Write}, path::{Path, PathBuf}};

type Document = cfb::CompoundFile<File>;
type Parameters = Vec<(String, String)>;
const MAX_FILE: u64 = 256 * 1024 * 1024;

fn invalid(message: impl AsRef<str>) -> Error {
    Error::msg(format!("绑定 AD 库格式不支持或已损坏：{}。原库保持不变，请选择其它库文件。", message.as_ref()))
}
fn io_context(action: &str, error: std::io::Error) -> Error {
    Error::msg(format!("绑定 AD 库{action}失败：{error}。请关闭占用该库的 AD/其它程序，并检查目录写入权限后重试。"))
}
fn paired(path: &Path) -> Result<PathBuf> {
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("SchLib")) || path.file_stem().is_none() {
        return Err(Error::msg("分类库绑定请选择 .SchLib 文件；封装同步到同名 .PcbLib"));
    }
    Ok(path.with_extension("PcbLib"))
}
fn open(path: &Path) -> Result<Document> {
    if fs::metadata(path)?.len() > MAX_FILE { return Err(invalid("库文件超过 256 MiB")); }
    cfb::open(path).map_err(|error| {
        if matches!(error.kind(),std::io::ErrorKind::PermissionDenied|std::io::ErrorKind::WouldBlock)||matches!(error.raw_os_error(),Some(32|33)) {io_context("读取",error)}
        else {invalid(format!("{}: {error}",path.display()))}
    })
}
fn open_memory(path:&Path)->Result<cfb::CompoundFile<Cursor<Vec<u8>>>>{
    let mut bytes=Vec::new();File::open(path)?.take(MAX_FILE+1).read_to_end(&mut bytes)?;
    if bytes.len() as u64>MAX_FILE{return Err(invalid("库文件超过 256 MiB"));}
    cfb::CompoundFile::open(Cursor::new(bytes)).map_err(|e|invalid(e.to_string()))
}
fn stream<T:Read+Seek>(doc: &mut cfb::CompoundFile<T>, name: &str) -> Result<Vec<u8>> {
    let mut value = Vec::new();
    let mut source = doc.open_stream(name).map_err(|error| invalid(format!("缺少或无法读取 {name}: {error}")))?;
    if source.len() > MAX_FILE { return Err(invalid(format!("{name} 过大"))); }
    source.read_to_end(&mut value)?;
    Ok(value)
}
fn put<T:Read+Write+Seek>(doc: &mut cfb::CompoundFile<T>, name: &str, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = Path::new(name).parent() { doc.create_storage_all(parent)?; }
    doc.create_stream(name)?.write_all(bytes)?;
    Ok(())
}
fn block<'a>(data: &'a [u8], offset: &mut usize) -> Result<(u8, &'a [u8])> {
    let header = data.get(*offset..*offset + 4).ok_or_else(|| invalid("参数块长度缺失"))?;
    let size = u32::from_le_bytes(header.try_into().unwrap());
    *offset += 4;
    let end = offset.checked_add((size & 0x00ff_ffff) as usize).ok_or_else(|| invalid("参数块长度溢出"))?;
    let payload = data.get(*offset..end).ok_or_else(|| invalid("参数块超出文件范围"))?;
    *offset = end;
    Ok(((size >> 24) as u8, payload))
}
fn params(bytes: &[u8]) -> Result<Parameters> {
    parameters::decode(bytes).map_err(|error| invalid(error.to_string()))
}
fn value<'a>(parameters: &'a Parameters, name: &str) -> Option<&'a str> {
    parameters::value(parameters, name)
}
fn number(parameters: &Parameters, name: &str) -> Result<usize> {
    let count = value(parameters, name).and_then(|value| value.parse::<usize>().ok()).ok_or_else(|| invalid(format!("{name} 缺失或无效")))?;
    if count > 100_000 { return Err(invalid(format!("{name} 超出支持范围"))); }
    Ok(count)
}
fn set(parameters: &mut Parameters, name: &str, new: impl ToString) {
    if let Some((_, value)) = parameters.iter_mut().find(|(key, _)| key.eq_ignore_ascii_case(name)) { *value = new.to_string(); }
    else { parameters.push((name.into(), new.to_string())); }
}
fn encode_params(parameters: &Parameters) -> Vec<u8> {
    let mut writer = BinWriter::new();
    let pairs: Vec<_> = parameters.iter().map(|(key, value)| (key.as_str(), value.clone())).collect();
    writer.write_params(&pairs);
    writer.into_vec()
}
fn short_name(payload: &[u8]) -> Result<String> {
    let length = *payload.first().ok_or_else(|| invalid("空目录名称"))? as usize;
    let content = payload.get(1..1 + length).ok_or_else(|| invalid("目录名称长度无效"))?;
    if payload.len() != length + 1 && !(payload.len() == length + 2 && payload.last() == Some(&0)) { return Err(invalid("目录名称有未知尾部数据")); }
    Ok(WINDOWS_1252.decode(content).0.into_owned())
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." { return Err(invalid("组件存储名称无效")); }
    Ok(())
}
fn string_index(data: &[u8], start: usize) -> Result<Vec<String>> {
    let count = data.get(start..start + 4).ok_or_else(|| invalid("目录数量缺失"))?;
    let count = u32::from_le_bytes(count.try_into().unwrap()) as usize;
    if count > 100_000 { return Err(invalid("目录组件过多")); }
    let mut offset = start + 4;
    let mut names = Vec::new();
    let mut unique = HashSet::new();
    for _ in 0..count {
        let (flags, bytes) = block(data, &mut offset)?;
        if flags != 0 { return Err(invalid("目录名称编码不支持")); }
        let name = short_name(bytes)?;
        validate_name(&name)?;
        if !unique.insert(name.to_lowercase()) { return Err(invalid("目录有重复组件")); }
        names.push(name);
    }
    if offset != data.len() { return Err(invalid("目录包含未知尾部索引")); }
    Ok(names)
}
fn encode_index(prefix: &[u8], names: &[String]) -> Vec<u8> {
    let mut writer = BinWriter::new();
    writer.write_bytes(prefix);
    writer.write_u32(names.len() as u32);
    for name in names { writer.write_string_block(name); }
    writer.into_vec()
}

struct SchIndex { header: Parameters, raw_header: Vec<u8>, names: Vec<String>, references: Vec<String>, mappings: Parameters, indexed: bool }
fn sch_index<T:Read+Seek>(doc: &mut cfb::CompoundFile<T>) -> Result<SchIndex> {
    let bytes = stream(doc, "/FileHeader")?;
    let mut offset = 0;
    let (flags, payload) = block(&bytes, &mut offset)?;
    let header = params(payload)?;
    if flags != 0 || !value(&header, "HEADER").is_some_and(|header| header.starts_with("Protel for Windows - Schematic Library Editor Binary File Version 5.0")) {
        return Err(invalid("不是受支持的 SchLib 5.0"));
    }
    let raw_header = payload.to_vec();
    let count = number(&header, "COMPCOUNT")?;
    let indexed = offset != bytes.len();
    let directory = if indexed { Some(string_index(&bytes, offset)?) } else { None };
    if directory.as_ref().is_some_and(|names| names.len() != count) { return Err(invalid("SchLib 组件数量不一致")); }
    let fonts = number(&header, "FONTIDCOUNT")?;
    if fonts == 0 || fonts > 4096 { return Err(invalid("SchLib 字体表超出范围")); }
    for id in 1..=fonts {
        if value(&header, &format!("FONTNAME{id}")).is_none() { return Err(invalid("SchLib 字体名称缺失")); }
    }
    let mappings = if doc.is_stream("/SectionKeys") {
        let bytes = stream(doc, "/SectionKeys")?;
        let mut offset = 0;
        let (_, payload) = block(&bytes, &mut offset)?;
        if offset != bytes.len() { return Err(invalid("SchLib SectionKeys 尾部未知")); }
        let mappings = params(payload)?;
        let count = number(&mappings, "KeyCount")?;
        for i in 0..count {
            let key = value(&mappings, &format!("SectionKey{i}")).ok_or_else(|| invalid("SchLib SectionKeys 缺少映射"))?;
            validate_name(key)?;
            if value(&mappings, &format!("LibRef{i}")).is_none() || !doc.is_stream(format!("/{key}/Data")) { return Err(invalid("SchLib SectionKeys 无效")); }
        }
        mappings
    } else { vec![("KeyCount".into(), "0".into())] };
    let mut names = Vec::new();
    let mut references = Vec::new();
    let mut unique = HashSet::new();
    for i in 0..count {
        let reference = value(&header, &format!("LIBREF{i}")).ok_or_else(|| invalid("SchLib 缺少组件名称"))?.to_string();
        let mapped = (0..number(&mappings, "KeyCount")?).find_map(|j| {
            (value(&mappings, &format!("LibRef{j}")) == Some(&reference)).then(|| value(&mappings, &format!("SectionKey{j}")).unwrap().to_string())
        });
        // Native AD preserves Unicode/spaces and substitutes slash in OLE names.
        let implicit: String = reference.chars().map(|c| if matches!(c, '/' | '\\') { '_' } else { c })
            .scan(0usize, |length,c| { *length += c.len_utf16(); (*length <= 31).then_some(c) }).collect();
        let expected = mapped.unwrap_or(implicit);
        let key = directory.as_ref().map(|d| d[i].clone()).unwrap_or_else(|| expected.clone());
        if !key.eq_ignore_ascii_case(&expected) { return Err(invalid("SchLib 二进制索引与名称映射不一致")); }
        validate_name(&key)?;
        if !unique.insert(key.to_lowercase()) || !doc.is_storage(format!("/{key}")) || !doc.is_stream(format!("/{key}/Data")) {
            return Err(invalid(format!("SchLib 组件 {reference} 的目录与存储不一致")));
        }
        let display=(0..number(&mappings,"KeyCount")?).find_map(|j| {
            (value(&mappings,&format!("SectionKey{j}"))==Some(key.as_str())).then(||value(&mappings,&format!("LibRef{j}")).unwrap().to_string())
        }).unwrap_or(reference);
        names.push(key); references.push(display);
    }
    Ok(SchIndex { header, raw_header, names, references, mappings, indexed })
}

struct PcbIndex { prefix: Vec<u8>, names: Vec<String>, mappings: Vec<(String, String)>, toc: Vec<Vec<u8>>, models: Vec<(Parameters, Vec<u8>)> }
fn pcb_index(doc: &mut Document) -> Result<PcbIndex> {
    let header = stream(doc, "/FileHeader")?;
    if !header.windows(b"PCB 6.0 Binary Library File".len()).any(|window| window == b"PCB 6.0 Binary Library File") { return Err(invalid("不是受支持的 PcbLib 6.0")); }
    let bytes = stream(doc, "/Library/Data")?;
    let mut offset = 0;
    let (flags, data) = block(&bytes, &mut offset)?;
    if flags != 0 || !data.windows(b"Protel_Advanced_PCB_Library".len()).any(|window| window == b"Protel_Advanced_PCB_Library") { return Err(invalid("PCB Library/Data 类型无效")); }
    let prefix = bytes[..offset].to_vec();
    let names = string_index(&bytes, offset)?;
    for name in &names {
        if !doc.is_storage(name) || !doc.is_stream(format!("{name}/Data")) { return Err(invalid("PcbLib 组件目录与存储不一致")); }
    }
    let mut mappings = Vec::new();
    if doc.is_stream("/SectionKeys") {
        let bytes = stream(doc, "/SectionKeys")?;
        let raw = bytes.get(..4).ok_or_else(|| invalid("PcbLib SectionKeys 无效"))?;
        let count = u32::from_le_bytes(raw.try_into().unwrap()) as usize;
        if count > 100_000 { return Err(invalid("PcbLib SectionKeys 过大")); }
        let mut offset = 4;
        for _ in 0..count {
            let (_, name) = block(&bytes, &mut offset)?;
            let (_, key) = block(&bytes, &mut offset)?;
            let name = short_name(name)?;
            let key = short_name(key)?;
            if !names.iter().any(|existing| existing.eq_ignore_ascii_case(&key)) { return Err(invalid("PcbLib SectionKeys 指向不存在的组件")); }
            mappings.push((name, key));
        }
        if offset != bytes.len() { return Err(invalid("PcbLib SectionKeys 尾部未知")); }
    }
    let toc = raw_blocks(&stream(doc, "/Library/ComponentParamsTOC/Data")?)?;
    if toc.len() != names.len() || stream_count(doc, "/Library/ComponentParamsTOC/Header")? != toc.len() { return Err(invalid("PCB ComponentParamsTOC 数量不一致")); }
    for (name, record) in names.iter().zip(&toc) {
        let mut offset = 0;
        let (_, payload) = block(record, &mut offset)?;
        let entry = params(payload)?;
        if !value(&entry, "Name").is_some_and(|value| value.eq_ignore_ascii_case(name)) { return Err(invalid("PCB ComponentParamsTOC 名称不一致")); }
    }
    let model_blocks = raw_blocks(&stream(doc, "/Library/Models/Data")?)?;
    if stream_count(doc, "/Library/Models/Header")? != model_blocks.len() { return Err(invalid("PCB Models 数量不一致")); }
    let mut models = Vec::new();
    let mut ids = HashSet::new();
    for (index, record) in model_blocks.iter().enumerate() {
        let mut offset = 0;
        let (_, payload) = block(record, &mut offset)?;
        let parameters = params(payload)?;
        let id = value(&parameters, "ID").ok_or_else(|| invalid("PCB 模型缺少 ID"))?;
        if !ids.insert(id.to_lowercase()) { return Err(invalid("PCB 模型 ID 重复")); }
        if !value(&parameters, "EMBED").is_some_and(|value| value.eq_ignore_ascii_case("TRUE")) { return Err(invalid("PCB 模型不是内嵌格式")); }
        models.push((parameters, stream(doc, &format!("/Library/Models/{index}"))?));
    }
    Ok(PcbIndex { prefix, names, mappings, toc, models })
}
fn raw_blocks(bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut offset = 0;
    let mut blocks = Vec::new();
    while offset < bytes.len() {
        let start = offset;
        let (flags, _) = block(bytes, &mut offset)?;
        if flags != 0 { return Err(invalid("索引参数块类型不支持")); }
        blocks.push(bytes[start..offset].to_vec());
        if blocks.len() > 100_000 { return Err(invalid("索引过大")); }
    }
    Ok(blocks)
}
fn stream_count(doc: &mut Document, path: &str) -> Result<usize> {
    let bytes = stream(doc, path)?;
    let count: [u8; 4] = bytes.try_into().map_err(|_| invalid(format!("{path} 数量头长度无效")))?;
    Ok(u32::from_le_bytes(count) as usize)
}

/// Validate existing files without creating directories, locks, backups, or a new library.
pub fn validate_binding(sch_path: &Path) -> Result<()> {
    let pcb_path = paired(sch_path)?;
    if sch_path.exists() { sch_index(&mut open(sch_path)?)?; }
    if pcb_path.exists() { pcb_index(&mut open(&pcb_path)?)?; }
    Ok(())
}

pub(crate) fn inspect_library(path: &Path) -> Result<Vec<crate::native_library::NativePart>> {
    let mut doc = open_memory(path)?; let index = sch_index(&mut doc)?;
    let mut parts = Vec::new();
    for (i,key) in index.names.iter().enumerate() {
        let bytes = stream(&mut doc, &format!("/{key}/Data"))?;
        let mut offset = 0; let mut properties = std::collections::BTreeMap::new();
        let mut description = value(&index.header, &format!("COMPDESCR{i}")).unwrap_or("").to_string();
        let mut footprints = Vec::new(); let mut pins = 0usize; let mut units = 1usize;
        while offset < bytes.len() {
            let (flags, payload) = block(&bytes, &mut offset)?;
            if flags == 1 { if payload.get(..4) == Some(&2i32.to_le_bytes()) { pins += 1; } continue; }
            if flags != 0 { continue; }
            let p = params(payload)?;
            match value(&p, "RECORD").unwrap_or("") {
                "1" => {
                    units = value(&p,"PARTCOUNT").and_then(|s| s.parse::<usize>().ok()).unwrap_or(2).saturating_sub(1).clamp(1,1024);
                    if description.is_empty() { description = value(&p,"COMPONENTDESCRIPTION").unwrap_or("").to_string(); }
                }
                "2" => pins += 1,
                "34" | "41" => {
                    if let Some(name) = value(&p,"NAME") { properties.insert(name.to_string(), value(&p,"TEXT").unwrap_or("").to_string()); }
                }
                "45" if value(&p,"MODELTYPE").is_some_and(|s| s.eq_ignore_ascii_case("PCBLIB")) => {
                    if let Some(name) = value(&p,"MODELNAME").filter(|name| !name.is_empty()) { footprints.push(name.to_string()); }
                }
                _ => {}
            }
        }
        parts.push(crate::native_library::NativePart { key:key.clone(), name:index.references[i].clone(), description, properties, footprints, pins, units });
    }
    Ok(parts)
}

pub(crate) fn preview_component(path: &Path, key: &str, unit: usize) -> Result<altium::sch_preview::SchPreview> {
    let mut doc = open_memory(path)?; let index = sch_index(&mut doc)?;
    if !index.names.iter().any(|name| name == key) { return Err(invalid("没有找到原理图符号")); }
    let header = stream(&mut doc,"/FileHeader")?;
    let data = stream(&mut doc,&format!("/{key}/Data"))?;
    let pin_path = format!("/{key}/PinTextData");
    let pins = if doc.is_stream(&pin_path) { Some(stream(&mut doc,&pin_path)?) } else { None };
    altium::sch_preview::SchPreview::from_streams_unit(&header,&data,pins.as_deref(),unit)
}

pub(crate) fn export_subset(source: &Path, keys: &[String], destination: &Path) -> Result<()> {
    let mut doc = open_memory(source)?; let index = sch_index(&mut doc)?;
    let wanted: HashSet<_> = keys.iter().collect();
    if wanted.iter().any(|key| !index.names.contains(key)) { return Err(invalid("导出器件已不在原库中")); }
    let mut created=false;
    let result = (|| -> Result<()> {
        let mut header = index.header.clone();
        let indexed_field = |name: &str| {
            let name = name.trim_start_matches("%UTF8%").to_ascii_uppercase();
            ["LIBREF","PARTCOUNT","COMPDESCR"].iter().any(|prefix| name.strip_prefix(prefix).is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit())))
        };
        header.retain(|(name,_)| !indexed_field(name));
        let mut names = Vec::new();
        for (i,key) in index.names.iter().enumerate() {
            if !wanted.contains(key) { doc.remove_storage_all(format!("/{key}"))?; continue; }
            let next = names.len(); names.push(key.clone());
            for prefix in ["LIBREF","PARTCOUNT","COMPDESCR"] {
                for utf in ["","%UTF8%"] {
                    let old = format!("{utf}{prefix}{i}");
                    if let Some((_,text)) = index.header.iter().find(|(name,_)| name.eq_ignore_ascii_case(&old)) { header.push((format!("{utf}{prefix}{next}"),text.clone())); }
                }
            }
        }
        set(&mut header,"COMPCOUNT",names.len());
        let mut writer = BinWriter::new(); writer.write_block_raw(0,&parameters::patch(&index.raw_header,&header)?);
        let bytes = if index.indexed { encode_index(&writer.into_vec(),&names) } else { writer.into_vec() };
        put(&mut doc,"/FileHeader",&bytes)?;
        let mut mappings = vec![("KeyCount".into(),"0".into())]; let mut count = 0;
        for i in 0..number(&index.mappings,"KeyCount")? {
            let key = value(&index.mappings,&format!("SectionKey{i}")).unwrap();
            if names.iter().any(|name| name == key) {
                mappings.push((format!("LibRef{count}"),value(&index.mappings,&format!("LibRef{i}")).unwrap().to_string()));
                mappings.push((format!("SectionKey{count}"),key.to_string())); count += 1;
            }
        }
        if count > 0 { set(&mut mappings,"KeyCount",count); put(&mut doc,"/SectionKeys",&encode_params(&mappings))?; }
        else if doc.is_stream("/SectionKeys") { doc.remove_stream("/SectionKeys")?; }
        doc.flush()?; sch_index(&mut doc)?;
        // One read and one write keep network-backed libraries responsive.
        // Validate the in-memory copy before creating a new output file.
        let bytes=doc.into_inner().into_inner();
        let mut output=OpenOptions::new().write(true).create_new(true).open(destination)?;
        created=true;
        output.write_all(&bytes)?;output.flush()?;output.sync_all()?;Ok(())
    })();
    if result.is_err()&&created { let _ = fs::remove_file(destination); }
    result
}

fn copy_component(source: &mut Document, dest: &mut Document, key: &str, old_key: Option<&str>) -> Result<()> {
    if let Some(old) = old_key { if dest.is_storage(old) { dest.remove_storage_all(old)?; } }
    if dest.exists(key) { return Err(invalid(format!("组件名称 {key} 与原库其它存储冲突"))); }
    let entries: Vec<_> = source.walk_storage(key)?.collect();
    for entry in entries {
        if entry.is_storage() {
            dest.create_storage_all(entry.path())?;
            dest.set_storage_clsid(entry.path(), *entry.clsid())?;
        } else if entry.is_stream() {
            let bytes = stream(source, entry.path().to_str().ok_or_else(|| invalid("CFB路径不是文本"))?)?;
            put(dest, entry.path().to_str().unwrap(), &bytes)?;
        }
    }
    Ok(())
}
fn font_signature(parameters: &Parameters, id: usize) -> Vec<String> {
    ["FONTNAME", "SIZE", "BOLD", "ITALIC", "UNDERLINE", "STRIKEOUT", "COLOR", "CHARSET", "ROTATION"].iter()
        .map(|field| value(parameters, &format!("{field}{id}")).unwrap_or(match *field { "BOLD" | "ITALIC" | "UNDERLINE" | "STRIKEOUT" => "F", "COLOR" | "CHARSET" => "0", _ => "" }).to_ascii_lowercase()).collect()
}
fn merge_fonts(old: &mut Parameters, fresh: &Parameters) -> Result<HashMap<usize, usize>> {
    let mut old_count = number(old, "FONTIDCOUNT")?;
    let mut mappings = HashMap::new();
    for id in 1..=number(fresh, "FONTIDCOUNT")? {
        let signature = font_signature(fresh, id);
        let found = (1..=old_count).find(|old_id| font_signature(old, *old_id) == signature);
        let target = if let Some(found) = found { found } else {
            if old_count >= 4096 { return Err(invalid("合并后的字体表超过 4096 项")); }
            old_count += 1;
            for prefix in ["FONTNAME", "SIZE", "BOLD", "ITALIC", "UNDERLINE", "STRIKEOUT", "COLOR", "CHARSET", "ROTATION"] {
                if let Some(text) = value(fresh, &format!("{prefix}{id}")) { set(old, &format!("{prefix}{old_count}"), text); }
            }
            old_count
        };
        mappings.insert(id, target);
    }
    set(old, "FONTIDCOUNT", old_count);
    Ok(mappings)
}
fn mapped_font(id: &str, fonts: &HashMap<usize, usize>) -> Result<String> {
    let id: usize = id.parse().map_err(|_| invalid("新组件字体引用无效"))?;
    if id == 0 { return Ok("0".into()); }
    fonts.get(&id).map(ToString::to_string).ok_or_else(|| invalid("新组件引用不存在的字体"))
}
fn remap_component_fonts(doc: &mut Document, key: &str, fonts: &HashMap<usize, usize>) -> Result<()> {
    let data = stream(doc, &format!("/{key}/Data"))?;
    let mut writer = BinWriter::new();
    let mut offset = 0;
    while offset < data.len() {
        let (flags, payload) = block(&data, &mut offset)?;
        if flags == 0 && payload.first() == Some(&b'|') {
            let raw = payload.strip_suffix(&[0]).unwrap_or(payload);
            let (text, _, errors) = GBK.decode(raw);
            if errors { return Err(invalid("新组件参数编码无效")); }
            let fields: Result<Vec<_>> = text.split('|').map(|token| {
                if let Some((name, id)) = token.split_once('=') {
                    if name.to_ascii_uppercase().ends_with("FONTID") {
                        return Ok(format!("{name}={}", mapped_font(id, fonts)?));
                    }
                }
                Ok(token.to_string())
            }).collect();
            let text = fields?.join("|");
            writer.write_block(0, |writer| writer.write_cstring(&text));
        } else { writer.write_block_raw(flags, payload); }
    }
    put(doc, &format!("/{key}/Data"), &writer.into_vec())?;
    let path = format!("/{key}/PinTextData");
    if doc.is_stream(&path) {
        let data = stream(doc, &path)?;
        let mut offset = 0;
        let (flags, header) = block(&data, &mut offset)?;
        let mut writer = BinWriter::new();
        writer.write_block_raw(flags, header);
        while offset < data.len() {
            let (flags, record) = block(&data, &mut offset)?;
            if flags != 1 || record.first() != Some(&0xd0) { return Err(invalid("新组件 PinTextData 压缩块无效")); }
            let length = *record.get(1).ok_or_else(|| invalid("PinTextData 名称缺失"))? as usize;
            let name = record.get(2..2 + length).ok_or_else(|| invalid("PinTextData 名称长度无效"))?;
            let compressed_size = record.get(2 + length..6 + length).ok_or_else(|| invalid("PinTextData 载荷缺失"))?;
            let compressed_size = u32::from_le_bytes(compressed_size.try_into().unwrap()) as usize;
            let compressed = record.get(6 + length..).ok_or_else(|| invalid("PinTextData 压缩流缺失"))?;
            if compressed.len() != compressed_size { return Err(invalid("PinTextData 长度不一致")); }
            let mut payload = Vec::new();
            flate2::read::ZlibDecoder::new(compressed).take(64).read_to_end(&mut payload)?;
            if payload.len() != 14 || payload[0] != 0x10 || payload[7] != 0x10 { return Err(invalid("新组件 PinTextData 字体属性布局无效")); }
            for pos in [1usize, 8] {
                let font = i16::from_le_bytes(payload[pos..pos + 2].try_into().unwrap());
                if font < 0 { return Err(invalid("PinTextData 字体索引为负")); }
                let target: i16 = mapped_font(&font.to_string(), fonts)?.parse().map_err(|_| invalid("字体索引超出范围"))?;
                payload[pos..pos + 2].copy_from_slice(&target.to_le_bytes());
            }
            writer.write_compressed_named(&WINDOWS_1252.decode(name).0, &payload);
        }
        put(doc, &path, &writer.into_vec())?;
    }
    Ok(())
}
fn merge_sch(original: &Path, fresh_path: &Path, staged: &Path, id: &str) -> Result<()> {
    if !original.exists() { fs::copy(fresh_path, staged)?; return Ok(()); }
    fs::copy(original, staged)?;
    let mut dest = cfb::open_rw(staged)?;
    let mut fresh = cfb::open_rw(fresh_path)?;
    let mut index = sch_index(&mut dest)?;
    let new = sch_index(&mut fresh)?;
    let fonts = merge_fonts(&mut index.header, &new.header)?;
    remap_component_fonts(&mut fresh, id, &fonts)?;
    let mut pairs = Vec::new();
    let mut mapped_key = None;
    for i in 0..number(&index.mappings, "KeyCount")? {
        let name = value(&index.mappings, &format!("LibRef{i}")).unwrap().to_string();
        let key = value(&index.mappings, &format!("SectionKey{i}")).unwrap().to_string();
        if name.eq_ignore_ascii_case(id) { mapped_key = Some(key); }
        else { pairs.push((name, key)); }
    }
    let position = index.names.iter().enumerate().position(|(i,name)| index.references[i].eq_ignore_ascii_case(id) || name.eq_ignore_ascii_case(id) || mapped_key.as_ref().is_some_and(|key| name.eq_ignore_ascii_case(key)));
    let old_key = position.map(|index_pos| index.names[index_pos].clone());
    copy_component(&mut fresh, &mut dest, id, old_key.as_deref())?;
    let position = if let Some(position) = position { index.names[position] = id.into(); position }
        else { let position = index.names.len(); index.names.push(id.into()); position };
    set(&mut index.header, &format!("PARTCOUNT{position}"), value(&new.header, "PARTCOUNT0").unwrap_or("2"));
    set(&mut index.header, &format!("LIBREF{position}"), id);
    set(&mut index.header, "COMPCOUNT", index.names.len());
    if index.indexed { set(&mut index.header, "WEIGHT", index.names.len()); }
    let mut writer = BinWriter::new();
    writer.write_block_raw(0, &parameters::patch(&index.raw_header, &index.header)?);
    let header = if index.indexed { encode_index(&writer.into_vec(), &index.names) } else { writer.into_vec() };
    put(&mut dest, "/FileHeader", &header)?;
    if !pairs.is_empty() {
        let mut mappings = vec![("KeyCount".into(), pairs.len().to_string())];
        for (index, (name, key)) in pairs.iter().enumerate() { mappings.push((format!("LibRef{index}"), name.clone())); mappings.push((format!("SectionKey{index}"), key.clone())); }
        put(&mut dest, "/SectionKeys", &encode_params(&mappings))?;
    } else if dest.is_stream("/SectionKeys") { dest.remove_stream("/SectionKeys")?; }
    dest.flush()?;
    sch_index(&mut dest)?;
    Ok(())
}
fn inflate_model(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut value = Vec::new();
    flate2::read::ZlibDecoder::new(bytes).take(64 * 1024 * 1024 + 1).read_to_end(&mut value)
        .map_err(|_| invalid("PCB 内嵌模型压缩数据无效"))?;
    if value.len() > 64 * 1024 * 1024 { return Err(invalid("PCB 单个模型展开后超过 64 MiB")); }
    Ok(value)
}
fn model_guid() -> String {
    let mut hex = String::new();
    for _ in 0..4 {
        let mut hash = 0x811c9dc5u32;
        for byte in crate::util::unique_id().bytes() { hash = (hash ^ byte as u32).wrapping_mul(0x01000193); }
        hex.push_str(&format!("{hash:08X}"));
    }
    format!("{{{}-{}-{}-{}-{}}}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}
fn merge_pcb(original: &Path, fresh_path: &Path, staged: &Path, id: &str) -> Result<()> {
    if !original.exists() { fs::copy(fresh_path, staged)?; return Ok(()); }
    fs::copy(original, staged)?;
    let mut dest = cfb::open_rw(staged)?;
    let mut fresh = cfb::open_rw(fresh_path)?;
    let mut index = pcb_index(&mut dest)?;
    let new = pcb_index(&mut fresh)?;
    let mut fresh_component = stream(&mut fresh, &format!("/{id}/Data"))?;
    for (mut model, compressed) in new.models {
        let original_id = value(&model, "ID").unwrap().to_string();
        let bytes = inflate_model(&compressed)?;
        let mut reusable = None;
        for (old, old_compressed) in &index.models {
            if *old_compressed == compressed || inflate_model(old_compressed)? == bytes {
                reusable = Some(value(old, "ID").unwrap().to_string()); break;
            }
        }
        let target_id = if let Some(existing) = reusable { existing } else {
            let target = if index.models.iter().any(|(old, _)| value(old, "ID").is_some_and(|old_id| old_id.eq_ignore_ascii_case(&original_id))) { model_guid() } else { original_id.clone() };
            set(&mut model, "ID", &target);
            index.models.push((model, compressed));
            target
        };
        if target_id != original_id {
            // GUIDs have equal lengths; replacing bytes preserves all binary record offsets.
            if target_id.len() != original_id.len() { return Err(invalid("模型 GUID 长度不支持")); }
            replace_bytes(&mut fresh_component, original_id.as_bytes(), target_id.as_bytes());
        }
    }
    put(&mut fresh, &format!("/{id}/Data"), &fresh_component)?;
    let mapped_key = index.mappings.iter().find(|(name, _)| name.eq_ignore_ascii_case(id)).map(|(_, key)| key.clone());
    let position = index.names.iter().position(|name| name.eq_ignore_ascii_case(id) || mapped_key.as_ref().is_some_and(|key| key.eq_ignore_ascii_case(name)));
    let old_key = position.map(|position| index.names[position].clone());
    copy_component(&mut fresh, &mut dest, id, old_key.as_deref())?;
    if let Some(position) = position { index.names[position] = id.into(); index.toc[position] = new.toc[0].clone(); }
    else { index.names.push(id.into()); index.toc.push(new.toc[0].clone()); }
    index.mappings.retain(|(name, _)| !name.eq_ignore_ascii_case(id));
    put(&mut dest, "/Library/Data", &encode_index(&index.prefix, &index.names))?;
    put(&mut dest, "/Library/ComponentParamsTOC/Header", &(index.toc.len() as u32).to_le_bytes())?;
    put(&mut dest, "/Library/ComponentParamsTOC/Data", &index.toc.concat())?;
    if !index.mappings.is_empty() {
        let mut writer = BinWriter::new();
        writer.write_u32(index.mappings.len() as u32);
        for (name, key) in &index.mappings { writer.write_block(0, |writer| { writer.write_pascal_short(name); writer.write_u8(0); }); writer.write_string_block(key); }
        put(&mut dest, "/SectionKeys", &writer.into_vec())?;
    } else if dest.is_stream("/SectionKeys") { dest.remove_stream("/SectionKeys")?; }
    let mut writer = BinWriter::new();
    for (index, (model, compressed)) in index.models.iter().enumerate() {
        // Model blocks do not use a leading pipe in the native PCB format.
        let raw = model.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("|");
        writer.write_block(0, |writer| writer.write_cstring(&raw));
        put(&mut dest, &format!("/Library/Models/{index}"), compressed)?;
    }
    put(&mut dest, "/Library/Models/Header", &(index.models.len() as u32).to_le_bytes())?;
    put(&mut dest, "/Library/Models/Data", &writer.into_vec())?;
    dest.flush()?;
    pcb_index(&mut dest)?;
    Ok(())
}
fn replace_bytes(bytes: &mut [u8], old: &[u8], new: &[u8]) {
    if old.is_empty() || old.len() != new.len() { return; }
    let mut start = 0;
    while start + old.len() <= bytes.len() {
        if bytes[start..start + old.len()] == *old { bytes[start..start + old.len()].copy_from_slice(new); start += old.len(); }
        else { start += 1; }
    }
}

struct Original { bytes: Option<Vec<u8>>, _handle: Option<File> }
fn original(path: &Path) -> Result<Original> {
    if !path.exists() { return Ok(Original { bytes: None, _handle: None }); }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Permit reads/rename, deny concurrent writes while the staged replacement is built.
        options.share_mode(0x00000001 | 0x00000004);
    }
    let mut handle = options.open(path).map_err(|error| io_context("打开", error))?;
    if handle.metadata()?.len() > MAX_FILE { return Err(invalid("库文件超过 256 MiB")); }
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes)?;
    Ok(Original { bytes: Some(bytes), _handle: Some(handle) })
}
fn journal_restore(journal: &Path, sch: &Path, pcb: &Path) -> Result<()> {
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(journal.join("manifest.json"))?)?;
    let mut originals = Vec::new();
    // Validate the entire journal before changing either original.
    for (kind, path) in [("sch", sch), ("pcb", pcb)] {
        let existed = manifest.get(kind).and_then(serde_json::Value::as_bool).ok_or_else(|| invalid("事务恢复记录无效"))?;
        let expected = if existed { Some(fs::read(journal.join(format!("{kind}.old")))?) } else { None };
        originals.push((kind, path, expected));
    }
    for (kind, path, expected) in originals {
        if let Some(expected) = expected {
            if fs::read(path).ok().as_ref() != Some(&expected) {
                let staged = journal.join(format!("{kind}.restore"));
                persist_file(&staged, &expected)?;
                fs::rename(&staged, path).map_err(|error| io_context("恢复原件", error))?;
            }
        } else if path.exists() { fs::remove_file(path).map_err(|error| io_context("恢复新建文件", error))?; }
    }
    fs::remove_dir_all(journal)?;
    Ok(())
}
fn persist_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn check_original(path: &Path, expected: &Original) -> Result<()> {
    let current = if path.exists() { Some(fs::read(path)?) } else { None };
    if current != expected.bytes { return Err(Error::msg("绑定 AD 库在同步期间被其它程序修改；已取消写入，请关闭 AD 后重试")); }
    Ok(())
}
fn replace(staged: &Path, target: &Path) -> Result<()> {
    // std::fs::rename replaces an existing file on Windows too. Deleting first
    // leaves a missing destination if the following rename fails.
    fs::rename(staged, target).map_err(|error| io_context("替换", error))
}

/// Update one stable supplier ID in a bound .SchLib and its matching .PcbLib.
/// Unrelated components, custom streams, icon storage and existing models are retained.
pub fn update(sch_path: &Path, mut entry: Entry, colors: SchColors) -> Result<(PathBuf, PathBuf)> {
    let pcb_path = paired(sch_path)?;
    if !crate::models::looks_like_lcsc_token(&entry.id) || (entry.symbol.is_none() && entry.footprint.is_none()) {
        return Err(Error::msg("绑定 AD 库需要明确的 C 编号及符号或封装"));
    }
    entry.id.make_ascii_uppercase();
    let parent = sch_path.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_context("创建目录", error))?;
    let stem = sch_path.file_name().unwrap().to_string_lossy();
    let lock_path = parent.join(format!(".{stem}.jlc-lock"));
    let _lock = crate::library_lock::acquire(&lock_path)?;
    let journal = parent.join(format!(".{stem}.jlc-transaction"));
    if journal.exists() { journal_restore(&journal, sch_path, &pcb_path)?; }
    validate_binding(sch_path)?;
    let mut original_sch = original(sch_path)?;
    let mut original_pcb = original(&pcb_path)?;
    let working = parent.join(format!(".{stem}.jlc-stage-{}", crate::util::unique_id()));
    fs::create_dir(&working)?;
    let result = (|| -> Result<()> {
        let sch_fresh = working.join("new.SchLib");
        let pcb_fresh = working.join("new.PcbLib");
        let sch_stage = working.join("ready.SchLib");
        let pcb_stage = working.join("ready.PcbLib");
        if let Some(symbol) = &mut entry.symbol {
            symbol.name = entry.id.clone();
            if entry.footprint.is_some() { symbol.meta.footprint_lib = entry.id.clone(); }
            altium::write_schlib_with_colors(&sch_fresh, symbol, colors)?;
            merge_sch(sch_path, &sch_fresh, &sch_stage, &entry.id)?;
        }
        if let Some(footprint) = &mut entry.footprint {
            footprint.name = entry.id.clone();
            altium::write_pcblib_with_step(&pcb_fresh, footprint, entry.step.as_deref())?;
            merge_pcb(&pcb_path, &pcb_fresh, &pcb_stage, &entry.id)?;
        }
        check_original(sch_path, &original_sch)?;
        check_original(&pcb_path, &original_pcb)?;
        let prepared = working.join("backup");
        fs::create_dir(&prepared)?;
        for (kind, original) in [("sch", &original_sch), ("pcb", &original_pcb)] {
            if let Some(bytes) = &original.bytes { persist_file(&prepared.join(format!("{kind}.old")), bytes)?; }
        }
        persist_file(&prepared.join("manifest.json"), &serde_json::to_vec(&serde_json::json!({
            "sch": original_sch.bytes.is_some(), "pcb": original_pcb.bytes.is_some()
        }))?)?;
        fs::rename(&prepared, &journal)?;
        // Windows replacement needs these handles closed, including on WSL
        // shares. The sidecar lock still excludes other synchronization tasks.
        drop(original_sch._handle.take());
        drop(original_pcb._handle.take());
        if sch_stage.exists() { replace(&sch_stage, sch_path)?; }
        if pcb_stage.exists() { replace(&pcb_stage, &pcb_path)?; }
        let previous = parent.join(format!(".{stem}.jlc-backup"));
        if previous.exists() { fs::remove_dir_all(&previous)?; }
        fs::rename(&journal, previous)?;
        Ok(())
    })();
    drop(original_sch._handle.take());
    drop(original_pcb._handle.take());
    if result.is_err() && journal.exists() {
        if let Err(error) = journal_restore(&journal, sch_path, &pcb_path) {
            let _ = fs::remove_dir_all(&working);
            return Err(Error::msg(format!("分类库同步失败且恢复未完成：{error}。原件备份保存在 {}，请关闭 AD 后重试。", journal.display())));
        }
    }
    let _ = fs::remove_dir_all(working);
    result?;
    Ok((sch_path.to_path_buf(), pcb_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{easyeda, ir::{self, PartMeta}};
    const SYMBOL: &str = include_str!("../tests/fixtures/FNR3015S2R2MT_symbol_easyeda.json");
    const FOOTPRINT: &str = include_str!("../tests/fixtures/FNR3015S2R2MT_footprint_easyeda.json");
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("lceda-bound-ad-{}-{}", std::process::id(), crate::util::unique_id()));
            fs::create_dir_all(&path).unwrap(); Self(path)
        }
    }
    impl Drop for Directory { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    fn entry(id: &str, mpn: &str) -> Entry {
        let metadata = PartMeta { lcsc: id.into(), mpn: mpn.into(), ..Default::default() };
        Entry {
            id: id.into(),
            symbol: Some(ir::symbol_ir("generic", "fixture", easyeda::parse_symbol(&serde_json::from_str(SYMBOL).unwrap()).unwrap(), metadata.clone())),
            footprint: Some(ir::footprint_ir("generic", "fixture", easyeda::parse_footprint(&serde_json::from_str(FOOTPRINT).unwrap()).unwrap(), metadata)),
            step: Some(format!("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('{id}'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;").into_bytes()),
        }
    }
    fn component_bytes(path: &Path, key: &str) -> Vec<u8> { stream(&mut open(path).unwrap(), &format!("/{key}/Data")).unwrap() }

    #[test]
    fn failed_replacement_leaves_the_existing_file() {
        let directory = Directory::new();
        let target = directory.0.join("Original.SchLib");
        fs::write(&target, b"original bytes").unwrap();
        assert!(replace(&directory.0.join("missing-stage"), &target).is_err());
        assert_eq!(fs::read(target).unwrap(), b"original bytes");
    }

    #[test]
    fn native_mixed_encoding_header_without_trailing_index_retains_all_old_fields() {
        let directory = Directory::new();
        let sch = directory.0.join("Native.SchLib");
        let source = directory.0.join("Source.SchLib");
        altium::write_schlib(&source, entry("C1", "Source").symbol.as_ref().unwrap()).unwrap();
        let data = component_bytes(&source, "generic");
        let mut header = b"|HEADER=Protel for Windows - Schematic Library Editor Binary File Version 5.0|Weight=12026|MinorVersion=9|FontIdCount=2|Size1=10|FontName1=Times New Roman|Size2=8|Rotation2=270|%UTF8%FontName2=".to_vec();
        header.extend_from_slice("楷体".as_bytes());
        header.extend_from_slice(b"|||FontName2=\xbf\xac\xcc\xe5|UseMBCS=T|CompCount=3|LibRef0=RES|PartCount0=2|LibRef1=USB-C/S|PartCount1=2|%UTF8%LibRef2=");
        header.extend_from_slice("共模电感".as_bytes());
        header.extend_from_slice(b"|||LibRef2=\xb9\xb2\xc4\xa3\xb5\xe7\xb8\xd0|PartCount2=2|%UTF8%CompDescr2=");
        header.extend_from_slice("中文参数".as_bytes()); header.extend_from_slice(b"|||CompDescr2=\xd6\xd0\xce\xc4\xb2\xce\xca\xfd\0");
        let mut doc = cfb::create(&sch).unwrap();
        let mut writer = BinWriter::new(); writer.write_block_raw(0, &header);
        put(&mut doc, "/FileHeader", &writer.into_vec()).unwrap();
        for key in ["RES", "USB-C_S", "共模电感"] { put(&mut doc, &format!("/{key}/Data"), &data).unwrap(); }
        put(&mut doc, "/Storage", b"embedded user icons").unwrap(); doc.flush().unwrap(); drop(doc);
        validate_binding(&sch).unwrap();
        for _ in 0..2 { update(&sch, entry("C2040", "RP2040"), SchColors::default()).unwrap(); }
        let mut doc = open(&sch).unwrap(); let index = sch_index(&mut doc).unwrap();
        assert!(!index.indexed); assert_eq!(index.names, ["RES", "USB-C_S", "共模电感", "C2040"]);
        assert_eq!(value(&index.header, "FONTNAME2"), Some("楷体"));
        assert_eq!(value(&index.header, "COMPDESCR2"), Some("中文参数"));
        assert_eq!(value(&index.header, "WEIGHT"), Some("12026"));
        for field in header.strip_suffix(&[0]).unwrap().split(|b| *b == b'|').filter(|v| !v.is_empty() && !v.starts_with(b"CompCount=") && !v.starts_with(b"FontIdCount=")) {
            assert!(index.raw_header.windows(field.len()).any(|bytes| bytes == field), "untouched header field must keep its original bytes");
        }
        for key in ["RES", "USB-C_S", "共模电感"] { assert_eq!(stream(&mut doc, &format!("/{key}/Data")).unwrap(), data); }
        assert_eq!(stream(&mut doc, "/Storage").unwrap(), b"embedded user icons");
    }

    #[test]
    fn adds_and_updates_same_named_parts_without_losing_models_binding_or_comment() {
        let directory = Directory::new();
        let sch = directory.0.join("Selected.SchLib");
        let pcb = sch.with_extension("PcbLib");
        let first = entry("C2040", "BAT54");
        let second = entry("C9999", "Another-BAT54");
        update(&sch, first.clone(), SchColors::default()).unwrap();
        let retained = component_bytes(&sch, "C2040");
        update(&sch, second, SchColors::easyeda()).unwrap();
        assert_eq!(retained, component_bytes(&sch, "C2040"), "unrelated component data must stay byte-identical");
        let mut refreshed = first;
        refreshed.symbol.as_mut().unwrap().description = "revised".into();
        update(&sch, refreshed, SchColors::default()).unwrap();
        let mut doc = open(&sch).unwrap();
        let index = sch_index(&mut doc).unwrap();
        assert_eq!(index.names, ["C2040", "C9999"]);
        for (id, mpn) in [("C2040", "BAT54"), ("C9999", "Another-BAT54")] {
            let bytes = component_bytes(&sch, id);
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains(&format!("|TEXT={mpn}|NAME=Comment|")));
            assert!(text.contains(&format!("|MODELNAME={id}|")));
            let bytes = component_bytes(&pcb, id);
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains("MODEL.EMBED=TRUE"));
            assert!(text.contains("MODEL.3D.ROTZ=90.000"));
        }
        let mut pcb_doc = open(&pcb).unwrap();
        let pcb_index = pcb_index(&mut pcb_doc).unwrap();
        assert_eq!(pcb_index.names.len(), 2);
        assert_eq!(pcb_index.models.len(), 2, "same-ID same-STEP refresh must reuse model payload");
        let actual: HashSet<_> = pcb_index.models.iter().map(|(_, compressed)| inflate_model(compressed).unwrap()).collect();
        assert!(actual.contains(&entry("C2040", "BAT54").step.unwrap()));
        assert!(actual.contains(&entry("C9999", "Another-BAT54").step.unwrap()));
        assert!(directory.0.join(".Selected.SchLib.jlc-backup/manifest.json").is_file());
    }

    #[test]
    fn preserves_preexisting_custom_components_streams_and_section_key_mapping() {
        let directory = Directory::new();
        let sch = directory.0.join("Custom.SchLib");
        let pcb = sch.with_extension("PcbLib");
        let mut custom = entry("C1000", "Handcrafted-MPN");
        // A name that requires SectionKeys verifies unrelated alias maps remain intact.
        custom.symbol.as_mut().unwrap().name = "Custom/Original".into();
        custom.footprint.as_mut().unwrap().name = "Custom/Original".into();
        altium::write_schlib_with_colors(&sch, custom.symbol.as_ref().unwrap(), SchColors::default()).unwrap();
        altium::write_pcblib_with_step(&pcb, custom.footprint.as_ref().unwrap(), custom.step.as_deref()).unwrap();
        let mut old_sch = cfb::open_rw(&sch).unwrap();
        put(&mut old_sch, "/Storage", b"retained custom icon bytes").unwrap();
        put(&mut old_sch, "/Unrelated/Nested/CustomData", b"sch opaque data").unwrap();
        old_sch.flush().unwrap();
        let old_sch_index = sch_index(&mut old_sch).unwrap();
        let old_sch_component = stream(&mut old_sch, &format!("/{}/Data", old_sch_index.names[0])).unwrap();
        let old_sch_mapping = stream(&mut old_sch, "/SectionKeys").unwrap();
        drop(old_sch);
        let mut old_pcb = cfb::open_rw(&pcb).unwrap();
        put(&mut old_pcb, "/UserOpaque/Settings", b"pcb opaque data").unwrap();
        old_pcb.flush().unwrap();
        let old_pcb_index = pcb_index(&mut old_pcb).unwrap();
        let old_pcb_component = stream(&mut old_pcb, &format!("/{}/Data", old_pcb_index.names[0])).unwrap();
        let old_model = stream(&mut old_pcb, "/Library/Models/0").unwrap();
        drop(old_pcb);
        update(&sch, entry("C2040", "BAT54"), SchColors::easyeda()).unwrap();
        let mut merged_sch = open(&sch).unwrap();
        assert_eq!(stream(&mut merged_sch, "/Storage").unwrap(), b"retained custom icon bytes");
        assert_eq!(stream(&mut merged_sch, "/Unrelated/Nested/CustomData").unwrap(), b"sch opaque data");
        assert_eq!(stream(&mut merged_sch, &format!("/{}/Data", old_sch_index.names[0])).unwrap(), old_sch_component);
        assert_eq!(stream(&mut merged_sch, "/SectionKeys").unwrap(), old_sch_mapping);
        let mut merged_pcb = open(&pcb).unwrap();
        assert_eq!(stream(&mut merged_pcb, "/UserOpaque/Settings").unwrap(), b"pcb opaque data");
        assert_eq!(stream(&mut merged_pcb, &format!("/{}/Data", old_pcb_index.names[0])).unwrap(), old_pcb_component);
        assert_eq!(stream(&mut merged_pcb, "/Library/Models/0").unwrap(), old_model);
        assert_eq!(pcb_index(&mut merged_pcb).unwrap().names.len(), 2);
        validate_binding(&sch).unwrap();
    }

    #[test]
    fn rejects_invalid_existing_library_without_touching_either_file() {
        let directory = Directory::new();
        let sch = directory.0.join("Invalid.SchLib");
        let pcb = sch.with_extension("PcbLib");
        fs::write(&sch, b"user file that is not an AD library").unwrap();
        fs::write(&pcb, b"keep paired file too").unwrap();
        assert!(validate_binding(&sch).is_err());
        assert!(update(&sch, entry("C2040", "BAT54"), SchColors::default()).is_err());
        assert_eq!(fs::read(&sch).unwrap(), b"user file that is not an AD library");
        assert_eq!(fs::read(&pcb).unwrap(), b"keep paired file too");
        let missing = directory.0.join("AllowedNew.SchLib");
        validate_binding(&missing).unwrap();
        assert!(!missing.exists());
        assert!(validate_binding(&directory.0.join("wrong.txt")).is_err());
    }

    #[test]
    fn remaps_added_symbol_fonts_and_keeps_existing_font_references_unchanged() {
        let directory = Directory::new();
        let sch = directory.0.join("Fonts.SchLib");
        update(&sch, entry("C2040", "BAT54"), SchColors::easyeda()).unwrap();
        let first = component_bytes(&sch, "C2040");
        let colors = SchColors { pin_name: SchColors::rgb(17, 34, 51), pin_number: SchColors::rgb(68, 85, 102), pin_font_size: 13, ..SchColors::default() };
        update(&sch, entry("C9999", "Another"), colors).unwrap();
        assert_eq!(first, component_bytes(&sch, "C2040"));
        let mut doc = open(&sch).unwrap();
        let index = sch_index(&mut doc).unwrap();
        let count = number(&index.header, "FONTIDCOUNT").unwrap();
        let bytes = component_bytes(&sch, "C9999");
        let mut offset = 0;
        let mut custom_font_refs = 0;
        while offset < bytes.len() {
            let (flags, data) = block(&bytes, &mut offset).unwrap();
            if flags != 0 || data.first() != Some(&b'|') { continue; }
            let text = String::from_utf8_lossy(data);
            for (name, font) in text.split('|').filter_map(|token| token.split_once('=')) {
                if name == "NAME_CUSTOMFONTID" || name == "DESIGNATOR_CUSTOMFONTID" {
                    let id: usize = font.parse().unwrap();
                    assert!(id <= count);
                    assert_eq!(value(&index.header, &format!("SIZE{id}")), Some("13"));
                    custom_font_refs += 1;
                }
            }
        }
        assert!(custom_font_refs > 0);
        update(&sch, entry("C9999", "Another"), colors).unwrap();
        let mut doc = open(&sch).unwrap();
        assert_eq!(number(&sch_index(&mut doc).unwrap().header, "FONTIDCOUNT").unwrap(), count, "repeated updates must not grow duplicate fonts");
    }
    #[test]
    fn changed_step_keeps_unrelated_models_and_rebinds_updated_body_to_new_payload() {
        let directory = Directory::new();
        let sch = directory.0.join("Models.SchLib");
        let original = entry("C2040", "BAT54");
        update(&sch, original.clone(), SchColors::default()).unwrap();
        update(&sch, entry("C9999", "Another"), SchColors::default()).unwrap();
        let pcb = sch.with_extension("PcbLib");
        let unrelated = component_bytes(&pcb, "C9999");
        let mut refreshed = original;
        refreshed.step = Some(b"ISO-10303-21;\nDATA;\n/* changed geometry */\nENDSEC;\nEND-ISO-10303-21;".to_vec());
        update(&sch, refreshed.clone(), SchColors::default()).unwrap();
        assert_eq!(component_bytes(&pcb, "C9999"), unrelated);
        let mut doc = open(&pcb).unwrap();
        let index = pcb_index(&mut doc).unwrap();
        assert_eq!(index.names.len(), 2);
        let (parameters, _) = index.models.iter().find(|(_, compressed)| inflate_model(compressed).unwrap() == *refreshed.step.as_ref().unwrap()).expect("new STEP payload must exist");
        let new_guid = value(parameters, "ID").unwrap();
        let bytes = component_bytes(&pcb, "C2040");
        assert!(String::from_utf8_lossy(&bytes).contains(&format!("MODELID={new_guid}|")), "updated body must reference the new model rather than the preserved old payload");
    }

    #[test]
    fn recovers_interrupted_pair_replacement_and_respects_process_lock() {
        let directory = Directory::new();
        let sch = directory.0.join("Recovery.SchLib");
        let pcb = sch.with_extension("PcbLib");
        update(&sch, entry("C2040", "BAT54"), SchColors::default()).unwrap();
        let before_sch = fs::read(&sch).unwrap();
        let before_pcb = fs::read(&pcb).unwrap();
        let journal = directory.0.join(".Recovery.SchLib.jlc-transaction");
        fs::create_dir(&journal).unwrap();
        fs::write(journal.join("sch.old"), &before_sch).unwrap();
        fs::write(journal.join("pcb.old"), &before_pcb).unwrap();
        fs::write(journal.join("manifest.json"), br#"{"sch":true,"pcb":true}"#).unwrap();
        fs::write(&sch, b"interrupted partial replacement").unwrap();
        update(&sch, entry("C9999", "Another"), SchColors::default()).unwrap();
        assert!(!journal.exists());
        let mut doc = open(&sch).unwrap();
        assert_eq!(sch_index(&mut doc).unwrap().names.len(), 2);
        drop(doc);
        let lock = OpenOptions::new().read(true).write(true).open(directory.0.join(".Recovery.SchLib.jlc-lock")).unwrap();
        lock.try_lock().unwrap();
        let original = fs::read(&sch).unwrap();
        assert!(update(&sch, entry("C8888", "Blocked"), SchColors::default()).is_err());
        assert_eq!(fs::read(&sch).unwrap(), original);
    }

}
