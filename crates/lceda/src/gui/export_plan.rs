use super::*;

#[derive(Clone)]
pub(super) enum ExportSelection { Items(Vec<SearchItem>), Ids(Vec<String>) }

/// The confirmation dialog owns its selection and destination. Neither follows
/// the shared preview, a later search result, or a later category selection.
#[derive(Clone)]
pub(super) struct ExportPlan {
    pub selection: ExportSelection,
    pub caption: String,
    pub output: PathBuf,
    pub library_name: Option<String>,
    isolated: bool,
}

impl ExportPlan {
    pub fn ids(ids: Vec<String>, caption: String, output: PathBuf) -> Self {
        Self { selection: ExportSelection::Ids(ids), caption, output, library_name: None, isolated: false }
    }
    pub fn item(item: SearchItem, caption: String, output: PathBuf) -> Self {
        Self { selection: ExportSelection::Items(vec![item]), caption, output, library_name: None, isolated: false }
    }
    pub fn library(library: &Library, category: &str, name: &str, caption: String, output: PathBuf) -> Self {
        let ids = library.favs_in(category).into_iter().map(|part| part.lcsc.clone()).collect();
        let mut plan = Self::ids(ids, caption, output);
        plan.output.push(format!("Library-{}-{}", lceda_core::util::sanitize_filename(name), lceda_core::util::unique_id()));
        plan.library_name = Some(name.into()); plan.isolated = true; plan
    }
    pub fn len(&self) -> usize {
        match &self.selection { ExportSelection::Items(items) => items.len(), ExportSelection::Ids(ids) => ids.len() }
    }
    pub fn saved_ids(&self) -> &[String] {
        match &self.selection { ExportSelection::Ids(ids) => ids, ExportSelection::Items(_) => &[] }
    }
    pub fn prepare_native(&mut self) {
        if !self.isolated { self.output.push(format!("Library-{}", lceda_core::util::unique_id())); self.isolated = true; }
    }
    pub fn rebase_native(&mut self, base: &Path) {
        self.output = base.join(self.output.file_name().unwrap());
    }
    pub fn review(&self, ui: &mut egui::Ui, library: &Library, lang: Lang) {
        let title = format!("{} · {} {}", self.caption, self.len(), i18n::t(lang, "components"));
        ui.add(egui::Label::new(theme::heading(&title)).truncate()).on_hover_text(title);
        let labels: Vec<_> = match &self.selection {
            ExportSelection::Items(items) => items.iter().take(16).map(|item| format!("{} · {}", item.lcsc_id().unwrap_or_default(), item.name())).collect(),
            ExportSelection::Ids(ids) => ids.iter().take(16).map(|id| library.native(id).map(|r| r.part.name.clone()).unwrap_or_else(|| id.clone())).collect(),
        };
        let mut summary = labels.join("、"); if self.len() > 16 { summary.push_str(" …"); }
        ui.add(egui::Label::new(egui::RichText::new(&summary).small().color(theme::secondary())).truncate()).on_hover_text(summary);
    }
}

pub(super) fn saved_selection<'a>(parts: impl IntoIterator<Item = &'a SavedPart>, current: Option<&str>) -> Option<String> {
    let mut first = None;
    for part in parts {
        if first.is_none() { first = Some(part.lcsc.clone()); }
        if current == Some(part.lcsc.as_str()) { return Some(part.lcsc.clone()); }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;
    fn part(id: &str, cat: &str) -> SavedPart {
        SavedPart { lcsc: id.into(), name: id.into(), manufacturer: String::new(), cat_id: cat.into() }
    }
    fn item(id: &str) -> SearchItem {
        SearchItem { index: 1, display_title: id.into(), title: id.into(), manufacturer: String::new(), model_uuid: None, raw: serde_json::json!({"product_code":id}) }
    }
    #[test]
    fn library_export_excludes_searched_queued_and_other_category_parts() {
        let mut lib = Library::default();
        lib.add_fav(part("C2040", UNCAT)); lib.add_fav(part("C25804", UNCAT)); lib.add_fav(part("C3013946", "other"));
        lib.add_queue(part("C142297", UNCAT)); lib.enrich_from_item(&item("C8755"));
        let plan = ExportPlan::library(&lib, UNCAT, "电阻/MCU", "整库".into(), PathBuf::from("out"));
        assert_eq!(plan.saved_ids(), &["C2040", "C25804"]);
        lib.add_fav(part("C8755", UNCAT));
        assert_eq!(plan.saved_ids(), &["C2040", "C25804"], "opening a dialog freezes its selection");
        assert_eq!(plan.output.parent(), Some(Path::new("out")));
        let again = ExportPlan::library(&lib, UNCAT, "电阻/MCU", "整库".into(), PathBuf::from("out"));
        assert_ne!(plan.output, again.output, "each library export has its own directory");
    }
    #[test]
    fn single_export_owns_the_selected_snapshot() {
        let mut selected = item("C3013946");
        let plan = ExportPlan::item(selected.clone(), "单颗".into(), PathBuf::from("out"));
        selected.raw = serde_json::json!({"product_code":"C2040"});
        let ExportSelection::Items(items) = plan.selection else { panic!("single export needs exact snapshots") };
        assert_eq!(items[0].lcsc_id().as_deref(), Some("C3013946"));
        assert_eq!(selected.lcsc_id().as_deref(), Some("C2040"));
    }
    #[test]
    fn context_selection_preserves_only_members_of_the_current_scope() {
        let parts = [part("C2040", UNCAT), part("LOCAL-original", UNCAT)];
        assert_eq!(saved_selection(&parts, Some("C3013946")).as_deref(), Some("C2040"));
        assert_eq!(saved_selection(&parts, Some("LOCAL-original")).as_deref(), Some("LOCAL-original"));
        assert_eq!(saved_selection(std::iter::empty(), Some("C2040")), None);
        let queue = [part("C25804", UNCAT)];
        assert_eq!(saved_selection(&queue, Some("C2040")).as_deref(), Some("C25804"));
    }
}
