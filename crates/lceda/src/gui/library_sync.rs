use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowState { Waiting, Resolving, Syncing, Done, Failed, Skipped }
impl RowState {
    fn key(self) -> &'static str {
        match self {
            Self::Waiting => "sync_waiting", Self::Resolving => "sync_resolving",
            Self::Syncing => "sync_writing", Self::Done => "done",
            Self::Failed => "sync_failed", Self::Skipped => "sync_skipped",
        }
    }
}

#[derive(Clone)]
struct SyncRow {
    id: String, name: String, targets: String, state: RowState, error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage { Validating, Running, Finished }
#[derive(Clone)]
struct Progress {
    rows: Vec<SyncRow>, stage: Stage, started: Instant, finished: Option<Instant>, note: Option<String>,
}
impl Progress {
    fn counts(&self) -> (usize, usize, usize) {
        let done = self.rows.iter().filter(|r| r.state == RowState::Done).count();
        let failed = self.rows.iter().filter(|r| r.state == RowState::Failed).count();
        let skipped = self.rows.iter().filter(|r| r.state == RowState::Skipped).count();
        (done, failed, skipped)
    }
    fn finish(&mut self) {
        for row in &mut self.rows {
            if row.state == RowState::Waiting { row.state = RowState::Skipped; }
        }
        self.stage = Stage::Finished;
        self.finished.get_or_insert_with(Instant::now);
    }
}

pub(super) struct LibrarySyncTask {
    progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>,
    targets: BTreeSet<(String, LibraryFormat)>, can_retry: bool,
}
type BindResult = lceda_core::Result<(String, LibraryFormat, PathBuf)>;
pub(super) type BindPromise = Promise<BindResult>;

impl LibrarySyncTask {
    fn new(rows: Vec<SyncRow>, targets: BTreeSet<(String, LibraryFormat)>, stage: Stage) -> Self {
        Self { progress: Arc::new(Mutex::new(Progress { rows, stage, started: Instant::now(), finished: None, note: None })),
            cancel: Arc::new(AtomicBool::new(false)), targets, can_retry: stage == Stage::Running }
    }
    pub(super) fn running(&self) -> bool { self.progress.lock().unwrap().stage != Stage::Finished }
    pub(super) fn status(&self) -> (usize, usize) {
        let p = self.progress.lock().unwrap(); let (done, failed, _) = p.counts(); (done + failed, p.rows.len())
    }
    pub(super) fn fail(&self, error: String) {
        let mut p = self.progress.lock().unwrap(); p.note = Some(error); p.finish();
    }
}

// Only this worker writes bound files. Cancellation is checked between parts,
// allowing the current file transaction to finish before stopping.
fn run_rows(
    task: &LibrarySyncTask, output: &Path,
    mut resolve: impl FnMut(&str) -> Result<SearchItem, String>,
    mut write: impl FnMut(&SearchItem) -> Result<DownloadPaths, String>,
) -> JobReport {
    let count = task.progress.lock().unwrap().rows.len();
    let mut report = JobReport { message: String::new(), exported: Vec::new(), notify: false, failed: 0 };
    for index in 0..count {
        if task.cancel.load(Ordering::SeqCst) { break; }
        let id = {
            let mut p = task.progress.lock().unwrap(); p.rows[index].state = RowState::Resolving;
            p.rows[index].id.clone()
        };
        let result = resolve(&id).and_then(|item| {
            // A cancellation during a lookup must not start a new write.
            if task.cancel.load(Ordering::SeqCst) { return Ok(None); }
            {
                let mut p = task.progress.lock().unwrap();
                p.rows[index].name = item.name().to_string(); p.rows[index].state = RowState::Syncing;
            }
            write(&item).map(|paths| Some((item, paths)))
        });
        let mut p = task.progress.lock().unwrap();
        match result {
            Ok(Some((item, paths))) => {
                p.rows[index].state = RowState::Done;
                report.message.push_str(&format!("OK {} · {}\n{}\n", id, item.name(), format_paths(&paths, output)));
                report.exported.push((item, paths));
            }
            Ok(None) => { p.rows[index].state = RowState::Skipped; }
            Err(error) => {
                p.rows[index].state = RowState::Failed; p.rows[index].error = Some(error.clone());
                report.failed += 1; report.message.push_str(&format!("FAIL {id}: {error}\n"));
            }
        }
    }
    let mut p = task.progress.lock().unwrap(); p.finish();
    let (done, failed, skipped) = p.counts();
    report.message.push_str(&format!("done={done}, failed={failed}, skipped={skipped}"));
    report
}

impl App {
    pub(super) fn begin_binding_validation(&mut self, category: String, format: LibraryFormat, path: PathBuf) {
        let targets = BTreeSet::from([(category.clone(), format)]);
        let rows = self.lib.favs_in(&category).into_iter().filter(|part| self.lib.native(&part.lcsc).is_none())
            .map(|part| SyncRow { id: part.lcsc.to_ascii_uppercase(), name: part.name.clone(),
                targets: format!("{} · {}", format.name(), path.display()), state: RowState::Waiting, error: None }).collect();
        let task = LibrarySyncTask::new(rows, targets, Stage::Validating);
        let cancel = task.cancel.clone();
        self.library_sync_task = Some(task); self.show_library_sync = true; self.bind_cat = None;
        self.bind_job = Some(Promise::spawn_thread("validate-library-binding", move || {
            match format {
                LibraryFormat::Ad => lceda_core::bound_ad::validate_binding(&path)?,
                LibraryFormat::KiCad => lceda_core::bound_kicad::validate_binding(&path)?,
            }
            if cancel.load(Ordering::SeqCst) { return Err(lceda_core::Error::msg("绑定已取消，原绑定保持不变")); }
            Ok((category, format, path))
        }));
    }

    pub(super) fn poll_binding_validation(&mut self) {
        if !self.bind_job.as_ref().is_some_and(|job| job.ready().is_some()) { return; }
        let result = self.bind_job.take().unwrap().block_and_take();
        if self.library_sync_task.as_ref().is_some_and(|task| task.cancel.load(Ordering::SeqCst)) {
            if let Some(task) = &self.library_sync_task { task.progress.lock().unwrap().finish(); }
            return;
        }
        let result = result.and_then(|(category, format, path)| {
            let mut next = self.lib.clone();
            next.set_binding(&category, format, &path).map_err(lceda_core::Error::msg)?;
            next.try_save()?; self.lib = next;
            self.queue_library_sync(&category, Some(format)); self.library_sync_after = None;
            Ok(())
        });
        if let Err(error) = result {
            self.log(error.to_string());
            if let Some(task) = &self.library_sync_task { task.fail(error.to_string()); }
        }
    }

    pub(super) fn launch_library_sync(&mut self, targets: BTreeSet<(String, LibraryFormat)>, req: ExportRequest, ids: BTreeSet<String>) {
        let rows = ids.into_iter().map(|id| {
            let name = self.lib.favs.iter().find(|p| p.lcsc.eq_ignore_ascii_case(&id)).map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
            let mut paths = Vec::new();
            if let Some(path) = req.ad_library_files.get(&id) { paths.push(format!("AD · {}", path.display())); }
            if let Some(path) = req.kicad_library_files.get(&id) { paths.push(format!("KiCad · {}", path.display())); }
            SyncRow { id, name, targets: paths.join("\n"), state: RowState::Waiting, error: None }
        }).collect();
        let task = LibrarySyncTask::new(rows, targets, Stage::Running);
        if task.progress.lock().unwrap().rows.is_empty() {
            task.progress.lock().unwrap().finish(); self.library_sync_task = Some(task); return;
        }
        if self.out_dir.trim().is_empty() {
            task.fail(i18n::t(self.lang, "choose_output").into()); self.library_sync_task = Some(task); self.show_library_sync = true; return;
        }
        let worker = LibrarySyncTask { progress: task.progress.clone(), cancel: task.cancel.clone(), targets: task.targets.clone(), can_retry: true };
        self.library_sync_task = Some(task); self.library_sync_job = true;
        let library = self.lib.clone(); let heading = i18n::t(self.lang, "sync_library").to_string(); self.log(&heading);
        self.job = Some(Promise::spawn_thread("sync-bound-library", move || {
            let client = LcedaClient::new();
            let mut report = run_rows(&worker, &req.out_dir, |id| {
                if let Some(item) = library.cached_item(id) { return Ok(item); }
                client.search(id).map_err(|error| error.to_string())?.into_iter()
                    .find(|item| item.lcsc_id().as_deref() == Some(id))
                    .ok_or_else(|| format!("{id}: 未找到相同编号，请先搜索并确认器件"))
            }, |item| {
                lceda_core::export::export_items(&client, std::slice::from_ref(item), &req).into_iter().next()
                    .ok_or_else(|| "同步未返回器件结果".to_string())?.1.map_err(|error| error.to_string())
            });
            report.message = format!("{heading}\n{}", report.message); Ok(report)
        }));
    }

    pub(super) fn library_sync_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_library_sync { return; }
        let Some(task) = &self.library_sync_task else { return; };
        let p = task.progress.lock().unwrap().clone(); let lang = self.lang;
        let running = p.stage != Stage::Finished; let stopping = task.cancel.load(Ordering::SeqCst);
        let (done, failed, skipped) = p.counts(); let total = p.rows.len(); let completed = done + failed;
        let mut open = true; let mut close = false; let mut cancel = false; let mut retry = false;
        fit_window(i18n::t(lang, "sync_progress"), "library_sync_progress", 660.0).open(&mut open).show(ctx, |ui| {
            ui.set_width(636.0);
            ui.horizontal_wrapped(|ui| {
                if running { ui.add(egui::Spinner::new().size(16.0)); }
                let key = if stopping && running { "sync_stopping" } else if p.stage == Stage::Validating { "sync_validating" }
                    else if running { "sync_running" } else if stopping { "sync_stopped" }
                    else if failed > 0 || p.note.is_some() { "sync_finished_errors" } else { "sync_finished" };
                ui.label(theme::heading(i18n::t(lang, key)));
                ui.label(egui::RichText::new(format!("{}s", p.finished.unwrap_or_else(Instant::now).duration_since(p.started).as_secs())).small().color(theme::secondary()));
            });
            let summary = format!("{completed} / {total} · {} {done} · {} {failed} · {} {skipped}",
                i18n::t(lang, "sync_success"), i18n::t(lang, "sync_failed"), i18n::t(lang, "sync_skipped"));
            let color = if failed > 0 && !running { ui.visuals().warn_fg_color } else { ACCENT };
            ui.label(summary);
            ui.add(egui::ProgressBar::new(if total > 0 { completed as f32 / total as f32 } else { 0.0 }).desired_height(8.0).fill(color));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("library_sync_rows").max_height((ctx.screen_rect().height() - 320.0).clamp(150.0, 340.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(note) = &p.note { ui.add(egui::Label::new(egui::RichText::new(note).color(ui.visuals().error_fg_color)).wrap().selectable(true)); }
                if total == 0 && p.stage == Stage::Finished && p.note.is_none() { ui.add(egui::Label::new(i18n::t(lang, "sync_empty")).wrap()); }
                for row in &p.rows {
                    let active = matches!(row.state, RowState::Resolving | RowState::Syncing);
                    egui::Frame::new().fill(if active { theme::well() } else { theme::fill_strong() }).corner_radius(7)
                        .inner_margin(8).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            let color = if row.state == RowState::Failed { ui.visuals().error_fg_color } else if row.state == RowState::Done { ui.visuals().hyperlink_color } else { theme::secondary() };
                            ui.label(egui::RichText::new(i18n::t(lang, row.state.key())).small().color(color));
                            ui.add(egui::Label::new(theme::heading(format!("{} · {}", row.id, row.name))).truncate());
                        });
                        ui.add(egui::Label::new(egui::RichText::new(row.targets.replace('\n', " · ")).small().color(theme::secondary())).truncate()).on_hover_text(&row.targets);
                        if let Some(error) = &row.error { ui.add(egui::Label::new(error).wrap().selectable(true)); }
                    });
                    ui.add_space(4.0);
                }
            });
            ui.add_space(8.0);
            if running { ui.add(egui::Label::new(egui::RichText::new(i18n::t(lang, "sync_cancel_hint")).small().color(theme::secondary())).wrap()); }
            ui.horizontal_wrapped(|ui| {
                close = theme::pill_button(ui, i18n::t(lang, if running { "sync_background" } else { "close" }), true, !running).clicked();
                if running { cancel = theme::pill_button(ui, i18n::t(lang, if stopping { "sync_stopping" } else { "sync_cancel" }), !stopping, false).clicked(); }
                else if task.can_retry && (failed > 0 || skipped > 0 || p.note.is_some()) {
                    retry = theme::pill_button(ui, i18n::t(lang, "sync_retry"), !self.busy(), false).clicked();
                }
            });
        });
        if cancel { task.cancel.store(true, Ordering::SeqCst); }
        if retry {
            let targets = task.targets.clone();
            for (category, format) in targets { self.queue_library_sync(&category, Some(format)); }
            self.library_sync_after = None;
        }
        if close || !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.show_library_sync = false; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> LibrarySyncTask {
        LibrarySyncTask::new((1001..=1003).map(|n| SyncRow { id: format!("C{n}"), name: String::new(), targets: "AD · test.SchLib".into(), state: RowState::Waiting, error: None }).collect(), BTreeSet::new(), Stage::Running)
    }
    fn item(id: &str) -> SearchItem {
        SearchItem { index: 1, display_title: id.into(), title: id.into(), manufacturer: String::new(),
            model_uuid: None, raw: serde_json::json!({"product_code": id}) }
    }
    #[test]
    fn reports_each_failure_without_stopping_other_parts() {
        let task = task();
        let report = run_rows(&task, Path::new("."), |id| {
            let p = task.progress.lock().unwrap(); assert!(p.rows.iter().any(|r| r.id == id && r.state == RowState::Resolving)); Ok(item(id))
        }, |item| {
            let id = item.lcsc_id().unwrap(); let p = task.progress.lock().unwrap();
            assert!(p.rows.iter().any(|r| r.id == id && r.state == RowState::Syncing));
            if id == "C1002" { Err("library is locked".into()) } else { Ok(DownloadPaths::default()) }
        });
        let p = task.progress.lock().unwrap(); assert_eq!(p.counts(), (2, 1, 0)); assert!(p.stage == Stage::Finished);
        assert_eq!(p.rows[1].error.as_deref(), Some("library is locked")); assert_eq!(report.exported.len(), 2); assert_eq!(report.failed, 1);
    }
    #[test]
    fn cancel_finishes_current_write_and_skips_remaining_parts() {
        let task = task(); let mut writes = 0;
        let report = run_rows(&task, Path::new("."), |id| Ok(item(id)), |_| {
            writes += 1; task.cancel.store(true, Ordering::SeqCst); Ok(DownloadPaths::default())
        });
        assert_eq!(writes, 1); assert_eq!(report.exported.len(), 1); assert_eq!(report.failed, 0);
        assert_eq!(task.progress.lock().unwrap().counts(), (1, 0, 2));
    }
    #[test]
    fn cancel_during_lookup_starts_no_write() {
        let task = task();
        let report = run_rows(&task, Path::new("."), |id| { task.cancel.store(true, Ordering::SeqCst); Ok(item(id)) }, |_| panic!("cancelled lookup must not start a write"));
        assert!(report.exported.is_empty()); assert_eq!(task.progress.lock().unwrap().counts(), (0, 0, 3));
    }
}
