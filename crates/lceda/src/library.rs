use lceda_core::models::{DownloadPaths, SearchItem};
use std::collections::BTreeMap;
use serde_json::{json, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};
use serde::{Deserialize, Serialize};
use lceda_core::native_library::{self,NativeLibrary,NativePart};
pub use lceda_core::native_library::LibraryFormat;

pub const UNCAT: &str = "uncat";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeRecord {
    pub format: LibraryFormat,
    pub source: PathBuf,
    pub library: PathBuf,
    pub part: NativePart,
}

#[derive(Debug, Clone)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub binding: Option<PathBuf>,
    pub kicad_binding: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SavedPart {
    pub lcsc: String,
    pub name: String,
    pub manufacturer: String,
    pub cat_id: String,
}

/// A durable local snapshot. Files are checked on use, rather than trusting stale flags.
#[derive(Debug, Clone, Default)]
pub struct PartRecord {
    pub native: Option<NativeRecord>,
    pub supplements: Vec<lceda_core::supplement::ResourceBundle>,
    pub notes: String,
    pub package: String,
    pub parameters: BTreeMap<String, String>,
    pub resources: BTreeMap<String, PathBuf>,
    pub snapshot: Option<SearchItem>,
    pub updated_at: u64,
}

#[cfg(test)]
impl PartRecord {
    pub fn has_symbol(&self) -> bool {
        ["schlib", "kicad_sym", "pads_c", "symbol_json"].iter()
            .any(|key| self.resources.get(*key).is_some_and(|p| p.is_file()))
    }
}

#[derive(Debug, Clone)]
pub struct Library {
    pub cats: Vec<Category>,
    pub favs: Vec<SavedPart>,
    pub queue: Vec<SavedPart>,
    pub records: BTreeMap<String, PartRecord>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            cats: vec![Category {
                id: UNCAT.into(),
                name: "默认收藏夹".into(),
                parent: None,
                binding: None,
                kicad_binding: None,
            }],
            favs: Vec::new(),
            queue: Vec::new(),
            records: BTreeMap::new(),
        }
    }
}

impl Library {
    pub fn load() -> Self {
        let Some(path) = library_path() else {
            return Self::default();
        };
        for candidate in [&path, &path.with_extension("json.bak")] {
            if let Ok(text) = fs::read_to_string(candidate) {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if v.is_object() { return Self::from_json(&v); }
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            eprintln!("保存本地器件库失败: {error}");
        }
    }

    pub fn try_save(&self) -> std::io::Result<()> {
        let path = library_path().ok_or_else(|| std::io::Error::other("无法确定配置目录"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Keep a recoverable old copy and write all bytes before replacing the live file.
        let temporary = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut file = fs::File::create(&temporary)?;
            file.write_all(self.to_json().to_string().as_bytes())?;
            file.sync_all()?;
        }
        if path.exists() {
            let valid = fs::read_to_string(&path).ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some_and(|v| v.is_object());
            if valid { fs::copy(&path, path.with_extension("json.bak"))?; }
        }
        #[cfg(windows)]
        if path.exists() { fs::remove_file(&path)?; }
        fs::rename(temporary, path)
    }

    pub fn enrich_from_item(&mut self, item: &SearchItem) {
        let Some(lcsc) = item.lcsc_id() else { return; };
        let record = self.records.entry(lcsc).or_default();
        if let Some(attrs) = item.raw.get("attributes").and_then(Value::as_object) {
            for (key, value) in attrs {
                let value = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
                record.parameters.insert(key.clone(), value);
            }
        }
        record.package = package_from_item(item);
        record.snapshot = Some(item.clone());
        record.updated_at = timestamp();
    }

    pub fn record_export(&mut self, lcsc: &str, paths: &DownloadPaths) {
        let record = self.records.entry(lcsc.to_string()).or_default();
        for (kind, path) in [
            ("step", &paths.step), ("obj", &paths.obj), ("mtl", &paths.mtl),
            ("symbol_json", &paths.symbol_json), ("footprint_json", &paths.footprint_json),
            ("schlib", &paths.schlib), ("pcblib", &paths.pcblib),
            ("kicad_sym", &paths.kicad_sym), ("kicad_mod", &paths.kicad_mod),
            ("pads_c", &paths.pads_c), ("pads_d", &paths.pads_d), ("pads_p", &paths.pads_p),
            ("datasheet", &paths.datasheet),
        ] {
            if let Some(path) = path {
                let absolute = if path.is_absolute() { path.clone() } else {
                    std::env::current_dir().map(|cwd| cwd.join(path)).unwrap_or_else(|_| path.clone())
                };
                record.resources.insert(kind.to_string(), absolute);
            }
        }
        record.updated_at = timestamp();
    }

    pub fn cached_item(&self, lcsc: &str) -> Option<SearchItem> {
        self.records.get(lcsc)?.snapshot.clone()
    }

    pub fn record(&self, lcsc: &str) -> Option<&PartRecord> {
        self.records.get(lcsc)
    }

    pub fn native(&self,id:&str)->Option<&NativeRecord>{self.records.get(id)?.native.as_ref()}

    pub fn add_supplement(&mut self, id: &str, bundle: lceda_core::supplement::ResourceBundle) -> Result<(), String> {
        let record = self.records.get_mut(id).ok_or("补全器件已从器件库移除")?;
        let native = record.native.as_ref().ok_or("仅支持给导入的原库器件补充资源")?;
        if native.part.key != bundle.native_key || native.format != bundle.format { return Err("补充资源与原器件不一致".into()); }
        record.supplements.push(bundle); record.updated_at = timestamp(); Ok(())
    }

    /// Keep external-library identities separate from supplier IDs, including
    /// symbols literally named C2040. Reimporting the same source updates rows.
    pub fn add_native_parts(&mut self,info:&NativeLibrary,keys:&[String],name:&str,snapshot:&Path)->Result<(String,usize),String>{
        if name.trim().is_empty()||keys.is_empty(){return Err("请填写库名并勾选器件".into());}
        let source=fs::canonicalize(&info.source).unwrap_or_else(|_|info.source.clone());
        let identity=source.to_string_lossy();let identity=if cfg!(windows){identity.to_lowercase()}else{identity.into_owned()};
        let mut rows=Vec::new();
        for key in keys {
            let part=info.parts.iter().find(|p|&p.key==key).ok_or_else(||"所选器件已不在库中".to_string())?;
            let token=format!("{}\n{identity}\n{}",info.format.name(),part.key);
            let id=format!("LOCAL-{:016x}",native_library::content_hash(token.as_bytes()));rows.push((id,part.clone()));
        }
        let existing=rows.iter().find_map(|(id,_)|self.favs.iter().find(|p|&p.lcsc==id).map(|p|p.cat_id.clone()));
        let category=if let Some(id)=existing{ id }else{
            let mut title=name.trim().to_string();let mut suffix=2;
            while self.cats.iter().any(|c|c.name==title){title=format!("{} ({suffix})",name.trim());suffix+=1;}
            self.add_root(&title).ok_or_else(||"无法创建器件库".to_string())?
        };
        for (id,part) in rows {
            let manufacturer=part.properties.iter().find(|(k,_)|k.eq_ignore_ascii_case("Manufacturer")).map(|(_,v)|v.clone()).unwrap_or_default();
            self.add_fav(SavedPart{lcsc:id.clone(),name:part.name.clone(),manufacturer,cat_id:category.clone()});
            let record=self.records.entry(id).or_default();record.parameters=part.properties.clone();
            record.package=part.footprints.join("; ");record.native=Some(NativeRecord{format:info.format,source:info.source.clone(),library:snapshot.to_path_buf(),part});
            record.updated_at=timestamp();
        }
        Ok((category,keys.len()))
    }

    #[cfg(test)]
    pub fn set_notes(&mut self, lcsc: &str, notes: String) {
        let record = self.records.entry(lcsc.to_string()).or_default();
        record.notes = notes;
        record.updated_at = timestamp();
    }

    fn from_json(v: &Value) -> Self {
        let mut lib = Self::default();
        if let Some(cats) = v.get("cats").and_then(Value::as_array) {
            let parsed: Vec<Category> = cats
                .iter()
                .filter_map(|c| {
                    Some(Category {
                        id: c.get("id")?.as_str()?.to_string(),
                        name: c.get("name")?.as_str()?.to_string(),
                        parent: c
                            .get("parent")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string()),
                        binding: None,
                        kicad_binding: None,
                    })
                })
                .collect();
            if !parsed.is_empty() {
                lib.cats = parsed;
            }
            if !lib.cats.iter().any(|c| c.id == UNCAT) {
                lib.cats.insert(
                    0,
                    Category {
                        id: UNCAT.into(),
                        name: "默认收藏夹".into(),
                        parent: None,
                        binding: None,
                        kicad_binding: None,
                    },
                );
            }
        }
        // Reuse validation so malformed or conflicting persisted bindings cannot target the same file.
        if let Some(cats) = v.get("cats").and_then(Value::as_array) {
            for category in cats {
                for (format, key) in [(LibraryFormat::Ad, "binding"), (LibraryFormat::KiCad, "kicad_binding")] {
                    if let (Some(id), Some(path)) = (category.get("id").and_then(Value::as_str), category.get(key).and_then(Value::as_str)) {
                        if let Err(error) = lib.set_binding(id, format, Path::new(path)) {
                            eprintln!("忽略无效分类库绑定 {id}: {error}");
                        }
                    }
                }
            }
        }
        lib.favs = v
            .get("favs")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(parse_part).collect())
            .unwrap_or_default();
        lib.queue = v
            .get("queue")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(parse_part).collect())
            .unwrap_or_default();
        if let Some(records) = v.get("records").and_then(Value::as_object) {
            for (lcsc, data) in records {
                let snapshot = data.get("snapshot").filter(|v| v.is_object()).map(|raw| SearchItem {
                    index: 1,
                    display_title: data.get("name").and_then(Value::as_str).unwrap_or("").into(),
                    title: data.get("title").and_then(Value::as_str).unwrap_or("").into(),
                    manufacturer: data.get("manufacturer").and_then(Value::as_str).unwrap_or("").into(),
                    model_uuid: data.get("model_uuid").and_then(Value::as_str).map(str::to_string),
                    raw: raw.clone(),
                });
                lib.records.insert(lcsc.clone(), PartRecord {
                    native: data.get("native").and_then(|v|serde_json::from_value(v.clone()).ok()),
                    supplements: data.get("supplements").and_then(|v|serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
                    notes: data.get("notes").and_then(Value::as_str).unwrap_or("").into(),
                    package: data.get("package").and_then(Value::as_str).unwrap_or("").into(),
                    parameters: data.get("parameters").and_then(Value::as_object).map(|o| o.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default(),
                    resources: data.get("resources").and_then(Value::as_object).map(|o| o.iter()
                        .filter_map(|(k, v)| Some((k.clone(), PathBuf::from(v.as_str()?)))).collect()).unwrap_or_default(),
                    snapshot,
                    updated_at: data.get("updated_at").and_then(Value::as_u64).unwrap_or(0),
                });
            }
        }
        lib
    }

    fn to_json(&self) -> Value {
        let records: BTreeMap<_, _> = self.records.iter().map(|(lcsc, record)| {
            let item = record.snapshot.as_ref();
            (lcsc, json!({
                "native": record.native,
                "supplements": record.supplements,
                "notes": record.notes,
                "package": record.package,
                "parameters": record.parameters,
                "resources": record.resources,
                "snapshot": item.map(|i| &i.raw),
                "name": item.map(SearchItem::name),
                "title": item.map(|i| &i.title),
                "manufacturer": item.map(|i| &i.manufacturer),
                "model_uuid": item.and_then(|i| i.model_uuid.as_ref()),
                "updated_at": record.updated_at,
            }))
        }).collect();
        json!({
            "version": 2,
            "records": records,
            "cats": self.cats.iter().map(|c| json!({
                "id": c.id,
                "name": c.name,
                "parent": c.parent,
                "binding": c.binding,
                "kicad_binding": c.kicad_binding,
            })).collect::<Vec<_>>(),
            "favs": self.favs.iter().map(part_json).collect::<Vec<_>>(),
            "queue": self.queue.iter().map(part_json).collect::<Vec<_>>(),
        })
    }

    #[allow(dead_code)]
    pub fn roots(&self) -> Vec<&Category> {
        self.cats.iter().filter(|c| c.parent.is_none()).collect()
    }

    #[allow(dead_code)]
    pub fn children<'a>(&'a self, id: &str) -> Vec<&'a Category> {
        self.cats
            .iter()
            .filter(|c| c.parent.as_deref() == Some(id))
            .collect()
    }

    pub fn cat(&self, id: &str) -> Option<&Category> {
        self.cats.iter().find(|c| c.id == id)
    }

    /// Binding is local to a category; child categories do not inherit their parent's target.
    pub fn binding_for_category(&self, cat_id: &str) -> Option<&Path> {
        self.binding_for(cat_id, LibraryFormat::Ad)
    }

    pub fn binding_for(&self, cat_id: &str, format: LibraryFormat) -> Option<&Path> {
        let category = self.cat(cat_id)?;
        match format { LibraryFormat::Ad => category.binding.as_deref(), LibraryFormat::KiCad => category.kicad_binding.as_deref() }
    }

    #[cfg(test)]
    pub fn binding_for_part(&self, lcsc: &str) -> Option<&Path> {
        self.binding_for_part_format(lcsc, LibraryFormat::Ad)
    }

    pub fn binding_for_part_format(&self, lcsc: &str, format: LibraryFormat) -> Option<&Path> {
        let part = self.favs.iter().find(|part| part.lcsc.eq_ignore_ascii_case(lcsc))?;
        self.binding_for(&part.cat_id, format)
    }

    pub fn bound_categories(&self) -> Vec<(&Category, &Path)> {
        self.cats.iter().filter_map(|category| Some((category, category.binding.as_deref().or(category.kicad_binding.as_deref())?))).collect()
    }

    #[cfg(test)]
    pub fn bind_library(&mut self, cat_id: &str, path: &Path) -> Result<(), String> {
        self.set_binding(cat_id, LibraryFormat::Ad, path)
    }

    pub fn set_binding(&mut self, cat_id: &str, format: LibraryFormat, path: &Path) -> Result<(), String> {
        if self.cat(cat_id).is_none() { return Err("没有找到器件库分类".into()); }
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(format.extension())) { return Err(format!("请选择 .{} 库文件", format.extension())); }
        let target = normalize_binding(path)?;
        let identity = binding_identity(&target, cfg!(windows));
        if let Some(other) = self.cats.iter().find(|category| category.id != cat_id &&
            self.binding_for(&category.id, format).is_some_and(|bound| binding_identity(bound, cfg!(windows)) == identity)) {
            return Err(format!("库文件已绑定到分类“{}”，请选择其它文件", other.name));
        }
        let category = self.cats.iter_mut().find(|category| category.id == cat_id).unwrap();
        match format { LibraryFormat::Ad => category.binding = Some(target), LibraryFormat::KiCad => category.kicad_binding = Some(target) }
        Ok(())
    }

    /// Removing a binding never modifies or removes the external library file.
    #[cfg(test)]
    pub fn unbind_library(&mut self, cat_id: &str) -> Result<(), String> {
        self.clear_binding(cat_id, LibraryFormat::Ad)
    }

    pub fn clear_binding(&mut self, cat_id: &str, format: LibraryFormat) -> Result<(), String> {
        let category = self.cats.iter_mut().find(|category| category.id == cat_id)
            .ok_or("没有找到器件库分类")?;
        match format { LibraryFormat::Ad => category.binding = None, LibraryFormat::KiCad => category.kicad_binding = None }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_root(&self, id: &str) -> bool {
        self.cat(id).is_some_and(|c| c.parent.is_none())
    }

    #[allow(dead_code)]
    pub fn can_add_child(&self, id: &str) -> bool {
        id != UNCAT && self.is_root(id)
    }

    pub fn add_root(&mut self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let id = new_id();
        self.cats.push(Category {
            id: id.clone(),
            name: name.into(),
            parent: None,
            binding: None,
            kicad_binding: None,
        });
        Some(id)
    }

    #[allow(dead_code)]
    pub fn add_child(&mut self, parent: &str, name: &str) -> Option<String> {
        if !self.can_add_child(parent) {
            return None;
        }
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let id = new_id();
        self.cats.push(Category {
            id: id.clone(),
            name: name.into(),
            parent: Some(parent.into()),
            binding: None,
            kicad_binding: None,
        });
        Some(id)
    }

    pub fn rename_cat(&mut self, id: &str, name: &str) {
        if id == UNCAT {
            return;
        }
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(c) = self.cats.iter_mut().find(|c| c.id == id) {
            c.name = name.into();
        }
    }

    pub fn remove_cat(&mut self, id: &str) {
        if id == UNCAT {
            return;
        }
        let drop: Vec<String> = self
            .cats
            .iter()
            .filter(|c| c.id == id || c.parent.as_deref() == Some(id))
            .map(|c| c.id.clone())
            .collect();
        self.cats.retain(|c| !drop.contains(&c.id));
        for p in &mut self.favs {
            if drop.contains(&p.cat_id) {
                p.cat_id = UNCAT.into();
            }
        }
    }

    pub fn favs_in(&self, id: &str) -> Vec<&SavedPart> {
        self.favs.iter().filter(|p| p.cat_id == id).collect()
    }

    #[allow(dead_code)]
    pub fn favs_in_tree(&self, id: &str) -> Vec<&SavedPart> {
        let mut ids = vec![id.to_string()];
        ids.extend(self.children(id).into_iter().map(|c| c.id.clone()));
        self.favs
            .iter()
            .filter(|p| ids.iter().any(|id| id == &p.cat_id))
            .collect()
    }

    pub fn add_fav(&mut self, part: SavedPart) -> bool {
        if part.lcsc.is_empty() {
            return false;
        }
        if self.favs.iter().any(|p| p.lcsc == part.lcsc) {
            if let Some(old) = self.favs.iter_mut().find(|p| p.lcsc == part.lcsc) {
                old.cat_id = part.cat_id;
                old.name = part.name;
                old.manufacturer = part.manufacturer;
            }
            return false;
        }
        self.favs.push(part);
        true
    }

    pub fn remove_fav(&mut self, lcsc: &str) {
        self.favs.retain(|p| p.lcsc != lcsc);
    }

    pub fn in_queue(&self, lcsc: &str) -> bool {
        self.queue.iter().any(|p| p.lcsc == lcsc)
    }

    pub fn saved_manufacturer(&self, lcsc: &str) -> Option<&str> {
        self.queue
            .iter()
            .chain(self.favs.iter())
            .find(|p| p.lcsc == lcsc)
            .map(|p| p.manufacturer.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn add_queue(&mut self, part: SavedPart) -> bool {
        if part.lcsc.is_empty() || self.in_queue(&part.lcsc) {
            return false;
        }
        self.queue.push(part);
        true
    }

    pub fn remove_queue(&mut self, lcsc: &str) {
        self.queue.retain(|p| p.lcsc != lcsc);
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    pub fn cat_path(&self, id: &str) -> String {
        let Some(c) = self.cat(id) else {
            return id.to_string();
        };
        if let Some(pid) = &c.parent {
            if let Some(p) = self.cat(pid) {
                return format!("{} / {}", p.name, c.name);
            }
        }
        c.name.clone()
    }

    #[allow(dead_code)]
    pub fn all_cat_ids(&self) -> Vec<String> {
        self.cats.iter().map(|c| c.id.clone()).collect()
    }
}

pub fn part_from_item(item: &SearchItem, cat_id: &str) -> Option<SavedPart> {
    let lcsc = item.lcsc_id()?;
    Some(SavedPart {
        lcsc,
        name: item.name().to_string(),
        manufacturer: item.manufacturer_label(),
        cat_id: cat_id.to_string(),
    })
}

fn parse_part(v: &Value) -> Option<SavedPart> {
    Some(SavedPart {
        lcsc: v.get("lcsc")?.as_str()?.to_string(),
        name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        manufacturer: v
            .get("manufacturer")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cat_id: v
            .get("cat_id")
            .and_then(Value::as_str)
            .unwrap_or(UNCAT)
            .to_string(),
    })
}

fn part_json(p: &SavedPart) -> Value {
    json!({
        "lcsc": p.lcsc,
        "name": p.name,
        "manufacturer": p.manufacturer,
        "cat_id": p.cat_id,
    })
}

fn normalize_binding(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err("请选择一个库文件".into());
    }
    if path.is_dir() { return Err("请选择库文件，不能绑定文件夹".into()); }
    let absolute = if path.is_absolute() { path.to_path_buf() } else {
        std::env::current_dir().map_err(|error| format!("无法确定库文件的绝对路径: {error}"))?.join(path)
    };
    // Resolve the deepest existing ancestor. Targets need not exist yet.
    let mut prefix = absolute.clone();
    let mut tail = Vec::new();
    let resolved = loop {
        match fs::canonicalize(&prefix) {
            Ok(mut existing) => {
                for component in tail.iter().rev() { existing.push(component); }
                break existing;
            }
            Err(_) => {
                let Some(name) = prefix.file_name().map(|s| s.to_os_string()) else { break absolute; };
                tail.push(name);
                if !prefix.pop() { break absolute; }
            }
        }
    };
    let mut normalized = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => { normalized.pop(); },
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.is_absolute() || normalized.file_name().is_none() { return Err("库文件路径无效".into()); }
    Ok(normalized)
}

pub(crate) fn binding_matches(bound: &Path, candidate: &Path) -> bool {
    normalize_binding(candidate).is_ok_and(|candidate| binding_identity(bound, cfg!(windows)) == binding_identity(&candidate, cfg!(windows)))
}

fn binding_identity(path: &Path, windows: bool) -> String {
    let value = path.to_string_lossy();
    if windows { value.replace('/', "\\").trim_start_matches("\\\\?\\").to_lowercase() }
    else { value.into_owned() }
}

fn new_id() -> String {
    // Timestamp-only IDs can collide when two categories are created in one frame.
    format!("c{}", lceda_core::util::unique_id())
}

fn library_path() -> Option<PathBuf> {
    crate::profile_dir().map(|d| d.join("library.json"))
}


pub fn package_from_item(item: &SearchItem) -> String {
    for key in ["Package", "package", "封装", "Supplier Footprint", "Footprint"] {
        if let Some(value) = item.raw.get("attributes").and_then(|a| a.get(key)).and_then(Value::as_str) {
            if !value.trim().is_empty() { return value.to_string(); }
        }
    }
    item.raw.get("package").and_then(Value::as_str).unwrap_or("").to_string()
}

fn timestamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_import_roundtrips_and_reimports_without_colliding_with_supplier_ids(){
        let part=NativePart{key:"C2040".into(),name:"C2040".into(),description:"local C2040".into(),properties:BTreeMap::new(),footprints:vec![],pins:2,units:1};
        let info=NativeLibrary{source:PathBuf::from("/tmp/UserLibrary.SchLib"),format:LibraryFormat::Ad,parts:vec![part],content_hash:1};
        let mut library=Library::default();library.add_fav(SavedPart{lcsc:"C2040".into(),name:"RP2040".into(),manufacturer:String::new(),cat_id:UNCAT.into()});
        let (cat,n)=library.add_native_parts(&info,&["C2040".into()],"UserLibrary",Path::new("/tmp/snapshot/UserLibrary.SchLib")).unwrap();assert_eq!(n,1);assert_eq!(library.favs.len(),2);
        let id=library.favs_in(&cat)[0].lcsc.clone();assert!(id.starts_with("LOCAL-"));assert!(library.cached_item(&id).is_none());
        let mut restored=Library::from_json(&library.to_json());assert_eq!(restored.native(&id).unwrap().part.name,"C2040");
        let (same,_)=restored.add_native_parts(&info,&["C2040".into()],"UserLibrary",Path::new("/tmp/new-copy/UserLibrary.SchLib")).unwrap();
        assert_eq!(same,cat);assert_eq!(restored.favs.len(),2);assert_eq!(restored.native(&id).unwrap().library,Path::new("/tmp/new-copy/UserLibrary.SchLib"));
        assert!(restored.add_native_parts(&info,&["Missing".into()],"Invalid",Path::new("/tmp/none")).is_err());assert_eq!(restored.cats.len(),2);
    }

    #[test]
    fn supplemental_resources_roundtrip_without_touching_shared_native_links(){
        use lceda_core::supplement::{ResourceBundle,ResourceKinds};
        let make=|key:&str|NativePart{key:key.into(),name:key.into(),description:String::new(),properties:BTreeMap::new(),footprints:vec!["SharedFootprint".into()],pins:2,units:1};
        let info=NativeLibrary{source:PathBuf::from("/tmp/Shared.SchLib"),format:LibraryFormat::Ad,parts:vec![make("A"),make("B")],content_hash:1};
        let mut library=Library::default();let(cat,_)=library.add_native_parts(&info,&["A".into(),"B".into()],"Shared",Path::new("/tmp/snapshot/Shared.SchLib")).unwrap();
        let ids:Vec<_>=library.favs_in(&cat).iter().map(|p|p.lcsc.clone()).collect();
        let bundle=ResourceBundle{directory:PathBuf::from("/tmp/resources/separate"),native_key:"A".into(),native_name:"A".into(),original_footprints:vec!["SharedFootprint".into()],supplier_id:"C25804".into(),supplier_name:"Resistor".into(),manufacturer:String::new(),format:LibraryFormat::Ad,kinds:ResourceKinds{footprint:true,model3d:false},files:BTreeMap::new(),hashes:BTreeMap::new()};
        assert!(library.add_supplement(&ids[1],bundle.clone()).is_err());library.add_supplement(&ids[0],bundle).unwrap();
        let mut restored=Library::from_json(&library.to_json());
        assert_eq!(restored.record(&ids[0]).unwrap().supplements.len(),1);assert!(restored.record(&ids[1]).unwrap().supplements.is_empty());
        for id in &ids{assert_eq!(restored.native(id).unwrap().part.footprints,["SharedFootprint"]);assert!(restored.cached_item(id).is_none());}
        restored.add_native_parts(&info,&["A".into(),"B".into()],"Shared",Path::new("/tmp/new/Shared.SchLib")).unwrap();
        assert_eq!(restored.record(&ids[0]).unwrap().supplements.len(),1);assert_eq!(restored.native(&ids[0]).unwrap().part.footprints,["SharedFootprint"]);
    }

    #[test]
    fn ad_and_kicad_bindings_are_independent_durable_and_not_inherited() {
        let mut lib = Library::default();
        let parent = lib.add_root("both").unwrap();
        let child = lib.add_child(&parent, "child").unwrap();
        let directory = std::env::temp_dir().join(format!("lceda-dual-binding-{}", lceda_core::util::unique_id()));
        let ad = directory.join("Parts.SchLib"); let ki = directory.join("Parts.kicad_sym");
        lib.set_binding(&parent, LibraryFormat::Ad, &ad).unwrap();
        lib.set_binding(&parent, LibraryFormat::KiCad, &ki).unwrap();
        assert!(lib.set_binding(&child, LibraryFormat::KiCad, &ki).is_err());
        assert!(lib.set_binding(&child, LibraryFormat::KiCad, &ad).is_err());
        let mut loaded = Library::from_json(&lib.to_json());
        for format in LibraryFormat::ALL { assert_eq!(loaded.binding_for(&parent, format), lib.binding_for(&parent, format)); assert!(loaded.binding_for(&child, format).is_none()); }
        loaded.clear_binding(&parent, LibraryFormat::Ad).unwrap();
        assert!(loaded.binding_for(&parent, LibraryFormat::Ad).is_none());
        assert!(loaded.binding_for(&parent, LibraryFormat::KiCad).is_some());
        assert_eq!(loaded.bound_categories().len(), 1);
    }

    #[test]
    fn category_bindings_roundtrip_and_old_categories_remain_unbound() {
        let mut lib = Library::from_json(&json!({"cats":[{"id":"old","name":"旧收藏"}], "favs":[{"lcsc":"C2040","cat_id":"old"}]}));
        assert!(lib.binding_for_category("old").is_none());
        let target = std::env::temp_dir().join(format!("lceda-binding-{}.SchLib", lceda_core::util::unique_id()));
        lib.bind_library("old", &target).unwrap();
        let loaded = Library::from_json(&lib.to_json());
        assert_eq!(loaded.cat("old").unwrap().name, "旧收藏");
        assert_eq!(loaded.binding_for_part("c2040"), loaded.binding_for_category("old"));
        assert_eq!(loaded.bound_categories().len(), 1);
        assert!(loaded.binding_for_category("old").unwrap().is_absolute());
        assert_eq!(loaded.binding_for_category("old").unwrap(), normalize_binding(&target).unwrap());
        assert!(loaded.binding_for_category(UNCAT).is_none());
    }

    #[test]
    fn category_binding_conflicts_are_rejected_and_removal_preserves_external_file() {
        let mut lib = Library::default();
        let first = lib.add_root("first").unwrap();
        let second = lib.add_root("second").unwrap();
        assert_ne!(first, second);
        let directory = std::env::temp_dir().join(format!("lceda-binding-{}", lceda_core::util::unique_id()));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("Library.SchLib");
        fs::write(&target, b"existing external library").unwrap();
        lib.bind_library(&first, &directory.join(".").join("Library.SchLib")).unwrap();
        assert!(lib.bind_library(&second, &target).is_err());
        assert!(lib.bind_library(&second, &directory).is_err());
        assert!(lib.bind_library("missing", &target).is_err());
        assert!(lib.bind_library(&second, Path::new("")).is_err());
        lib.unbind_library(&first).unwrap();
        lib.bind_library(&second, &target).unwrap();
        lib.remove_cat(&second);
        assert_eq!(fs::read(&target).unwrap(), b"existing external library");
        fs::remove_dir_all(directory).unwrap();
        assert_eq!(binding_identity(Path::new(r"C:\Libraries\User.SchLib"), true), binding_identity(Path::new("c:/libraries/user.schlib"), true));
    }

    #[test]
    fn old_favorites_survive_and_records_roundtrip() {
        let mut lib = Library::from_json(&json!({"favs": [{"lcsc":"C2040", "name":"part"}]}));
        assert_eq!(lib.favs[0].cat_id, UNCAT);
        let item = SearchItem {
            index: 1, display_title: "part".into(), title: "part".into(),
            manufacturer: "maker".into(), model_uuid: None,
            raw: json!({"product_code":"C2040", "attributes":{"Package":"SOT-23", "Voltage":"3.3V"}}),
        };
        lib.enrich_from_item(&item);
        lib.set_notes("C2040", "validated on board A".into());
        let loaded = Library::from_json(&lib.to_json());
        assert_eq!(loaded.cached_item("C2040").unwrap().name(), "part");
        let record = loaded.record("C2040").unwrap();
        assert_eq!(record.notes, "validated on board A");
        assert_eq!(record.parameters["Voltage"], "3.3V");
        assert_eq!(record.package, "SOT-23");
    }

    #[test]
    fn resource_status_checks_files_instead_of_persisted_flags() {
        let dir = std::env::temp_dir().join(format!("lceda-library-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let symbol = dir.join("part.SchLib");
        fs::write(&symbol, b"test").unwrap();
        let mut lib = Library::default();
        lib.record_export("C2040", &DownloadPaths { schlib: Some(symbol.clone()), ..Default::default() });
        assert!(lib.record("C2040").unwrap().has_symbol());
        fs::remove_file(&symbol).unwrap();
        assert!(!lib.record("C2040").unwrap().has_symbol());
        fs::remove_dir(&dir).unwrap();
    }
}
