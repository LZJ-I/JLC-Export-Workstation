use super::preview3d::{self, GpuPreview};
use super::theme::{self, ACCENT, LABEL, SECONDARY, WELL, WINDOW_BG};
use crate::i18n::{self, Lang};
use crate::update::{self, CheckResult, UpdateInfo, UpdatePhase, UpdateProgress};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use eframe::egui_glow::glow;
use lceda_core::client::LcedaClient;
use lceda_core::export::{ExportRequest, export};
use lceda_core::mesh::{self, Mesh};
use lceda_core::models::SearchItem;
use poll_promise::Promise;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

type SearchPromise = Promise<lceda_core::Result<Vec<SearchItem>>>;
type ExportPromise = Promise<lceda_core::Result<String>>;
type PreviewPromise = Promise<PreviewData>;
type UpdatePromise = Promise<CheckResult>;
type ApplyPromise = Promise<Result<(), String>>;

const GAP: f32 = 10.0;
const BTN_H: f32 = 32.0;
const CARD_PAD: f32 = 12.0;

const ICON_PNG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.png"));

#[derive(Clone, Copy)]
struct BatchOpts {
    step: bool,
    obj: bool,
    ad: bool,
    kicad: bool,
    pads: bool,
    datasheet: bool,
    source: bool,
}

impl Default for BatchOpts {
    fn default() -> Self {
        Self {
            step: true,
            obj: false,
            ad: true,
            kicad: true,
            pads: false,
            datasheet: false,
            source: false,
        }
    }
}

impl BatchOpts {
    fn any(self) -> bool {
        self.step || self.obj || self.ad || self.kicad || self.pads || self.datasheet || self.source
    }

    fn request(self, out_dir: PathBuf) -> ExportRequest {
        ExportRequest {
            step: self.step,
            obj: self.obj,
            ad: self.ad,
            kicad: self.kicad,
            pads: self.pads,
            datasheet: self.datasheet,
            source_json: self.source || self.ad || self.kicad || self.pads,
            force: true,
            out_dir,
            ad_embed_3d: true,
            kicad_attach_3d: true,
            merge: false,
            merge_name: "lceda".into(),
        }
    }
}

struct PreviewData {
    image: Option<Vec<u8>>,
    mesh: Option<Mesh>,
    mesh_note: Option<String>,
}

pub fn run(lang: Lang) -> anyhow::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(i18n::t(lang, "app_title"))
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([960.0, 620.0])
        .with_transparent(false)
        .with_decorations(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "lceda",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(App::new(lang, cc.gl.clone())))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

struct App {
    lang: Lang,
    keyword: String,
    out_dir: String,
    items: Vec<SearchItem>,
    selected: Option<usize>,
    logs: Vec<String>,
    search: Option<SearchPromise>,
    job: Option<ExportPromise>,
    preview: Option<PreviewPromise>,
    image_tex: Option<TextureHandle>,
    mesh: Option<Arc<Mesh>>,
    mesh_tex: Option<TextureHandle>,
    mesh_note: Option<String>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    alert: Option<String>,
    show_about: bool,
    show_settings: bool,
    show_batch: bool,
    batch_opts: BatchOpts,
    about_note: Option<String>,
    gpu: Option<Arc<egui::mutex::Mutex<GpuPreview>>>,
    update_check: Option<UpdatePromise>,
    update_manual: bool,
    update: Option<UpdateInfo>,
    update_job: Option<ApplyPromise>,
    update_progress: Option<update::ProgressHandle>,
    frame: u32,
    pending_search: Option<String>,
    shot_path: Option<PathBuf>,
    shot_requested: bool,
    shot_settle: u32,
    ad_embed_3d: bool,
    kicad_attach_3d: bool,
    batch_merge: bool,
}

impl App {
    fn new(lang: Lang, gl: Option<Arc<glow::Context>>) -> Self {
        update::cleanup_old_binary();
        let out_dir = default_out_dir();
        let logs = vec![
            i18n::t(lang, "no_dotnet").into(),
            format!("{}  {}", i18n::t(lang, "output"), out_dir),
        ];
        let gpu = gl.and_then(|ctx| {
            GpuPreview::new(ctx.as_ref()).map(|g| Arc::new(egui::mutex::Mutex::new(g)))
        });
        let skip_update = env::var("LCEDA_SHOT").is_ok();
        let prefs = crate::prefs::load();
        Self {
            lang,
            keyword: String::new(),
            out_dir,
            items: Vec::new(),
            selected: None,
            logs,
            search: None,
            job: None,
            preview: None,
            image_tex: None,
            mesh: None,
            mesh_tex: None,
            mesh_note: None,
            yaw: 0.7,
            pitch: 0.55,
            zoom: 1.0,
            alert: None,
            show_about: false,
            show_settings: false,
            show_batch: false,
            batch_opts: BatchOpts::default(),
            about_note: None,
            gpu,
            update_check: if skip_update {
                None
            } else {
                Some(Promise::spawn_thread("update", update::check_for_update))
            },
            update_manual: false,
            update: None,
            update_job: None,
            update_progress: None,
            frame: 0,
            pending_search: env::var("LCEDA_SEARCH").ok().filter(|s| !s.is_empty()),
            shot_path: env::var("LCEDA_SHOT").ok().filter(|s| !s.is_empty()).map(PathBuf::from),
            shot_requested: false,
            shot_settle: 0,
            ad_embed_3d: prefs.ad_embed_3d,
            kicad_attach_3d: prefs.kicad_attach_3d,
            batch_merge: prefs.batch_merge,
        }
    }

    fn log(&mut self, msg: impl Into<String>) {
        self.logs.push(msg.into());
        if self.logs.len() > 200 {
            self.logs.remove(0);
        }
    }

    fn alert(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.log(msg.clone());
        self.alert = Some(msg);
    }

    fn show_alert(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.alert.clone() else {
            return;
        };
        let mut close = false;
        let line_count = msg.lines().count().max(1);
        let longest = msg.lines().map(|l| l.chars().count()).max().unwrap_or(12);
        let width = ((longest as f32) * 8.5 + 40.0).clamp(300.0, 440.0);
        let tall = line_count > 8;
        fit_window(i18n::t(self.lang, "notice"), "lceda_notice_fit", width)
            .max_height(if tall { 280.0 } else { 800.0 })
            .show(ctx, |ui| {
                ui.set_width(width - 8.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                let add_text = |ui: &mut egui::Ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&msg).color(LABEL))
                            .wrap()
                            .halign(egui::Align::Min),
                    );
                };
                if tall {
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .auto_shrink([false, true])
                        .show(ui, add_text);
                } else {
                    add_text(ui);
                }
                ui.add_space(8.0);
                if dialog_ok_row(ui, i18n::t(self.lang, "ok")) {
                    close = true;
                }
            });
        if close {
            self.alert = None;
        }
    }

    fn show_about(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut close = false;
        let mut open_repo = false;
        let mut check = false;
        let width = 420.0;
        let checking = self.update_check.is_some();
        let can_check = self.update_job.is_none() && self.update.is_none();
        let check_label = if checking {
            i18n::t(self.lang, "checking_update")
        } else {
            i18n::t(self.lang, "check_update")
        };
        fit_window(i18n::t(self.lang, "about"), "lceda_about_fit", width).show(ctx, |ui| {
            ui.set_width(width - 8.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(
                egui::RichText::new(i18n::t(self.lang, "app_title"))
                    .color(LABEL)
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(6.0);
            kv_line(ui, i18n::t(self.lang, "about_ver"), update::current_version());
            kv_line(ui, i18n::t(self.lang, "about_author"), "LZJ-I");
            kv_line(ui, i18n::t(self.lang, "about_repo"), update::REPO);
            kv_line(ui, i18n::t(self.lang, "about_license"), "CC BY-NC-4.0");
            if let Some(note) = &self.about_note {
                ui.add_space(6.0);
                ui.add(egui::Label::new(egui::RichText::new(note).color(SECONDARY)).wrap());
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::pill_button(ui, i18n::t(self.lang, "ok"), true, true).clicked() {
                        close = true;
                    }
                    if theme::pill_button(ui, i18n::t(self.lang, "open_repo"), true, false).clicked()
                    {
                        open_repo = true;
                    }
                    if theme::pill_button(ui, check_label, can_check, false).clicked() {
                        check = true;
                    }
                });
            });
        });
        if check {
            self.request_update_check(true);
        }
        if open_repo {
            let _ = webbrowser::open(update::REPO_URL);
        }
        if close {
            self.show_about = false;
            self.about_note = None;
        }
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut close = false;
        let mut persist = false;
        let lang = self.lang;
        let width = 420.0;
        fit_window(i18n::t(lang, "settings"), "lceda_settings_fit", width).show(ctx, |ui| {
            ui.set_width(width - 8.0);
            ui.label(egui::RichText::new(i18n::t(lang, "settings_3d_hint")).color(SECONDARY));
            ui.add_space(8.0);
            let before_ad = self.ad_embed_3d;
            let before_kicad = self.kicad_attach_3d;
            ui.checkbox(&mut self.ad_embed_3d, i18n::t(lang, "ad_embed_3d"));
            ui.checkbox(&mut self.kicad_attach_3d, i18n::t(lang, "kicad_attach_3d"));
            if self.ad_embed_3d != before_ad || self.kicad_attach_3d != before_kicad {
                persist = true;
            }
            ui.add_space(10.0);
            if dialog_ok_row(ui, i18n::t(lang, "ok")) {
                close = true;
            }
        });
        if persist {
            self.persist_prefs();
        }
        if close {
            self.show_settings = false;
        }
    }

    fn show_batch_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_batch {
            return;
        }
        let mut close = false;
        let mut start = false;
        let lang = self.lang;
        let width = 400.0;
        fit_window(i18n::t(lang, "batch_title"), "lceda_batch_fit", width).show(ctx, |ui| {
            ui.set_width(width - 8.0);
            ui.label(egui::RichText::new(i18n::t(lang, "batch_hint")).color(SECONDARY));
            ui.add_space(8.0);
            egui::Grid::new("batch_opts")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.checkbox(&mut self.batch_opts.step, i18n::t(lang, "download_step"));
                    ui.checkbox(&mut self.batch_opts.obj, i18n::t(lang, "download_obj"));
                    ui.end_row();
                    ui.checkbox(&mut self.batch_opts.ad, i18n::t(lang, "export_ad"));
                    ui.checkbox(&mut self.batch_opts.kicad, i18n::t(lang, "export_kicad"));
                    ui.end_row();
                    ui.checkbox(&mut self.batch_opts.pads, i18n::t(lang, "export_pads"));
                    ui.checkbox(&mut self.batch_opts.datasheet, i18n::t(lang, "datasheet"));
                    ui.end_row();
                    ui.checkbox(&mut self.batch_opts.source, i18n::t(lang, "export_source"));
                    ui.end_row();
                });
            if self.batch_opts.ad || self.batch_opts.kicad {
                ui.add_space(4.0);
                let before = self.batch_merge;
                ui.checkbox(&mut self.batch_merge, i18n::t(lang, "batch_merge"));
                if self.batch_merge != before {
                    self.persist_prefs();
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::pill_button(ui, i18n::t(lang, "batch_start"), true, true).clicked() {
                        start = true;
                    }
                    if theme::pill_button(ui, i18n::t(lang, "batch_cancel"), true, false).clicked() {
                        close = true;
                    }
                });
            });
        });
        if start {
            if !self.batch_opts.any() {
                self.alert(i18n::t(self.lang, "batch_none"));
            } else if self.start_batch() {
                close = true;
            }
        }
        if close {
            self.show_batch = false;
        }
    }

    fn request_update_check(&mut self, manual: bool) {
        if self.update.is_some() || self.update_job.is_some() {
            if manual {
                self.show_about = false;
            }
            return;
        }
        if self.update_check.is_some() {
            if manual {
                self.update_manual = true;
                self.about_note = None;
            }
            return;
        }
        self.update_manual = manual;
        self.about_note = None;
        self.update_check = Some(Promise::spawn_thread("update", update::check_for_update));
    }

    fn show_update(&mut self, ctx: &egui::Context) {
        let Some(info) = self.update.clone() else {
            return;
        };
        let downloading = self.update_job.is_some();
        let mut later = false;
        let mut now = false;
        let mut open = false;
        let body = i18n::t(self.lang, "update_body")
            .replace("{cur}", update::current_version())
            .replace("{new}", &info.version);
        let notes = info.notes.trim();
        let width = if notes.is_empty() { 400.0 } else { 440.0 };
        let tall = notes.lines().count() > 8 || notes.chars().count() > 240;
        fit_window(i18n::t(self.lang, "update_title"), "lceda_update_fit", width)
            .max_height(if tall { 420.0 } else { 800.0 })
            .show(ctx, |ui| {
                ui.set_width(width - 8.0);
                ui.add(egui::Label::new(egui::RichText::new(body).color(LABEL)).wrap());
                if !notes.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(i18n::t(self.lang, "update_notes"))
                            .color(SECONDARY)
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                    let add_notes = |ui: &mut egui::Ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(notes).color(LABEL).size(13.0))
                                .wrap()
                                .halign(egui::Align::Min),
                        );
                    };
                    if tall {
                        egui::ScrollArea::vertical()
                            .id_salt("update_notes")
                            .max_height(220.0)
                            .auto_shrink([false, true])
                            .show(ui, add_notes);
                    } else {
                        add_notes(ui);
                    }
                }
                if downloading {
                    ui.add_space(8.0);
                    let (label, fraction, indeterminate) = self
                        .update_progress
                        .as_ref()
                        .and_then(|p| p.lock().ok())
                        .map(|g| {
                            let key = match g.phase {
                                UpdatePhase::Extracting => "update_extracting",
                                UpdatePhase::Installing => "update_installing",
                                _ => "update_downloading",
                            };
                            (i18n::t(self.lang, key), g.fraction, g.indeterminate)
                        })
                        .unwrap_or((i18n::t(self.lang, "update_downloading"), 0.0, true));
                    ui.label(egui::RichText::new(label).color(SECONDARY));
                    ui.add_space(4.0);
                    if indeterminate {
                        ui.add(egui::ProgressBar::new(0.0).animate(true));
                    } else {
                        ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !downloading {
                            if info.zip_url.is_some()
                                && theme::pill_button(
                                    ui,
                                    i18n::t(self.lang, "update_now"),
                                    true,
                                    true,
                                )
                                .clicked()
                            {
                                now = true;
                            }
                            if theme::pill_button(ui, i18n::t(self.lang, "update_open"), true, false)
                                .clicked()
                            {
                                open = true;
                            }
                            if theme::pill_button(
                                ui,
                                i18n::t(self.lang, "update_later"),
                                true,
                                false,
                            )
                            .clicked()
                            {
                                later = true;
                            }
                        }
                    });
                });
            },
        );
        if open {
            let _ = webbrowser::open(&info.page_url);
        }
        if later {
            self.update = None;
        }
        if now {
            let info = info.clone();
            let progress = Arc::new(Mutex::new(UpdateProgress::default()));
            let progress_for_job = Arc::clone(&progress);
            self.update_progress = Some(progress);
            self.update_job = Some(Promise::spawn_thread("apply-update", move || {
                update::download_and_apply(&info, Some(progress_for_job))
            }));
        }
    }

    fn busy(&self) -> bool {
        self.search.is_some() || self.job.is_some()
    }

    fn selected_item(&self) -> Option<&SearchItem> {
        self.selected.and_then(|i| self.items.get(i))
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [
            WINDOW_BG.r() as f32 / 255.0,
            WINDOW_BG.g() as f32 / 255.0,
            WINDOW_BG.b() as f32 / 255.0,
            1.0,
        ]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_jobs(ctx);

        egui::TopBottomPanel::top("bar")
            .exact_height(56.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::fill())
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .stroke(egui::Stroke::new(1.0_f32, theme::hairline())),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.heading(egui::RichText::new(i18n::t(self.lang, "app_title")).color(LABEL));
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(i18n::t(self.lang, "no_dotnet"))
                            .color(SECONDARY)
                            .small(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.lang == Lang::Zh {
                            "中文 / EN"
                        } else {
                            "EN / 中文"
                        };
                        if theme::pill_button(ui, label, true, false).clicked() {
                            self.lang = self.lang.toggle();
                        }
                        ui.add_space(6.0);
                        if theme::pill_button(ui, i18n::t(self.lang, "about"), true, false).clicked()
                        {
                            self.show_about = true;
                            self.about_note = None;
                        }
                        ui.add_space(6.0);
                        if theme::pill_button(ui, i18n::t(self.lang, "settings"), true, false).clicked()
                        {
                            self.show_settings = true;
                        }
                    });
                });
            });

        egui::SidePanel::left("parts")
            .resizable(true)
            .default_width(380.0)
            .min_width(300.0)
            .max_width(520.0)
            .frame(egui::Frame::new().inner_margin(12.0).fill(WINDOW_BG))
            .show(ctx, |ui| {
                self.left_pane(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(12.0).fill(WINDOW_BG))
            .show(ctx, |ui| {
                self.right_pane(ui);
            });
        self.show_alert(ctx);
        self.show_about(ctx);
        self.show_settings(ctx);
        self.show_batch_dialog(ctx);
        self.show_update(ctx);
        self.tick_debug_shot(ctx);
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        if let (Some(gl), Some(gpu)) = (gl, self.gpu.take()) {
            gpu.lock().destroy(gl);
        }
    }
}

impl App {
    fn poll_jobs(&mut self, ctx: &egui::Context) {
        if let Some(p) = &self.search {
            if p.ready().is_some() {
                match self.search.take().unwrap().block_and_take() {
                    Ok(items) => {
                        self.log(format!("{}: {}", i18n::t(self.lang, "search"), items.len()));
                        self.items = items;
                        self.selected = if self.items.is_empty() { None } else { Some(0) };
                        if self.items.is_empty() {
                            self.alert(i18n::t(self.lang, "no_results"));
                        } else {
                            self.queue_preview();
                        }
                    }
                    Err(e) => self.alert(format!("{}: {e}", i18n::t(self.lang, "error"))),
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.job {
            if p.ready().is_some() {
                match self.job.take().unwrap().block_and_take() {
                    Ok(msg) => {
                        self.alert(msg);
                    }
                    Err(e) => {
                        self.alert(format!("{}: {e}", i18n::t(self.lang, "error")));
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.preview {
            if p.ready().is_some() {
                let data = self.preview.take().unwrap().block_and_take();
                self.image_tex = data.image.as_ref().and_then(|b| load_texture(ctx, b));
                self.mesh = data.mesh.map(Arc::new);
                self.mesh_note = data.mesh_note;
                self.mesh_tex = None;
                if let Some(note) = &self.mesh_note {
                    self.log(note.clone());
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.update_check {
            if p.ready().is_some() {
                let result = self.update_check.take().unwrap().block_and_take();
                let manual = self.update_manual;
                self.update_manual = false;
                match result {
                    CheckResult::Available(info) => {
                        self.update = Some(info);
                        self.show_about = false;
                        self.about_note = None;
                    }
                    CheckResult::UpToDate if manual => {
                        let note = i18n::t(self.lang, "already_latest")
                            .replace("{ver}", update::current_version());
                        if self.show_about {
                            self.about_note = Some(note);
                        } else {
                            self.alert(note);
                        }
                    }
                    CheckResult::Failed if manual => {
                        let note = i18n::t(self.lang, "update_check_fail").to_string();
                        if self.show_about {
                            self.about_note = Some(note);
                        } else {
                            self.alert(note);
                        }
                    }
                    _ => {}
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.update_job {
            if p.ready().is_some() {
                match self.update_job.take().unwrap().block_and_take() {
                    Ok(()) => {
                        self.update = None;
                        self.update_progress = None;
                    }
                    Err(e) => {
                        self.update_progress = None;
                        self.alert(e);
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
    }

    fn left_pane(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.label(
                egui::RichText::new(i18n::t(self.lang, "keyword"))
                    .strong()
                    .color(LABEL),
            );
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut self.keyword)
                    .hint_text(i18n::t(self.lang, "keyword_hint"))
                    .desired_width(ui.available_width() - 76.0);
                let resp = ui.add(edit);
                let go = theme::pill_button(ui, i18n::t(self.lang, "search"), !self.busy(), true).clicked();
                if (go || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !self.busy()
                {
                    self.do_search();
                }
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(i18n::t(self.lang, "components")).strong());
            egui::ScrollArea::vertical().id_salt("part_list").show(ui, |ui| {
                ui.set_width(ui.available_width());
                let mut clicked = None;
                for (idx, item) in self.items.iter().enumerate() {
                    let selected = self.selected == Some(idx);
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 44.0),
                        egui::Sense::click(),
                    );
                    let bg = if selected {
                        Color32::from_rgba_unmultiplied(0, 122, 255, 36)
                    } else if resp.hovered() {
                        WELL
                    } else {
                        Color32::TRANSPARENT
                    };
                    if bg != Color32::TRANSPARENT {
                        ui.painter().rect_filled(rect, 8.0, bg);
                    }
                    ui.painter().text(
                        egui::pos2(rect.left() + 10.0, rect.top() + 6.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}.  {}", item.index, item.name()),
                        egui::FontId::proportional(14.0),
                        LABEL,
                    );
                    let mut sub = item.lcsc_id().unwrap_or_default();
                    if item.model_uuid.is_some() {
                        if !sub.is_empty() {
                            sub.push_str("  ·  ");
                        }
                        sub.push_str("3D");
                    }
                    if !sub.is_empty() {
                        ui.painter().text(
                            egui::pos2(rect.left() + 10.0, rect.top() + 24.0),
                            egui::Align2::LEFT_TOP,
                            sub,
                            egui::FontId::proportional(12.0),
                            SECONDARY,
                        );
                    }
                    if resp.clicked() {
                        clicked = Some(idx);
                    }
                }
                if let Some(idx) = clicked {
                    self.selected = Some(idx);
                    self.queue_preview();
                }
            });
        });
    }

    fn right_pane(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("right_pane")
                .max_rect(area)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(area.intersect(ui.clip_rect()));
                ui.set_min_size(area.size());
                ui.set_max_size(area.size());

                let mut top_h = (area.height() * 0.38).clamp(280.0, 320.0);
                let min_bottom = 220.0;
                if area.height() < top_h + GAP + min_bottom {
                    top_h = (area.height() - GAP - min_bottom).max(240.0);
                }
                let mut photo_w = (top_h * 0.78).clamp(200.0, 252.0);
                let min_actions = 260.0;
                if area.width() - photo_w - GAP < min_actions {
                    photo_w = (area.width() - GAP - min_actions).max(168.0);
                }

                let top = egui::Rect::from_min_size(area.min, egui::vec2(area.width(), top_h));
                let photo = egui::Rect::from_min_size(top.min, egui::vec2(photo_w, top_h));
                let actions = egui::Rect::from_min_max(
                    egui::pos2(photo.max.x + GAP, top.min.y),
                    egui::pos2(top.max.x, top.max.y),
                );
                let mesh = egui::Rect::from_min_max(
                    egui::pos2(area.min.x, top.max.y + GAP),
                    area.max,
                );

                self.photo_card(ui, photo);
                self.action_card(ui, actions);
                self.mesh_card(ui, mesh);
            },
        );
        ui.advance_cursor_after_rect(area);
    }

    fn photo_card(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        card_shell(ui, rect, "photo", |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(i18n::t(self.lang, "preview")).strong());
                if self.image_tex.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(i18n::t(self.lang, "preview_zoom"))
                                .color(SECONDARY)
                                .small(),
                        );
                    });
                }
            });
            let h = ui.max_rect().height();
            let reserved = 28.0 + if self.selected_item().is_some() { 78.0 } else { 10.0 };
            let well_h = (h - reserved).max(80.0);
            let well_w = ui.max_rect().width();
            let (well, _) = ui.allocate_exact_size(egui::vec2(well_w, well_h), egui::Sense::hover());
            paint_well(ui, well);
            let painter = ui.painter_at(well);
            if self.preview.is_some() && self.image_tex.is_none() {
                painter.text(
                    well.center(),
                    egui::Align2::CENTER_CENTER,
                    i18n::t(self.lang, "loading"),
                    egui::FontId::proportional(14.0),
                    SECONDARY,
                );
            } else if let Some(tex) = &self.image_tex {
                let size = tex.size_vec2();
                let inner = well.shrink(8.0);
                let scale = (inner.width() / size.x).min(inner.height() / size.y);
                let draw = size * scale;
                let img_rect = egui::Rect::from_center_size(inner.center(), draw).intersect(inner);
                painter.image(
                    tex.id(),
                    img_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                    if img_rect.contains(pointer) {
                        paint_photo_zoom(ui, tex, img_rect, pointer, rect);
                    }
                }
            } else {
                painter.text(
                    well.center(),
                    egui::Align2::CENTER_CENTER,
                    i18n::t(self.lang, "preview_none"),
                    egui::FontId::proportional(13.0),
                    SECONDARY,
                );
            }
            if let Some(item) = self.selected_item() {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(item.name()).color(LABEL).size(14.0).strong())
                        .truncate(),
                );
                if let Some(id) = item.lcsc_id() {
                    ui.label(egui::RichText::new(id).color(ACCENT).small());
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "{}  {}",
                            i18n::t(self.lang, "manufacturer"),
                            if item.manufacturer.is_empty() {
                                "—"
                            } else {
                                &item.manufacturer
                            }
                        ))
                        .color(SECONDARY)
                        .small(),
                    )
                    .truncate(),
                );
            }
        });
    }

    fn mesh_card(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        card_shell(ui, rect, "mesh", |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(i18n::t(self.lang, "model3d")).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.mesh.is_some() {
                        ui.label(
                            egui::RichText::new(i18n::t(self.lang, "drag_orbit"))
                                .color(SECONDARY)
                                .small(),
                        );
                    }
                });
            });
            let well_h = ui.available_height().max(80.0);
            let well_w = ui.max_rect().width();
            let (well, resp) = ui.allocate_exact_size(
                egui::vec2(well_w, well_h),
                egui::Sense::click_and_drag(),
            );
            paint_well(ui, well);
            if resp.dragged() {
                let d = resp.drag_delta();
                self.yaw += d.x * 0.01;
                self.pitch += d.y * 0.01;
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.zoom = (self.zoom * (1.0 + scroll * 0.003)).clamp(0.15, 8.0);
                }
            }
            if resp.double_clicked() {
                self.yaw = 0.7;
                self.pitch = 0.55;
                self.zoom = 1.0;
            }
            let painter = ui.painter_at(well);
            let inner = well.shrink(8.0);
            if self.preview.is_some() && self.mesh.is_none() {
                painter.text(
                    well.center(),
                    egui::Align2::CENTER_CENTER,
                    i18n::t(self.lang, "loading"),
                    egui::FontId::proportional(14.0),
                    SECONDARY,
                );
            } else if let (Some(gpu), Some(mesh)) = (self.gpu.clone(), self.mesh.clone()) {
                ui.painter().add(preview3d::paint_callback(
                    inner,
                    gpu,
                    mesh,
                    self.yaw,
                    self.pitch,
                    self.zoom,
                ));
            } else if let Some(mesh) = self.mesh.as_ref() {
                let image = rasterize_mesh(
                    mesh,
                    inner,
                    self.yaw,
                    self.pitch,
                    self.zoom,
                    ui.ctx().pixels_per_point(),
                );
                let tex = ui.ctx().load_texture("mesh3d", image, TextureOptions::LINEAR);
                painter.image(
                    tex.id(),
                    inner,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                self.mesh_tex = Some(tex);
            } else {
                let msg = if self.selected_item().map(|i| i.model_uuid.is_some()).unwrap_or(false) {
                    self.mesh_note
                        .as_deref()
                        .unwrap_or(i18n::t(self.lang, "mesh_fail"))
                } else {
                    i18n::t(self.lang, "no_3d")
                };
                painter.text(
                    well.center(),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    egui::FontId::proportional(13.0),
                    SECONDARY,
                );
            }
        });
    }

    fn action_card(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let has3d = self.selected_item().map(|i| i.model_uuid.is_some()).unwrap_or(false);
        let has_ad = self
            .selected_item()
            .map(|i| i.has_symbol_or_footprint())
            .unwrap_or(false);
        let has_item = self.selected_item().is_some();
        let has_ds = self
            .selected_item()
            .and_then(|i| i.datasheet_url())
            .is_some();
        let en = !self.busy() && has_item;
        let lang = self.lang;

        let mut step = false;
        let mut obj = false;
        let mut ad = false;
        let mut kicad = false;
        let mut datasheet = false;
        let mut page = false;
        let mut pads = false;
        let mut batch = false;
        let mut browse = false;
        let mut open = false;

        card_shell(ui, rect, "actions", |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            ui.set_width(ui.available_width());
            let pairs = [
                (
                    i18n::t(lang, "download_step"),
                    en && has3d,
                    true,
                    i18n::t(lang, "download_obj"),
                    en && has3d,
                    false,
                ),
                (
                    i18n::t(lang, "export_ad"),
                    en && has_ad,
                    true,
                    i18n::t(lang, "export_kicad"),
                    en && has_ad,
                    true,
                ),
                (
                    i18n::t(lang, "datasheet"),
                    en && has_ds,
                    false,
                    i18n::t(lang, "open_page"),
                    has_item,
                    false,
                ),
                (
                    i18n::t(lang, "export_pads"),
                    en && has_ad,
                    true,
                    i18n::t(lang, "batch"),
                    !self.busy(),
                    false,
                ),
            ];
            let clicks = action_grid(ui, "action_btns", &pairs);
            step = clicks[0].0;
            obj = clicks[0].1;
            ad = clicks[1].0;
            kicad = clicks[1].1;
            datasheet = clicks[2].0;
            page = clicks[2].1;
            pads = clicks[3].0;
            batch = clicks[3].1;

            ui.add_space(4.0);
            ui.label(egui::RichText::new(i18n::t(lang, "output")).strong().color(LABEL));
            let path_w = ui.available_width();
            ui.add_sized(
                egui::vec2(path_w, 28.0),
                egui::TextEdit::singleline(&mut self.out_dir)
                    .desired_width(path_w)
                    .clip_text(true)
                    .hint_text(i18n::t(lang, "output_hint")),
            );
            let path_clicks = action_grid(
                ui,
                "path_btns",
                &[(
                    i18n::t(lang, "browse"),
                    true,
                    false,
                    i18n::t(lang, "open_folder"),
                    true,
                    true,
                )],
            );
            browse = path_clicks[0].0;
            open = path_clicks[0].1;
        });

        if browse {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                self.out_dir = dir.display().to_string();
            }
        }
        if open {
            self.open_out_dir();
        }
        if step {
            self.require_export(has_item, has3d, i18n::t(self.lang, "no_3d_dl"), |req| {
                req.step = true;
            });
        }
        if obj {
            self.require_export(has_item, has3d, i18n::t(self.lang, "no_3d_dl"), |req| {
                req.obj = true;
            });
        }
        if ad {
            self.require_export(has_item, has_ad, i18n::t(self.lang, "no_cad"), |req| {
                req.ad = true;
                req.source_json = true;
            });
        }
        if kicad {
            self.require_export(has_item, has_ad, i18n::t(self.lang, "no_cad"), |req| {
                req.kicad = true;
                req.source_json = true;
            });
        }
        if pads {
            self.require_export(has_item, has_ad, i18n::t(self.lang, "no_cad"), |req| {
                req.pads = true;
                req.source_json = true;
            });
        }
        if datasheet {
            self.require_export(has_item, has_ds, i18n::t(self.lang, "no_datasheet"), |req| {
                req.datasheet = true;
            });
        }
        if page {
            if let Some(it) = self.selected_item() {
                let url = it.product_url();
                if let Err(e) = webbrowser::open(&url) {
                    self.alert(format!("{}: {e}", i18n::t(self.lang, "error")));
                }
            } else {
                self.alert(i18n::t(self.lang, "select_first"));
            }
        }
        if batch {
            if self.busy() {
                self.alert(i18n::t(self.lang, "working"));
            } else {
                self.show_batch = true;
            }
        }
    }

    fn open_out_dir(&mut self) {
        let path = PathBuf::from(self.out_dir.trim());
        if let Err(e) = std::fs::create_dir_all(&path) {
            self.alert(format!("{}: {e}", i18n::t(self.lang, "error")));
            return;
        }
        if !open_folder(&path) {
            self.alert(format!("{}: {}", i18n::t(self.lang, "error"), path.display()));
        }
    }

    fn do_search(&mut self) {
        let kw = self.keyword.trim().to_string();
        if kw.is_empty() {
            self.alert(i18n::t(self.lang, "empty_keyword"));
            return;
        }
        self.log(i18n::t(self.lang, "searching"));
        self.search = Some(Promise::spawn_thread("search", move || LcedaClient::new().search(&kw)));
    }

    fn queue_preview(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };
        self.mesh = None;
        self.mesh_tex = None;
        self.mesh_note = None;
        self.yaw = 0.7;
        self.pitch = 0.55;
        self.zoom = 1.0;
        self.preview = Some(Promise::spawn_thread("preview", move || {
            let client = LcedaClient::new();
            let mut data = PreviewData {
                image: None,
                mesh: None,
                mesh_note: None,
            };
            if let Some(url) = item.image_url() {
                data.image = client.get_bytes(&url).ok();
            }
            if item.model_uuid.is_some() {
                match client.download_obj_bytes(&item) {
                    Ok(bytes) => match load_preview_mesh(&bytes) {
                        Ok(mesh) => data.mesh = Some(mesh),
                        Err(note) => data.mesh_note = Some(note),
                    },
                    Err(e) => data.mesh_note = Some(format!("3D: {e}")),
                }
            }
            data
        }));
    }

    fn require_export(
        &mut self,
        has_item: bool,
        supported: bool,
        unsupported: &str,
        mutate: impl FnOnce(&mut ExportRequest),
    ) {
        if self.busy() {
            self.alert(i18n::t(self.lang, "working"));
            return;
        }
        if !has_item {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        }
        if !supported {
            self.alert(unsupported);
            return;
        }
        self.run_export(mutate);
    }

    fn run_export(&mut self, mutate: impl FnOnce(&mut ExportRequest)) {
        let Some(item) = self.selected_item().cloned() else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        let out_dir = PathBuf::from(self.out_dir.trim());
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.alert(format!("{}: {e}", i18n::t(self.lang, "error")));
            return;
        }
        let mut req = ExportRequest {
            force: true,
            out_dir: out_dir.clone(),
            ..Default::default()
        };
        mutate(&mut req);
        req.ad_embed_3d = self.ad_embed_3d;
        req.kicad_attach_3d = self.kicad_attach_3d;
        self.log(format!("{}  {}", i18n::t(self.lang, "saving_to"), out_dir.display()));
        self.job = Some(Promise::spawn_thread("export", move || {
            let client = LcedaClient::new();
            let paths = export(&client, &item, &req)?;
            Ok(format_paths(&paths, &out_dir))
        }));
    }

    fn persist_prefs(&self) {
        crate::prefs::save(&crate::prefs::Prefs {
            ad_embed_3d: self.ad_embed_3d,
            kicad_attach_3d: self.kicad_attach_3d,
            batch_merge: self.batch_merge,
        });
    }

    fn start_batch(&mut self) -> bool {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("text", &["txt", "csv", "list"])
            .pick_file()
        else {
            return false;
        };
        let out_dir = PathBuf::from(self.out_dir.trim());
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.alert(format!("{}: {e}", i18n::t(self.lang, "error")));
            return false;
        }
        let mut req = self.batch_opts.request(out_dir.clone());
        req.ad_embed_3d = self.ad_embed_3d;
        req.kicad_attach_3d = self.kicad_attach_3d;
        req.merge = self.batch_merge && (self.batch_opts.ad || self.batch_opts.kicad);
        self.log(format!("{}  {}", i18n::t(self.lang, "saving_to"), out_dir.display()));
        self.job = Some(Promise::spawn_thread("batch", move || {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| lceda_core::Error::msg(format!("read {}: {e}", file.display())))?;
            let ids = lceda_core::models::parse_id_list(&text);
            let client = LcedaClient::new();
            let mut lines = Vec::new();
            let mut fail = 0;
            for (kw, result) in lceda_core::export::export_batch(&client, &ids, &req) {
                match result {
                    Ok(paths) => lines.push(format!("OK {kw}\n{}", format_paths(&paths, &out_dir))),
                    Err(e) => {
                        fail += 1;
                        lines.push(format!("FAIL {kw}: {e}"));
                    }
                }
            }
            lines.push(format!("done, failed={fail}"));
            Ok(lines.join("\n"))
        }));
        true
    }

    fn tick_debug_shot(&mut self, ctx: &egui::Context) {
        if let Some(kw) = self.pending_search.take() {
            self.keyword = kw;
            self.do_search();
        }
        if self.shot_path.is_none() {
            return;
        }
        self.frame = self.frame.saturating_add(1);
        ctx.request_repaint();

        if let Some(path) = self.shot_path.clone() {
            let mut got = None;
            ctx.input(|i| {
                for ev in &i.raw.events {
                    if let egui::Event::Screenshot { image, .. } = ev {
                        got = Some(image.clone());
                    }
                }
            });
            if let Some(image) = got {
                if let Err(e) = save_debug_shot(&image, &path) {
                    eprintln!("LCEDA_SHOT failed: {e}");
                    std::process::exit(2);
                }
                std::process::exit(0);
            }
        }

        let jobs_idle = self.search.is_none() && self.preview.is_none() && self.job.is_none();
        let preview_ready = self.mesh.is_some() || self.mesh_note.is_some() || self.image_tex.is_some();
        let waited = if self.keyword.is_empty() {
            self.frame >= 12
        } else {
            self.frame >= 40 && jobs_idle && preview_ready
        };
        if waited {
            self.shot_settle = self.shot_settle.saturating_add(1);
        }
        if self.shot_settle >= 18 && !self.shot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.shot_requested = true;
        }
        if self.frame > 1800 {
            eprintln!("LCEDA_SHOT timed out");
            std::process::exit(3);
        }
    }
}

fn kv_line(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{key}：")).color(SECONDARY));
        ui.label(egui::RichText::new(value).color(LABEL));
    });
}

fn fit_window<'a>(
    title: impl Into<egui::WidgetText>,
    id: &'static str,
    width: f32,
) -> egui::Window<'a> {
    egui::Window::new(title)
        .id(egui::Id::new(id))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .default_width(width)
        .default_height(1.0)
        .min_width(width)
        .max_width(width)
        .min_height(0.0)
}

fn dialog_ok_row(ui: &mut egui::Ui, ok: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::pill_button(ui, ok, true, true).clicked() {
                clicked = true;
            }
        });
    });
    clicked
}

fn card_shell(ui: &mut egui::Ui, rect: egui::Rect, id: &'static str, add: impl FnOnce(&mut egui::Ui)) {
    theme::paint_card(ui.painter(), rect);
    let inner = rect.shrink(CARD_PAD);
    if inner.width() < 8.0 || inner.height() < 8.0 {
        return;
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(inner)
            .layout(egui::Layout::top_down_justified(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(inner.intersect(ui.clip_rect()));
            ui.set_min_width(inner.width());
            ui.set_max_width(inner.width());
            ui.set_max_height(inner.height());
            add(ui);
        },
    );
}

fn paint_well(ui: &egui::Ui, rect: egui::Rect) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 10.0, WELL);
    p.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0_f32, theme::hairline()),
        egui::StrokeKind::Inside,
    );
}

fn action_grid(
    ui: &mut egui::Ui,
    id: &'static str,
    rows: &[(&str, bool, bool, &str, bool, bool)],
) -> Vec<(bool, bool)> {
    const GAP: f32 = 8.0;
    let total = ui.available_width().max(80.0);
    let col = ((total - GAP) / 2.0).floor().max(48.0);
    let mut out = Vec::with_capacity(rows.len());
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([GAP, 6.0])
        .min_col_width(col)
        .max_col_width(col)
        .show(ui, |ui| {
            for row in rows {
                let a = theme::action_button(ui, row.0, row.1, row.2, egui::vec2(col, BTN_H)).clicked();
                let b = theme::action_button(ui, row.3, row.4, row.5, egui::vec2(col, BTN_H)).clicked();
                out.push((a, b));
                ui.end_row();
            }
        });
    out
}

fn load_preview_mesh(bytes: &[u8]) -> Result<Mesh, String> {
    if bytes.starts_with(b"%PDF") || bytes.starts_with(b"<") || bytes.starts_with(b"{") {
        return Err("3D 接口返回的不是 OBJ".into());
    }
    let text = String::from_utf8_lossy(bytes);
    if !text.lines().any(|l| l.trim_start().starts_with("v ")) {
        return Err("OBJ 里没有顶点".into());
    }
    let parsed = mesh::load_preview_obj(&text)?;
    Ok(mesh::compact(&parsed))
}

fn short_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn format_paths(paths: &lceda_core::models::DownloadPaths, out_dir: &Path) -> String {
    if !paths.has_files() {
        return "没有写出文件".into();
    }
    let folder = paths
        .folder
        .as_ref()
        .map(|p| short_name(p))
        .unwrap_or_else(|| out_dir.display().to_string());
    let mut lines = vec!["已保存到".into(), folder];
    if let Some(p) = &paths.step {
        lines.push(format!("STEP  {}", short_name(p)));
    }
    if let Some(p) = &paths.obj {
        lines.push(format!("OBJ  {}", short_name(p)));
    }
    if let Some(p) = &paths.datasheet {
        lines.push(format!("PDF  {}", short_name(p)));
    }
    if let Some(p) = &paths.schlib {
        lines.push(format!("SchLib  {}", short_name(p)));
    }
    if let Some(p) = &paths.pcblib {
        lines.push(format!("PcbLib  {}", short_name(p)));
    }
    if let Some(p) = &paths.kicad_sym {
        lines.push(format!("KiCad  {}", short_name(p)));
    }
    if let Some(p) = &paths.kicad_mod {
        lines.push(format!("封装  {}", short_name(p)));
    }
    if let Some(p) = &paths.pads_c {
        lines.push(format!("PADS .c  {}", short_name(p)));
    }
    if let Some(p) = &paths.pads_d {
        lines.push(format!("PADS .d  {}", short_name(p)));
    }
    if let Some(p) = &paths.pads_p {
        lines.push(format!("PADS .p  {}", short_name(p)));
    }
    if let Some(p) = &paths.symbol_json {
        lines.push(format!("符号 JSON  {}", short_name(p)));
    }
    if let Some(p) = &paths.footprint_json {
        lines.push(format!("封装 JSON  {}", short_name(p)));
    }
    lines.join("\n")
}

fn default_out_dir() -> String {
    directories::UserDirs::new()
        .and_then(|u| {
            u.download_dir()
                .map(|p| p.join("lceda-out"))
                .or_else(|| u.document_dir().map(|p| p.join("lceda-out")))
        })
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "lceda-out".into())
}

fn open_folder(path: &Path) -> bool {
    #[cfg(windows)]
    {
        Command::new("explorer").arg(path).spawn().is_ok()
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open").arg(path).spawn().is_ok()
            || Command::new("explorer.exe").arg(path).spawn().is_ok()
    }
}

fn save_debug_shot(image: &ColorImage, path: &Path) -> Result<(), String> {
    let w = image.width() as u32;
    let h = image.height() as u32;
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for p in &image.pixels {
        rgba.extend_from_slice(&p.to_array());
    }
    let buf = image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "invalid screenshot buffer".to_string())?;
    buf.save(path).map_err(|e| e.to_string())
}

fn load_texture(ctx: &egui::Context, bytes: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("part", color, TextureOptions::LINEAR))
}

/// 立创商城式局部放大：图上跟一块取景框，旁边用同一张图的 UV 裁切放大。
fn paint_photo_zoom(
    ui: &egui::Ui,
    tex: &TextureHandle,
    img_rect: egui::Rect,
    pointer: egui::Pos2,
    card: egui::Rect,
) {
    const LENS: f32 = 0.40;
    let size = img_rect.size();
    if size.x < 8.0 || size.y < 8.0 {
        return;
    }
    let uv = ((pointer - img_rect.min) / size).clamp(egui::Vec2::ZERO, egui::Vec2::splat(1.0));
    let half = LENS * 0.5;
    let cx = uv.x.clamp(half, 1.0 - half);
    let cy = uv.y.clamp(half, 1.0 - half);
    let uv_min = egui::pos2(cx - half, cy - half);
    let uv_max = egui::pos2(cx + half, cy + half);
    let lens = egui::Rect::from_min_max(
        img_rect.min + size * uv_min.to_vec2(),
        img_rect.min + size * uv_max.to_vec2(),
    );
    let lens_painter = ui.painter_at(img_rect);
    lens_painter.rect_filled(
        lens,
        2.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 70),
    );
    lens_painter.rect_stroke(
        lens,
        2.0,
        egui::Stroke::new(1.5_f32, ACCENT),
        egui::StrokeKind::Outside,
    );

    let zoom_w = (img_rect.width() * 1.75).clamp(220.0, 400.0);
    let zoom_h = zoom_w * (size.y / size.x).clamp(0.55, 1.45);
    let screen = ui.ctx().screen_rect().shrink(8.0);
    let mut origin = egui::pos2(card.max.x + 8.0, card.min.y);
    if origin.x + zoom_w > screen.max.x {
        origin.x = (card.min.x - 8.0 - zoom_w).max(screen.min.x);
    }
    if origin.y + zoom_h > screen.max.y {
        origin.y = (screen.max.y - zoom_h).max(screen.min.y);
    }

    egui::Area::new(egui::Id::new("photo_lens_zoom"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            let (panel, _) =
                ui.allocate_exact_size(egui::vec2(zoom_w, zoom_h), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(panel, 8.0, Color32::WHITE);
            p.rect_stroke(
                panel,
                8.0,
                egui::Stroke::new(1.0_f32, theme::hairline()),
                egui::StrokeKind::Inside,
            );
            let inner = panel.shrink(5.0);
            p.image(
                tex.id(),
                inner,
                egui::Rect::from_min_max(uv_min, uv_max),
                Color32::WHITE,
            );
        });
}

fn rasterize_mesh(
    mesh: &Mesh,
    rect: egui::Rect,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pixels_per_point: f32,
) -> ColorImage {
    let ppp = pixels_per_point.clamp(1.0, 3.0);
    let w = (rect.width() * ppp).round().max(1.0) as usize;
    let h = (rect.height() * ppp).round().max(1.0) as usize;
    let mut pixels = vec![0_u8; w * h * 4];
    for px in pixels.chunks_exact_mut(4) {
        px[0] = 236;
        px[1] = 236;
        px[2] = 241;
        px[3] = 255;
    }
    if mesh.vertices.is_empty() || mesh.triangles.is_empty() {
        return ColorImage::from_rgba_unmultiplied([w, h], &pixels);
    }

    let (min, max) = mesh.vertices.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(mut min, mut max), v| {
        for i in 0..3 {
            min[i] = min[i].min(v[i]);
            max[i] = max[i].max(v[i]);
        }
        (min, max)
    });
    let cx = (min[0] + max[0]) * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    let cz = (min[2] + max[2]) * 0.5;
    let span = (max[0] - min[0])
        .max(max[1] - min[1])
        .max(max[2] - min[2])
        .max(1e-3);
    let (sy, cyaw) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let ox = w as f32 * 0.5;
    let oy = h as f32 * 0.5;
    let scale = (h.min(w) as f32) * 0.72 * zoom;

    let to_cam = |v: [f32; 3]| {
        let x = (v[0] - cx) / span;
        let y = (v[1] - cy) / span;
        let z = (v[2] - cz) / span;
        let x1 = x * cyaw - y * sy;
        let y1 = x * sy + y * cyaw;
        let y2 = y1 * cp - z * sp;
        let z2 = y1 * sp + z * cp;
        [x1, y2, z2]
    };

    let mut depth = vec![f32::NEG_INFINITY; w * h];
    let wf = w as f32;
    let hf = h as f32;
    let wi = w as i32;
    let hi = h as i32;

    for (i, tri) in mesh.triangles.iter().enumerate() {
        let a = mesh.vertices.get(tri[0] as usize).copied().unwrap_or([0.0; 3]);
        let b = mesh.vertices.get(tri[1] as usize).copied().unwrap_or([0.0; 3]);
        let c = mesh.vertices.get(tri[2] as usize).copied().unwrap_or([0.0; 3]);
        let ca = to_cam(a);
        let cb = to_cam(b);
        let cc = to_cam(c);
        let e1 = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
        let e2 = [cc[0] - ca[0], cc[1] - ca[1], cc[2] - ca[2]];
        let nx = e1[1] * e2[2] - e1[2] * e2[1];
        let ny = e1[2] * e2[0] - e1[0] * e2[2];
        let nz = e1[0] * e2[1] - e1[1] * e2[0];
        if nz <= 0.0 {
            continue;
        }
        let nl = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
        let light = ((nx * 0.35 + ny * 0.75 + nz * 0.55) / nl).clamp(0.0, 1.0);
        let shade = (0.32 + 0.68 * light).clamp(0.28, 1.0);
        let [cr, cg, cb_col] = mesh.tri_rgb.get(i).copied().unwrap_or([196, 196, 200]);
        let r = (cr as f32 * shade) as u8;
        let g = (cg as f32 * shade) as u8;
        let bcol = (cb_col as f32 * shade) as u8;

        let ax = ox + ca[0] * scale;
        let ay = oy - ca[1] * scale;
        let bx = ox + cb[0] * scale;
        let by = oy - cb[1] * scale;
        let cxp = ox + cc[0] * scale;
        let cy = oy - cc[1] * scale;
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cxp - ax);
        if area.abs() < 1e-4 {
            continue;
        }
        let min_x = ax.min(bx).min(cxp).floor().max(0.0) as i32;
        let max_x = ax.max(bx).max(cxp).ceil().min(wf - 1.0) as i32;
        let min_y = ay.min(by).min(cy).floor().max(0.0) as i32;
        let max_y = ay.max(by).max(cy).ceil().min(hf - 1.0) as i32;
        if min_x > max_x || min_y > max_y || min_x >= wi || min_y >= hi {
            continue;
        }
        let za = ca[2];
        let zb = cb[2];
        let zc = cc[2];
        let inv_area = 1.0 / area;
        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let w0 = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
                let w1 = (cxp - bx) * (py - by) - (cy - by) * (px - bx);
                let w2 = (ax - cxp) * (py - cy) - (ay - cy) * (px - cxp);
                let inside = if area > 0.0 {
                    w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
                } else {
                    w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
                };
                if !inside {
                    continue;
                }
                let z = (w0 * zc + w1 * za + w2 * zb) * inv_area;
                let idx = (y as usize) * w + x as usize;
                if z >= depth[idx] {
                    depth[idx] = z;
                    let o = idx * 4;
                    pixels[o] = r;
                    pixels[o + 1] = g;
                    pixels[o + 2] = bcol;
                    pixels[o + 3] = 255;
                }
            }
        }
    }
    ColorImage::from_rgba_unmultiplied([w, h], &pixels)
}
