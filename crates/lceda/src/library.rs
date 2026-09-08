use lceda_core::models::SearchItem;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub const UNCAT: &str = "uncat";

#[derive(Debug, Clone)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SavedPart {
    pub lcsc: String,
    pub name: String,
    pub manufacturer: String,
    pub cat_id: String,
}

#[derive(Debug, Clone)]
pub struct Library {
    pub cats: Vec<Category>,
    pub favs: Vec<SavedPart>,
    pub queue: Vec<SavedPart>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            cats: vec![Category {
                id: UNCAT.into(),
                name: "默认收藏夹".into(),
                parent: None,
            }],
            favs: Vec::new(),
            queue: Vec::new(),
        }
    }
}

impl Library {
    pub fn load() -> Self {
        let Some(path) = library_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return Self::default();
        };
        Self::from_json(&v)
    }

    pub fn save(&self) {
        let Some(path) = library_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, self.to_json().to_string());
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
                    },
                );
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
        lib
    }

    fn to_json(&self) -> Value {
        json!({
            "cats": self.cats.iter().map(|c| json!({
                "id": c.id,
                "name": c.name,
                "parent": c.parent,
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

fn new_id() -> String {
    format!(
        "c{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(1)
    )
}

fn library_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant")
        .map(|d| d.config_dir().join("library.json"))
}
