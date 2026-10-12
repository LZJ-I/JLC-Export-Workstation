use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
struct QueuedScope { category: String, format: LibraryFormat, binding: PathBuf }

fn queue_scopes(library: &Library, targets: &BTreeSet<(String, LibraryFormat)>) -> Vec<QueuedScope> {
    targets.iter().filter_map(|(category, format)| library.binding_for(category, *format)
        .map(|path| QueuedScope { category: category.clone(), format: *format, binding: path.to_path_buf() })).collect()
}
fn restore_scopes(library: &Library, scopes: Vec<QueuedScope>) -> BTreeSet<(String, LibraryFormat)> {
    scopes.into_iter().filter(|scope| library.binding_for(&scope.category, scope.format).is_some_and(|bound| crate::library::binding_matches(bound, &scope.binding)))
        .map(|scope| (scope.category, scope.format)).collect()
}
fn write_queue(path: &Path, scopes: &[QueuedScope]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| std::io::Error::other("队列路径缺少目录"))?;
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".ad-queue-{}.tmp", lceda_core::util::unique_id()));
    let result = (|| {
        let bytes = serde_json::to_vec(&serde_json::json!({ "version": 1, "scopes": scopes }))?;
        let mut file = std::fs::File::create(&temp)?; file.write_all(&bytes)?; file.sync_all()?; drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" { fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32; }
            let from: Vec<_> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 { return Err(std::io::Error::last_os_error()); }
        }
        #[cfg(not(windows))]
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(temp); }
    result
}

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
    detail: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage { Validating, AwaitingAd, Running, Finished }
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
                p.rows[index].detail = paths.ad_sync.clone();
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
    pub(super) fn restore_sync_queue(&mut self) {
        let Some(root) = crate::profile_dir() else { return; };
        let path = root.join("pending-library-sync.json");
        if !path.exists() { return; }
        let scopes = (|| -> Result<Vec<QueuedScope>, String> {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.len() > 512 * 1024 { return Err("同步队列过大，未自动恢复".into()); }
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if value["version"].as_u64() != Some(1) { return Err("同步队列版本不支持，未自动恢复".into()); }
            serde_json::from_value(value["scopes"].clone()).map_err(|e| e.to_string())
        })();
        match scopes {
            Ok(scopes) => { self.pending_library_sync = restore_scopes(&self.lib, scopes); }
            Err(error) => self.log(error),
        }
    }
    pub(super) fn persist_sync_queue(&mut self) -> bool {
        let Some(root) = crate::profile_dir() else { return false; };
        let scopes = queue_scopes(&self.lib, &self.pending_library_sync);
        match write_queue(&root.join("pending-library-sync.json"), &scopes) {
            Ok(()) => true,
            Err(error) => { self.log(format!("同步队列保存失败，未开始新任务：{error}")); false }
        }
    }
    pub(super) fn native_ad_waiting(&self) -> bool {
        cfg!(windows) && crate::profile_dir().map(|p| p.join("ad-bridge")).is_some_and(|root|
            lceda_core::ad_plugin::installed(&root) && lceda_core::ad_bridge::enabled(&root) && lceda_core::ad_bridge::connection(&root).is_err())
    }
    pub(super) fn poll_ad_plugin(&mut self) {
        if self.ad_plugin_job.as_ref().is_some_and(|job| job.ready().is_some()) {
            let result = self.ad_plugin_job.take().unwrap().block_and_take();
            let note = match result { Ok(note) => note, Err(error) => error.to_string() };
            self.log(&note); self.ad_plugin_note = Some(note);
        }
    }
    fn start_ad_plugin_uninstall(&mut self, root: PathBuf) {
        if self.busy() { return; }
        self.ad_plugin_note = None;
        self.ad_plugin_job = Some(Promise::spawn_thread("uninstall-ad-native-plugin", move || lceda_core::ad_plugin::uninstall(&root)));
    }
    pub(super) fn settings_ad_plugin(&mut self, ui: &mut egui::Ui) {
        let Some(root) = crate::profile_dir().map(|p| p.join("ad-bridge")) else { return; };
        let lang = self.lang;
        let installed = lceda_core::ad_plugin::installed(&root);
        let connected = lceda_core::ad_bridge::connection(&root).is_ok();
        let working = self.ad_plugin_job.is_some();
        let mut uninstall = false;
        theme::show_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::heading(i18n::t(lang, "settings_ad_plugin")));
                if working { ui.add(egui::Spinner::new().size(14.0)); }
                ui.label(egui::RichText::new(i18n::t(lang, if working { "working" } else if installed { "ad_plugin_present" } else { "ad_plugin_not_installed" })).small().color(theme::secondary()));
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(i18n::t(lang, "ad_plugin_uninstall_hint")).wrap());
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                uninstall = theme::pill_button(ui, i18n::t(lang, "ad_plugin_uninstall_ad"), installed && !connected && !self.busy(), false)
                    .on_disabled_hover_text(i18n::t(lang, if !installed { "ad_plugin_not_installed" } else if connected { "ad_plugin_close_ad" } else { "working" })).clicked();
                if theme::pill_button(ui, i18n::t(lang, "ad_plugin_manage"), true, false).clicked() {
                    self.show_ad_connection = true;
                }
            });
            if let Some(note) = &self.ad_plugin_note {
                ui.add_space(6.0);
                ui.add(egui::Label::new(note).wrap().selectable(true));
            }
        });
        if uninstall { self.start_ad_plugin_uninstall(root); }
    }
    pub(super) fn show_queued_ad(&mut self) {
        let targets: BTreeSet<_> = self.pending_library_sync.iter().filter(|(_, format)| *format == LibraryFormat::Ad).cloned().collect();
        if targets.is_empty() || self.library_sync_task.as_ref().is_some_and(|task|
            task.targets == targets && task.progress.lock().unwrap().stage == Stage::AwaitingAd) { return; }
        let mut rows = Vec::new();
        for (category, format) in &targets {
            if let Some(path) = self.lib.binding_for(category, *format) {
                for part in self.lib.favs_in(category) {
                    if self.lib.native(&part.lcsc).is_none() {
                        rows.push(SyncRow { id: part.lcsc.to_ascii_uppercase(), name: part.name.clone(),
                            targets: format!("AD · {}", path.display()), state: RowState::Waiting, error: None, detail: None });
                    }
                }
            }
        }
        self.library_sync_task = Some(LibrarySyncTask::new(rows, targets, Stage::AwaitingAd));
    }
    pub(super) fn ad_connection_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_ad_connection { return; }
        let Some(root) = crate::profile_dir().map(|p| p.join("ad-bridge")) else { return; };
        let lang = self.lang;
        let connection = lceda_core::ad_bridge::connection(&root).ok();
        let connected = connection.is_some();
        let native = connection.is_some_and(|connection| connection.native);
        let installed = lceda_core::ad_plugin::installed(&root);
        let enabled = lceda_core::ad_bridge::enabled(&root);
        let installing = self.ad_plugin_job.is_some();
        let mut open = true; let mut launch = false; let mut folder = false; let mut offline = false; let mut close = false;
        let mut install = false; let mut enable = false; let mut uninstall = false;
        fit_window(i18n::t(lang, "ad_connection_title"), "ad_connection", 600.0).open(&mut open).show(ctx, |ui| {
            ui.set_width(576.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::heading(i18n::t(lang, if native { "ad_native_connected" } else if connected { "ad_connected" } else { "ad_disconnected" })));
                if installing || enabled && !connected { ui.add(egui::Spinner::new().size(14.0)); }
            });
            ui.add(egui::Label::new(i18n::t(lang, "ad_connection_hint")).wrap());
            ui.add_space(10.0);
            egui::ScrollArea::vertical().min_scrolled_height(220.0).max_height((ctx.screen_rect().height() - 290.0).clamp(220.0, 400.0)).auto_shrink([false, true]).show(ui, |ui| {
            egui::Frame::new().fill(theme::well()).corner_radius(7).inner_margin(10).show(ui, |ui| {
                ui.label(theme::heading(i18n::t(lang, "ad_plugin_title")));
                ui.add(egui::Label::new(i18n::t(lang, "ad_plugin_hint")).wrap());
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    install = theme::pill_button(ui, i18n::t(lang, if installing { "working" } else if installed { "ad_plugin_update" } else { "ad_plugin_install" }), cfg!(windows) && !connected && !installing && !self.busy(), true).clicked();
                    if installed && !enabled { enable = theme::pill_button(ui, i18n::t(lang, "ad_plugin_enable"), !installing && !self.busy(), false).clicked(); }
                    if installed { ui.label(egui::RichText::new(i18n::t(lang, "ad_plugin_installed")).small().color(theme::secondary())); }
                });
            });
            if let Some(note) = &self.ad_plugin_note {
                ui.add_space(6.0); ui.add(egui::Label::new(note).wrap().selectable(true));
            }
            if root.join("native-error.txt").is_file() {
                egui::CollapsingHeader::new(i18n::t(lang, "ad_plugin_diagnostics")).show(ui, |ui| {
                    if let Ok(text) = std::fs::read_to_string(root.join("native-error.txt")) { ui.add(egui::Label::new(text.chars().take(4000).collect::<String>()).wrap().selectable(true)); }
                });
            }
            egui::CollapsingHeader::new(i18n::t(lang, "ad_plugin_paths")).show(ui, |ui| {
                ui.add(egui::Label::new(i18n::t(lang, "ad_plugin_paths_hint")).wrap());
                for (key, value) in [("ad_plugin_ad_dir", &mut self.ad_install_dir), ("ad_plugin_ext_dir", &mut self.ad_extensions_dir)] {
                    ui.label(i18n::t(lang, key));
                    ui.horizontal(|ui| {
                        ui.add_sized([ui.available_width() - 64.0, 28.0], egui::TextEdit::singleline(value));
                        if ui.add_enabled(!installing, egui::Button::new(i18n::t(lang, "browse"))).clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() { *value = path.display().to_string(); }
                        }
                    });
                }
            });
            egui::CollapsingHeader::new(i18n::t(lang, "ad_script_compat")).show(ui, |ui| {
                for key in ["ad_connection_step1", "ad_connection_step2", "ad_connection_step3"] { ui.add(egui::Label::new(i18n::t(lang, key)).wrap()); }
                launch = theme::pill_button(ui, i18n::t(lang, "ad_open_project"), cfg!(windows) && !installed && !connected && !installing && !self.busy(), false).clicked();
                if installed { ui.add(egui::Label::new(i18n::t(lang, "ad_script_uninstall_hint")).wrap()); }
            });
            ui.add_space(8.0);
            ui.add(egui::Label::new(egui::RichText::new(i18n::t(lang, "ad_preserve_hint")).small().color(theme::secondary())).wrap());
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(root.display().to_string()).small().color(theme::secondary())).truncate()).on_hover_text(root.display().to_string());
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                folder = theme::pill_button(ui, i18n::t(lang, "ad_open_folder"), root.is_dir(), false).clicked();
                if installed { uninstall = theme::pill_button(ui, i18n::t(lang, "ad_plugin_uninstall"), !connected && !self.busy(), false).clicked(); }
                close = theme::pill_button(ui, i18n::t(lang, "close"), true, false).clicked();
            });
            if enabled {
                ui.add_space(6.0);
                offline = theme::pill_button(ui, i18n::t(lang, "ad_file_mode"), !self.busy(), false).clicked();
            }
        });
        if install {
            let root = root.clone();
            let ad = (!self.ad_install_dir.trim().is_empty()).then(|| PathBuf::from(self.ad_install_dir.trim()));
            let extensions = (!self.ad_extensions_dir.trim().is_empty()).then(|| PathBuf::from(self.ad_extensions_dir.trim()));
            self.ad_plugin_note = None;
            self.ad_plugin_job = Some(Promise::spawn_thread("install-ad-native-plugin", move || lceda_core::ad_plugin::install(&root, ad.as_deref(), extensions.as_deref())));
        }
        if enable { if let Err(error) = lceda_core::ad_bridge::enable(&root) { self.alert(error.to_string()); } }
        if uninstall {
            self.start_ad_plugin_uninstall(root.clone());
        }
        if launch {
            match lceda_core::ad_bridge::install(&root) {
                Ok(project) => { if !open_folder(&project) { self.alert(i18n::t(lang, "ad_open_failed")); } }
                Err(error) => self.alert(error.to_string()),
            }
        }
        if folder { open_folder(&root); }
        if offline {
            if let Err(error) = lceda_core::ad_bridge::disable(&root) { self.alert(error.to_string()); }
        }
        if close || !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.show_ad_connection = false; }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    pub(super) fn begin_binding_validation(&mut self, category: String, format: LibraryFormat, path: PathBuf) {
        let targets = BTreeSet::from([(category.clone(), format)]);
        let rows = self.lib.favs_in(&category).into_iter().filter(|part| self.lib.native(&part.lcsc).is_none())
            .map(|part| SyncRow { id: part.lcsc.to_ascii_uppercase(), name: part.name.clone(),
                targets: format!("{} · {}", format.name(), path.display()), state: RowState::Waiting, error: None, detail: None }).collect();
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

    #[allow(unused_mut)] // AD editor connection is Windows-only.
    pub(super) fn launch_library_sync(&mut self, targets: BTreeSet<(String, LibraryFormat)>, mut req: ExportRequest, ids: BTreeSet<String>) {
        let rows = ids.into_iter().map(|id| {
            let name = self.lib.favs.iter().find(|p| p.lcsc.eq_ignore_ascii_case(&id)).map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
            let mut paths = Vec::new();
            if let Some(path) = req.ad_library_files.get(&id) { paths.push(format!("AD · {}", path.display())); }
            if let Some(path) = req.kicad_library_files.get(&id) { paths.push(format!("KiCad · {}", path.display())); }
            SyncRow { id, name, targets: paths.join("\n"), state: RowState::Waiting, error: None, detail: None }
        }).collect();
        let task = LibrarySyncTask::new(rows, targets, Stage::Running);
        if task.progress.lock().unwrap().rows.is_empty() {
            task.progress.lock().unwrap().finish(); self.library_sync_task = Some(task); return;
        }
        if self.out_dir.trim().is_empty() {
            task.fail(i18n::t(self.lang, "choose_output").into()); self.library_sync_task = Some(task); self.show_library_sync = true; return;
        }
        let worker = LibrarySyncTask { progress: task.progress.clone(), cancel: task.cancel.clone(), targets: task.targets.clone(), can_retry: true };
        #[cfg(windows)]
        if !req.ad_library_files.is_empty() {
            if let Some(root) = crate::profile_dir().map(|p| p.join("ad-bridge")) {
                if lceda_core::ad_bridge::enabled(&root) || lceda_core::ad_bridge::connection(&root).is_ok() {
                    let progress = task.progress.clone();
                    let lang = self.lang;
                    req.ad_bridge = Some(lceda_core::ad_bridge::Bridge::new(root).with_progress(move |phase| {
                        let key = format!("ad_phase_{phase}");
                        let text = i18n::t(lang, &key).to_string();
                        if let Some(row) = progress.lock().unwrap().rows.iter_mut().find(|r| r.state == RowState::Syncing) { row.detail = Some(text); }
                    }));
                }
            }
        }
        if !req.ad_library_files.is_empty() {
            let ready = if let Some(bridge) = &req.ad_bridge { lceda_core::ad_bridge::connection(&bridge.directory).map(|_| ()) }
                else { lceda_core::ad_bridge::require_editor_closed() };
            if let Err(error) = ready {
                task.fail(error.to_string()); self.library_sync_task = Some(task); self.show_library_sync = true; return;
            }
        }
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
        #[allow(unused_mut)]
        let mut connect_ad = false;
        fit_window(i18n::t(lang, "sync_progress"), "library_sync_progress", 660.0).open(&mut open).show(ctx, |ui| {
            ui.set_width(636.0);
            ui.horizontal_wrapped(|ui| {
                if running { ui.add(egui::Spinner::new().size(16.0)); }
                let key = if stopping && running { "sync_stopping" } else if p.stage == Stage::AwaitingAd { "ad_queue_waiting" } else if p.stage == Stage::Validating { "sync_validating" }
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
                        if let Some(detail) = &row.detail { ui.add(egui::Label::new(egui::RichText::new(detail).small().color(theme::secondary())).wrap().selectable(true)); }
                    });
                    ui.add_space(4.0);
                }
            });
            ui.add_space(8.0);
            if running { ui.add(egui::Label::new(egui::RichText::new(i18n::t(lang, if p.stage == Stage::AwaitingAd { "ad_queue_hint" } else { "sync_cancel_hint" })).small().color(theme::secondary())).wrap()); }
            ui.horizontal_wrapped(|ui| {
                close = theme::pill_button(ui, i18n::t(lang, if running { "sync_background" } else { "close" }), true, !running).clicked();
                if running { cancel = theme::pill_button(ui, i18n::t(lang, if stopping { "sync_stopping" } else { "sync_cancel" }), !stopping, false).clicked(); }
                else if task.can_retry && (failed > 0 || skipped > 0 || p.note.is_some()) {
                    retry = theme::pill_button(ui, i18n::t(lang, "sync_retry"), !self.busy(), false).clicked();
                }
                #[cfg(windows)]
                if (!running || p.stage == Stage::AwaitingAd) && task.targets.iter().any(|(_, format)| *format == LibraryFormat::Ad) {
                    connect_ad = theme::pill_button(ui, i18n::t(lang, "ad_connect"), true, false).clicked();
                }
            });
        });
        if cancel {
            task.cancel.store(true, Ordering::SeqCst);
            if p.stage == Stage::AwaitingAd {
                let targets = task.targets.clone(); task.progress.lock().unwrap().finish();
                for target in targets { self.pending_library_sync.remove(&target); }
            }
        }
        if connect_ad { self.show_library_sync = false; self.show_ad_connection = true; }
        if retry {
            let targets = task.targets.clone();
            for (category, format) in targets { self.queue_library_sync(&category, Some(format)); }
            self.library_sync_after = None;
        }
        if cancel && p.stage == Stage::AwaitingAd { self.persist_sync_queue(); }
        if close || !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.show_library_sync = false; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> LibrarySyncTask {
        LibrarySyncTask::new((1001..=1003).map(|n| SyncRow { id: format!("C{n}"), name: String::new(), targets: "AD · test.SchLib".into(), state: RowState::Waiting, error: None, detail: None }).collect(), BTreeSet::new(), Stage::Running)
    }
    fn item(id: &str) -> SearchItem {
        SearchItem { index: 1, display_title: id.into(), title: id.into(), manufacturer: String::new(),
            model_uuid: None, raw: serde_json::json!({"product_code": id}) }
    }
    #[test]
    fn queued_scopes_drop_removed_or_rebound_libraries_and_keep_formats_separate() {
        let mut library = Library::default();
        let root = std::env::temp_dir().join(format!("lceda-queue-{}", lceda_core::util::unique_id()));
        std::fs::create_dir_all(&root).unwrap();
        library.set_binding(UNCAT, LibraryFormat::Ad, &root.join("User.SchLib")).unwrap();
        library.set_binding(UNCAT, LibraryFormat::KiCad, &root.join("User.kicad_sym")).unwrap();
        let targets = BTreeSet::from([(UNCAT.into(), LibraryFormat::Ad), (UNCAT.into(), LibraryFormat::KiCad)]);
        let scopes = queue_scopes(&library, &targets);
        let path = root.join("pending.json"); write_queue(&path, &scopes).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let restored = restore_scopes(&library, serde_json::from_value(json["scopes"].clone()).unwrap());
        assert_eq!(restored, targets);
        #[cfg(windows)]
        {
            let regular = library.binding_for(UNCAT, LibraryFormat::Ad).unwrap().to_string_lossy().trim_start_matches("\\\\?\\").to_string();
            let restored = restore_scopes(&library, vec![QueuedScope { category: UNCAT.into(), format: LibraryFormat::Ad, binding: PathBuf::from(regular) }]);
            assert_eq!(restored, BTreeSet::from([(UNCAT.into(), LibraryFormat::Ad)]));
        }
        library.set_binding(UNCAT, LibraryFormat::Ad, &root.join("Different.SchLib")).unwrap();
        let restored = restore_scopes(&library, scopes);
        assert_eq!(restored, BTreeSet::from([(UNCAT.into(), LibraryFormat::KiCad)]));
        write_queue(&path, &[]).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(json["scopes"].as_array().unwrap().len(), 0);
        std::fs::remove_dir_all(root).unwrap();
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
