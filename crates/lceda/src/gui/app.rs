use super::preview3d::{self, GpuPreview};
use super::preview2d::{SymbolView, FootprintView, scroll_zoom};
use lceda_core::ir::{SymbolIr, FootprintIr};
use super::theme::{self, ACCENT};
use crate::i18n::{self, Lang};
use crate::bom::BomDocument;
use crate::instance::InstanceGuard;
use crate::library::{part_from_item, Library, LibraryFormat, SavedPart, UNCAT};
use crate::prefs::{ExportSection, SettingsTab, ThemeMode, PreviewMode};
use lceda_core::desc::{DescField, PropGroup, PropRow};
use crate::sponsor;
use crate::update::{self, CheckResult, UpdateInfo, UpdatePhase, UpdateProgress};
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use eframe::egui_glow::glow;
use lceda_core::client::LcedaClient;
use lceda_core::altium::{PIN_FONT_SIZE_MAX, PIN_FONT_SIZE_MIN, SchColorScheme, SchColors};
use lceda_core::export::{ExportRequest, export};
use lceda_core::mesh::{self, Mesh};
use lceda_core::models::{SearchItem, DownloadPaths};
use poll_promise::Promise;
use std::collections::{BTreeSet, HashMap};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use lceda_core::native_library::{self,NativeLibrary};
use lceda_core::altium::sch_preview::SchPreview;
#[path = "native_import.rs"]
mod native_import;
use native_import::{NativeImportState,PreparedImport};
#[path = "resource_assist.rs"]
mod resource_assist;
use resource_assist::ResourceAssist;
#[path = "library_sync.rs"]
mod library_sync;
use library_sync::{LibrarySyncTask, BindPromise};

type SearchPromise = Promise<lceda_core::Result<Vec<SearchItem>>>;
struct JobReport {
    message: String,
    exported: Vec<(SearchItem, DownloadPaths)>,
    notify: bool,
    failed: usize,
}
type ExportPromise = Promise<lceda_core::Result<JobReport>>;
type BomPromise = Promise<Result<BomDocument, String>>;
type PreviewPromise = Promise<PreviewData>;
type UpdatePromise = Promise<CheckResult>;
type ApplyPromise = Promise<Result<(), String>>;
type SponsorPromise = Promise<Result<sponsor::Pack, String>>;
type SponsorQrPromise = Promise<Result<(String, Vec<u8>), String>>;
type ResolvePromise = Promise<Result<(String, SearchItem), String>>;

const DEFAULT_DESC_PREVIEW_PART: &str = "C25804"; // Description example: 10kΩ, 0603, 1%
const GAP: f32 = 10.0;
const CARD_PAD: f32 = 12.0;
const NAV_W: f32 = 156.0;
const NAV_COLLAPSED: f32 = 56.0;
const PARTS_MIN: f32 = 200.0;
const PARTS_MAX: f32 = 480.0;
const RIGHT_MIN: f32 = 500.0;
const WIN_CORNER: u8 = 12;
const RESIZE_EDGE: f32 = 6.0;
const RESIZE_CORNER: f32 = 12.0;

#[derive(Clone, Copy)]
enum PartAct {
    None,
    Export,
    Queue,
    Fav,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavPage {
    Search,
    Favorites,
    Queue,
    Sponsor,
    Settings,
    About,
}

impl NavPage {
    #[allow(dead_code)]
    fn title(self, lang: Lang) -> &'static str {
        match self {
            Self::Search => i18n::t(lang, "search"),
            Self::Favorites => i18n::t(lang, "favorites"),
            Self::Queue => i18n::t(lang, "queue"),
            Self::Sponsor => i18n::t(lang, "sponsor"),
            Self::Settings => i18n::t(lang, "settings"),
            Self::About => i18n::t(lang, "about"),
        }
    }
}

const ICON_PNG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.png"));
const PAY_WECHAT: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pay/wechat.png"));
const PAY_ALIPAY: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pay/alipay.png"));
const PAY_AFDIAN: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pay/afdian.png"));
const FLAG_CN: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/region/cn.png"));
const FLAG_WORLD: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/region/world.png"));
const SIDEBAR_ICON_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/sidebar.png"));
const NAV_SEARCH_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/search.png"));
const NAV_FAV_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/star-full.png"));
const NAV_QUEUE_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/list-flat.png"));
const NAV_SPONSOR_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/heart.png"));
const NAV_SETTINGS_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/settings-gear.png"));
const NAV_ABOUT_PNG: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/info.png"));

struct PayTex {
    wechat: TextureHandle,
    alipay: TextureHandle,
    afdian: TextureHandle,
    cn: TextureHandle,
    world: TextureHandle,
}

#[derive(Clone)]
struct NavTex {
    sidebar: TextureHandle,
    search: TextureHandle,
    fav: TextureHandle,
    queue: TextureHandle,
    sponsor: TextureHandle,
    settings: TextureHandle,
    about: TextureHandle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
            ..Default::default()
        }
    }
}

struct PreviewData {
    native_scene: Option<SchPreview>,
    lcsc: Option<String>,
    files: Vec<(String, PathBuf)>,
    image: Option<Vec<u8>>,
    mesh: Option<Mesh>,
    mesh_note: Option<String>,
    symbol: Option<SymbolIr>,
    symbol_note: Option<String>,
    footprint: Option<FootprintIr>,
    footprint_note: Option<String>,
}

pub fn run(lang: Lang, instance: InstanceGuard) -> anyhow::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG).ok();
    let prefs = crate::prefs::load();
    let shooting = env::var("LCEDA_SHOT").is_ok();
    let (win_w, win_h) = if shooting {
        (env_f32("LCEDA_SHOT_W").unwrap_or(1180.0), env_f32("LCEDA_SHOT_H").unwrap_or(760.0))
    } else {
        (prefs.win_w, prefs.win_h)
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(i18n::t(lang, "app_title"))
        .with_inner_size([win_w, win_h])
        .with_min_inner_size([960.0, 620.0])
        .with_transparent(true)
        .with_decorations(false);
    if !shooting && prefs.always_on_top {
        viewport = viewport.with_always_on_top();
    }
    if !shooting && prefs.win_max {
        viewport = viewport.with_maximized(true);
    }
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: shooting || !prefs.win_max,
        ..Default::default()
    };
    eframe::run_native(
        "lceda",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(App::new(lang, cc.gl.clone(), instance)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

struct App {
    resource_assist: Option<ResourceAssist>,
    native_import: Option<NativeImportState>,
    native_load: Option<Promise<lceda_core::Result<NativeLibrary>>>,
    native_save: Option<Promise<lceda_core::Result<PreparedImport>>>,
    native_selected: Option<String>,
    native_scene: Option<SchPreview>,
    native_unit: usize,
    native_export: Option<Vec<String>>,
    native_export_resources: bool,
    library_filter: String,
    lang: Lang,
    lang_pinned: bool,
    show_bom_import: bool,
    bind_cat: Option<String>,
    bind_format: LibraryFormat,
    bind_path: String,
    pending_library_sync: BTreeSet<(String, LibraryFormat)>,
    library_sync_after: Option<Instant>,
    library_sync_task: Option<LibrarySyncTask>,
    library_sync_job: bool,
    show_library_sync: bool,
    bind_job: Option<BindPromise>,
    symbol: Option<SymbolIr>,
    symbol_note: Option<String>,
    symbol_view: SymbolView,
    preview_mode: PreviewMode,
    footprint: Option<FootprintIr>,
    footprint_note: Option<String>,
    footprint_view: FootprintView,
    bom: Option<BomDocument>,
    bom_job: Option<BomPromise>,
    bom_cancel: Arc<AtomicBool>,
    bom_progress: Arc<Mutex<(usize, usize)>>,
    export_items_pending: Option<Vec<SearchItem>>,
    show_activity: bool,
    confirm_clear_queue: bool,
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
    page: NavPage,
    settings_tab: SettingsTab,
    export_section: ExportSection,
    theme: ThemeMode,
    always_on_top: bool,
    dark_applied: Option<bool>,
    instance: InstanceGuard,
    nav_expanded: bool,
    nav_anim_w: f32,
    brand_tex: Option<TextureHandle>,
    nav_tex: Option<NavTex>,
    pay_tex: Option<PayTex>,
    sponsor_click_guard: bool,
    lib: Library,
    detail: Option<SearchItem>,
    cache: HashMap<String, SearchItem>,
    resolve: Option<ResolvePromise>,
    fav_cat: String,
    fav_inside: bool,
    fav_pick: bool,
    show_new_cat: bool,
    show_rename_cat: bool,
    show_import_cat: bool,
    new_cat_name: String,
    sponsor_job: Option<SponsorPromise>,
    sponsor_pack: Option<sponsor::Pack>,
    sponsor_note: Option<String>,
    sponsor_qr_job: Option<SponsorQrPromise>,
    sponsor_qr: Option<(String, TextureHandle)>,
    sponsor_qr_title: String,
    sponsor_popup: bool,
    show_welcome: bool,
    welcome_hide: bool,
    hide_welcome: bool,
    show_batch: bool,
    export_ids_pending: Option<Vec<String>>,
    parts_width: f32,
    photo_frac: Option<f32>,
    top_frac: Option<f32>,
    win_w: f32,
    win_h: f32,
    win_max: bool,
    win_round_sig: Option<(i32, i32, bool)>,
    batch_opts: BatchOpts,
    about_note: Option<String>,
    gpu: Option<Arc<egui::mutex::Mutex<GpuPreview>>>,
    update_check: Option<UpdatePromise>,
    update_manual: bool,
    update: Option<UpdateInfo>,
    update_job: Option<ApplyPromise>,
    update_progress: Option<update::ProgressHandle>,
    md_cache: CommonMarkCache,
    frame: u32,
    pending_search: Option<String>,
    shot_path: Option<PathBuf>,
    shot_requested: bool,
    shot_settle: u32,
    shot_input: Option<PathBuf>,
    shot_input_id: u64,
    shot_pointer_release: Option<egui::Pos2>,
    ad_embed_3d: bool,
    kicad_attach_3d: bool,
    rename_footprint: bool,
    batch_merge: bool,
    sch_scheme: SchColorScheme,
    sch_custom: SchColors,
    desc_fields: Vec<DescField>,
    show_detail: bool,
    detail_query: String,
    desc_preview_kw: String,
    desc_preview_item: Option<SearchItem>,
    desc_preview_job: Option<SearchPromise>,
    desc_preview_note: Option<String>,
}

impl App {
    fn new(lang: Lang, gl: Option<Arc<glow::Context>>, instance: InstanceGuard) -> Self {
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
        let out_dir = prefs.out_dir.clone().unwrap_or(out_dir);
        let mut app = Self {
            resource_assist:None,native_import:None,native_load:None,native_save:None,native_selected:None,native_scene:None,native_unit:1,native_export:None,native_export_resources:true,library_filter:String::new(),
            lang,
            lang_pinned: prefs.lang.is_some(),
            show_bom_import: false,
            bind_cat: None,
            bind_format: LibraryFormat::Ad,
            bind_path: String::new(),
            pending_library_sync: BTreeSet::new(),
            library_sync_after: None,
            library_sync_task: None,
            library_sync_job: false,
            show_library_sync: false,
            bind_job: None,
            symbol: None,
            symbol_note: None,
            symbol_view: SymbolView::default(),
            preview_mode: env::var("LCEDA_PREVIEW").ok().map(|v| PreviewMode::parse(&v)).unwrap_or(prefs.preview_mode),
            footprint: None,
            footprint_note: None,
            footprint_view: FootprintView::default(),
            bom: None,
            bom_job: None,
            bom_cancel: Arc::new(AtomicBool::new(false)),
            bom_progress: Arc::new(Mutex::new((0, 0))),
            export_items_pending: None,
            show_activity: false,
            confirm_clear_queue: false,
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
            yaw: env_f32("LCEDA_YAW").unwrap_or(0.7),
            pitch: env_f32("LCEDA_PITCH").unwrap_or(0.55),
            zoom: env_f32("LCEDA_ZOOM").unwrap_or(1.0),
            alert: None,
            page: match env::var("LCEDA_PAGE").as_deref() {
                Ok("settings") => NavPage::Settings,
                Ok("about") => NavPage::About,
                Ok("favorites") => NavPage::Favorites,
                Ok("queue") => NavPage::Queue,
                Ok("bom") => NavPage::Favorites,
                Ok("sponsor") => NavPage::Sponsor,
                _ if !prefs.hide_welcome && !skip_update => NavPage::Settings,
                _ => NavPage::Search,
            },
            settings_tab: match env::var("LCEDA_SETTINGS_TAB").as_deref() {
                Ok("export") => SettingsTab::Export,
                Ok("appearance") => SettingsTab::Appearance,
                Ok("general") => SettingsTab::General,
                _ => prefs.settings_tab,
            },
            export_section: match env::var("LCEDA_EXPORT_SEC").as_deref() {
                Ok("description") => ExportSection::Description,
                Ok("schematic") => ExportSection::Schematic,
                Ok("format") => ExportSection::Format,
                _ => prefs.export_section,
            },
            theme: env::var("LCEDA_THEME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| ThemeMode::parse(&s))
                .unwrap_or(prefs.theme),
            always_on_top: prefs.always_on_top,
            dark_applied: None,
            instance,
            nav_expanded: prefs.nav_expanded,
            nav_anim_w: if prefs.nav_expanded { NAV_W } else { NAV_COLLAPSED },
            brand_tex: None,
            nav_tex: None,
            pay_tex: None,
            sponsor_click_guard: false,
            lib: Library::load(),
            detail: None,
            cache: HashMap::new(),
            resolve: None,
            fav_cat: UNCAT.into(),
            fav_inside: false,
            fav_pick: false,
            show_new_cat: false,
            show_rename_cat: false,
            show_import_cat: false,
            new_cat_name: String::new(),
            sponsor_job: None,
            sponsor_pack: None,
            sponsor_note: None,
            sponsor_qr_job: None,
            sponsor_qr: None,
            sponsor_qr_title: String::new(),
            sponsor_popup: false,
            show_welcome: !prefs.hide_welcome && !skip_update,
            welcome_hide: true,
            hide_welcome: prefs.hide_welcome,
            show_batch: false,
            export_ids_pending: None,
            parts_width: prefs.parts_width,
            photo_frac: prefs.photo_frac,
            top_frac: prefs.top_frac,
            win_w: prefs.win_w,
            win_h: prefs.win_h,
            win_max: prefs.win_max,
            win_round_sig: None,
            batch_opts: BatchOpts {
                step: prefs.export_step,
                obj: prefs.export_obj,
                ad: prefs.export_ad,
                kicad: prefs.export_kicad,
                pads: prefs.export_pads,
                datasheet: prefs.export_datasheet,
                source: prefs.export_source,
            },
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
            md_cache: CommonMarkCache::default(),
            frame: 0,
            pending_search: if skip_update {
                env::var("LCEDA_SEARCH").ok().filter(|s| !s.trim().is_empty())
            } else { None },
            shot_path: env::var("LCEDA_SHOT").ok().filter(|s| !s.is_empty()).map(PathBuf::from),
            shot_requested: false,
            shot_settle: 0,
            shot_input: if skip_update { env::var_os("LCEDA_SHOT_INPUT").map(PathBuf::from) } else { None },
            shot_input_id: 0,
            shot_pointer_release: None,
            ad_embed_3d: prefs.ad_embed_3d,
            kicad_attach_3d: prefs.kicad_attach_3d,
            rename_footprint: prefs.rename_footprint,
            batch_merge: prefs.batch_merge,
            sch_scheme: prefs.sch_scheme,
            sch_custom: prefs.sch_custom,
            desc_fields: prefs.desc_fields.clone(),
            show_detail: false,
            detail_query: String::new(),
            desc_preview_kw: env::var("LCEDA_DESC_PREVIEW").unwrap_or_else(|_| DEFAULT_DESC_PREVIEW_PART.into()),
            desc_preview_item: None,
            desc_preview_job: None,
            desc_preview_note: None,
        };
        if skip_update {
            if let Ok(path)=env::var("LCEDA_NATIVE_FILE"){
                let path=PathBuf::from(path);
                let format=if path.extension().is_some_and(|s|s.eq_ignore_ascii_case("SchLib")){LibraryFormat::Ad}else{LibraryFormat::KiCad};
                app.load_native_library(path,format);
            }
            if let Ok(id)=env::var("LCEDA_NATIVE_SELECT"){app.select_saved(&id);}
            if let Ok(scope)=env::var("LCEDA_RESOURCE_ASSIST"){
                let ids=if scope=="all"{app.lib.favs.iter().filter(|p|app.lib.native(&p.lcsc).is_some()).map(|p|p.lcsc.clone()).collect()}else{app.native_selected.iter().cloned().collect()};
                let choice=env::var("LCEDA_RESOURCE_KINDS").unwrap_or_default();
                app.open_resource_assist(ids,lceda_core::supplement::ResourceKinds{footprint:choice=="footprint"||choice=="both",model3d:choice=="3d"||choice=="both"});
            }
            if let Ok(path) = env::var("LCEDA_BOM_FILE") {
                if let Ok(mut doc) = BomDocument::load(Path::new(&path)) {
                    for row in &mut doc.rows {
                        if let Some(item) = app.lib.cached_item(&row.lcsc) {
                            row.candidates = vec![item]; row.selected = Some(0);
                        }
                    }
                    app.bom = Some(doc);
                    app.page = NavPage::Favorites;
                    app.show_bom_import = true;
                }
            }
        }
        app
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
                        egui::Label::new(egui::RichText::new(&msg).color(theme::label()))
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
                if consume_dialog_confirm(ui, true) {
                    close = true;
                }
            });
        if close {
            self.alert = None;
        }
    }

    fn show_welcome(&mut self, ctx: &egui::Context) {
        if !self.show_welcome {
            return;
        }
        let mut close = false;
        let mut open_repo = false;
        let lang = self.lang;
        let width = 440.0;
        fit_window(i18n::t(lang, "notice"), "lceda_welcome_fit", width).show(ctx, |ui| {
            ui.set_width(width - 8.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add(
                egui::Label::new(egui::RichText::new(i18n::t(lang, "welcome_p1")).color(theme::label()))
                    .wrap(),
            );
            ui.add(
                egui::Label::new(egui::RichText::new(i18n::t(lang, "welcome_p2")).color(theme::label()))
                    .wrap(),
            );
            ui.add(
                egui::Label::new(egui::RichText::new(i18n::t(lang, "welcome_p3")).color(theme::label()))
                    .wrap(),
            );
            ui.add_space(4.0);
            ui.checkbox(&mut self.welcome_hide, i18n::t(lang, "welcome_hide"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::pill_button(ui, i18n::t(lang, "ok"), true, true).clicked() {
                        close = true;
                    }
                    if theme::pill_button(ui, i18n::t(lang, "open_repo"), true, false).clicked() {
                        open_repo = true;
                    }
                    if consume_dialog_confirm(ui, true) {
                        close = true;
                    }
                });
            });
        });
        if open_repo {
            let _ = webbrowser::open(update::REPO_URL);
        }
        if close {
            self.show_welcome = false;
            self.hide_welcome = self.welcome_hide;
            self.persist_prefs();
            self.goto(NavPage::Settings);
        }
    }

    fn show_modal_scrim(&self, ctx: &egui::Context) {
        if !self.dialog_open() {
            return;
        }
        let screen = ctx.screen_rect();
        // Middle covers panels; Foreground dialogs must stay above this scrim.
        egui::Area::new(egui::Id::new("modal_scrim"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(ctx, |ui| {
                ui.allocate_response(screen.size(), egui::Sense::click());
                let dim = if theme::is_dark() {
                    Color32::from_black_alpha(150)
                } else {
                    Color32::from_black_alpha(88)
                };
                ui.painter().rect_filled(screen, 0.0, dim);
            });
    }

    fn show_detail(&mut self, ctx: &egui::Context) {
        if self.native_selected.is_some() { self.native_details(ctx); return; }
        if !self.show_detail {
            return;
        }
        let Some(item) = self.selected_item().cloned() else {
            self.show_detail = false;
            return;
        };
        let lang = self.lang;
        let rows = item.ad_properties(&self.desc_fields, true);
        let ds = item.datasheet_url();
        let page = item.product_url();
        let title = format!("{}  {}", i18n::t(lang, "details"), item.name());
        let mut close = false;
        let mut open_ds = false;
        let mut open_page = false;
        let mut query = std::mem::take(&mut self.detail_query);
        let width = 580.0;
        let mut open = true;
        fit_window(title, "lceda_detail_fit", width)
            .open(&mut open)
            .min_width(540.0)
            .max_width(680.0)
            .max_height((ctx.screen_rect().height() - 64.0).max(360.0))
            .resizable(true)
            .show(ctx, |ui| {
                ui.set_min_width(width - 8.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(i18n::t(lang, "details_legend"))
                            .color(theme::secondary())
                            .size(12.0),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let search_w = (ui.available_width() - 8.0).max(160.0);
                    theme::search_field(
                        ui,
                        &mut query,
                        i18n::t(lang, "details_search"),
                        "detail_prop_search",
                        egui::vec2(search_w, 32.0),
                    );
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical().id_salt("detail_body").max_height((ctx.screen_rect().height() - 260.0).clamp(180.0, 410.0)).show(ui, |ui| {
                    show_ad_props(ui, lang, &rows, &query, None, "detail_props", true);
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::pill_button(ui, i18n::t(lang, "ok"), true, true).clicked() {
                            close = true;
                        }
                        if theme::pill_button(ui, i18n::t(lang, "details_online"), true, false)
                            .clicked()
                        {
                            open_page = true;
                        }
                        if ds.is_some()
                            && theme::pill_button(ui, i18n::t(lang, "datasheet"), true, false)
                                .clicked()
                        {
                            open_ds = true;
                        }
                    });
                });
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    if query.is_empty() {
                        close = true;
                    } else {
                        query.clear();
                    }
                }
            });
        if !open {
            close = true;
        }
        if open_page {
            let _ = webbrowser::open(&page);
        }
        if open_ds {
            if let Some(url) = ds {
                let _ = webbrowser::open(&url);
            }
        }
        if close {
            self.show_detail = false;
            self.detail_query.clear();
        } else {
            self.detail_query = query;
        }
    }

    fn goto(&mut self, page: NavPage) {
        self.page = page;
        if page == NavPage::Favorites {
            self.fav_inside = false;
        }
        if page == NavPage::Sponsor {
            self.ensure_sponsor();
        }
    }

    fn ensure_sponsor(&mut self) {
        if self.sponsor_pack.is_none() && self.sponsor_job.is_none() {
            self.sponsor_note = None;
            self.sponsor_job = Some(Promise::spawn_thread("sponsor", sponsor::load));
        }
    }

    fn open_channel_qr(&mut self, ch: &sponsor::Channel) {
        if let Some(rel) = ch.image.clone() {
            let title = sponsor::title(ch, self.lang == Lang::Zh).to_string();
            self.sponsor_note = None;
            self.sponsor_qr = None;
            self.sponsor_qr_title = title.clone();
            self.sponsor_popup = true;
            self.sponsor_click_guard = true;
            self.sponsor_qr_job = Some(Promise::spawn_thread("sponsor-qr", move || {
                sponsor::fetch_image(&rel)
                    .map(|bytes| (title, bytes))
                    .ok_or_else(|| "无法加载收款码".into())
            }));
        } else if let Some(url) = &ch.url {
            let _ = webbrowser::open(url);
        } else {
            self.sponsor_note = Some(i18n::t(self.lang, "sponsor_no_qr").into());
        }
    }

    fn want_dark(&self, ctx: &egui::Context) -> bool {
        match self.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::Auto => matches!(ctx.system_theme(), Some(egui::Theme::Dark)),
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let dark = self.want_dark(ctx);
        if self.dark_applied != Some(dark) {
            theme::apply_visuals(ctx, dark);
            self.dark_applied = Some(dark);
        }
    }

    fn page_about(&mut self, ui: &mut egui::Ui) {
        let mut open_repo = false;
        let mut open_issues = false;
        let mut open_license = false;
        let mut check = false;
        let checking = self.update_check.is_some();
        let can_check = self.update_job.is_none() && self.update.is_none();
        let check_label = if checking {
            i18n::t(self.lang, "checking_update")
        } else {
            i18n::t(self.lang, "check_update")
        };
        let lang = self.lang;
        egui::ScrollArea::vertical()
            .id_salt("about_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::show_card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "about_intro_title"))
                            .color(theme::label())
                            .size(16.0)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(i18n::t(lang, "about_intro")).color(theme::secondary()),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(10.0);
                theme::show_card(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(i18n::t(lang, "about_ver"))
                                .color(theme::label())
                                .strong(),
                        );
                        if ui
                            .link(egui::RichText::new(format!("({check_label})")).color(ACCENT))
                            .clicked()
                            && can_check
                        {
                            check = true;
                        }
                    });
                    ui.label(
                        egui::RichText::new(format!("v{}", update::current_version()))
                            .color(theme::secondary()),
                    );
                    if let Some(note) = &self.about_note {
                        ui.add(egui::Label::new(egui::RichText::new(note).color(theme::secondary())).wrap());
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}:", i18n::t(lang, "about_repo")))
                                .color(theme::label())
                                .strong(),
                        );
                        if ui.link(update::REPO).clicked() {
                            open_repo = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}:", i18n::t(lang, "about_author")))
                                .color(theme::label())
                                .strong(),
                        );
                        ui.label(egui::RichText::new("LZJ-I").color(theme::secondary()));
                    });
                    ui.separator();
                    if ui
                        .link(egui::RichText::new(i18n::t(lang, "about_issues")).color(theme::label()))
                        .clicked()
                    {
                        open_issues = true;
                    }
                });
                ui.add_space(10.0);
                theme::show_card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "about_love"))
                            .color(theme::label())
                            .size(15.0)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(i18n::t(lang, "about_star")).color(theme::secondary()),
                        )
                        .wrap(),
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(i18n::t(lang, "about_copyright"))
                                    .color(theme::secondary())
                                    .size(12.0),
                            )
                            .wrap(),
                        );
                        ui.add_space(4.0);
                        if ui
                            .link(
                                egui::RichText::new(i18n::t(lang, "about_license_name"))
                                    .color(ACCENT)
                                    .size(12.0),
                            )
                            .clicked()
                        {
                            open_license = true;
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
        if open_issues {
            let _ = webbrowser::open(&format!("{}/issues", update::REPO_URL));
        }
        if open_license {
            let _ = webbrowser::open("https://creativecommons.org/licenses/by-nc/4.0/");
        }
    }

    fn page_settings(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let mut persist = false;
        let mut theme_changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (tab, key) in [
                (SettingsTab::General, "settings_tab_general"),
                (SettingsTab::Appearance, "settings_tab_appearance"),
                (SettingsTab::Export, "settings_tab_export"),
            ] {
                if ui
                    .selectable_label(self.settings_tab == tab, i18n::t(lang, key))
                    .clicked()
                {
                    self.settings_tab = tab;
                    persist = true;
                }
            }
        });
        ui.add_space(8.0);
        ui.add(egui::Separator::default().spacing(10.0));
        egui::ScrollArea::vertical()
            .id_salt("settings_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match self.settings_tab {
                    SettingsTab::General => {
                        persist |= self.settings_general(ui);
                    }
                    SettingsTab::Appearance => {
                        persist |= self.settings_appearance(ui, &mut theme_changed);
                    }
                    SettingsTab::Export => {
                        persist |= self.settings_export(ui);
                    }
                }
            });
        if theme_changed {
            self.dark_applied = None;
        }
        if persist {
            self.persist_prefs();
        }
    }

    fn settings_general(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut persist = false;
        let mut browse = false;
        let mut open_dir = false;
        theme::show_card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t(lang, "lang_label"))
                    .strong()
                    .color(theme::label()),
            );
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(!self.lang_pinned, i18n::t(lang, "lang_follow"))
                    .clicked()
                {
                    self.lang_pinned = false;
                    self.lang = Lang::detect();
                    persist = true;
                }
                if ui
                    .selectable_label(self.lang_pinned && self.lang == Lang::Zh, "中文")
                    .clicked()
                {
                    self.lang_pinned = true;
                    self.lang = Lang::Zh;
                    persist = true;
                }
                if ui
                    .selectable_label(self.lang_pinned && self.lang == Lang::En, "English")
                    .clicked()
                {
                    self.lang_pinned = true;
                    self.lang = Lang::En;
                    persist = true;
                }
            });
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(i18n::t(lang, "output"))
                    .strong()
                    .color(theme::label()),
            );
            let path_w = ui.available_width();
            let path = ui.add_sized(
                egui::vec2(path_w, 28.0),
                theme::edit_single(&mut self.out_dir, i18n::t(lang, "output_hint"))
                    .desired_width(path_w),
            );
            if path.lost_focus() {
                persist = true;
            }
            ui.horizontal(|ui| {
                if theme::pill_button(ui, i18n::t(lang, "browse"), true, false).clicked() {
                    browse = true;
                }
                if theme::pill_button(ui, i18n::t(lang, "open_folder"), true, true).clicked() {
                    open_dir = true;
                }
            });
        });
        if browse {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                self.out_dir = dir.display().to_string();
                persist = true;
            }
        }
        if open_dir {
            self.open_out_dir();
        }
        persist
    }

    fn settings_appearance(&mut self, ui: &mut egui::Ui, theme_changed: &mut bool) -> bool {
        let lang = self.lang;
        let mut persist = false;
        theme::show_card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t(lang, "theme_label"))
                    .strong()
                    .color(theme::label()),
            );
            ui.horizontal(|ui| {
                for (mode, key) in [
                    (ThemeMode::Auto, "theme_auto"),
                    (ThemeMode::Light, "theme_light"),
                    (ThemeMode::Dark, "theme_dark"),
                ] {
                    if ui
                        .selectable_label(self.theme == mode, i18n::t(lang, key))
                        .clicked()
                    {
                        self.theme = mode;
                        *theme_changed = true;
                        persist = true;
                    }
                }
            });
        });
        persist
    }

    fn settings_export(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut persist = false;
        let previous = (self.ad_embed_3d, self.kicad_attach_3d, self.rename_footprint, self.resolved_sch_colors(), self.desc_fields.clone());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (sec, key) in [
                (ExportSection::Format, "export_sec_format"),
                (ExportSection::Schematic, "export_sec_sch"),
                (ExportSection::Description, "export_sec_desc"),
            ] {
                if ui
                    .selectable_label(self.export_section == sec, i18n::t(lang, key))
                    .clicked()
                {
                    self.export_section = sec;
                    persist = true;
                }
            }
        });
        ui.add_space(8.0);
        match self.export_section {
            ExportSection::Format => persist |= self.settings_export_format(ui),
            ExportSection::Schematic => persist |= self.sch_color_card(ui),
            ExportSection::Description => persist |= self.settings_desc_card(ui),
        }
        let current = (self.ad_embed_3d, self.kicad_attach_3d, self.rename_footprint, self.resolved_sch_colors(), self.desc_fields.clone());
        if current != previous {
            let ids: Vec<_> = self.lib.bound_categories().into_iter().map(|(category, _)| category.id.clone()).collect();
            for id in ids { self.queue_library_sync(&id, None); }
            self.library_sync_after = Some(Instant::now() + Duration::from_millis(700));
        }
        persist
    }

    fn settings_export_format(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut persist = false;
        theme::show_card(ui, |ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                egui::RichText::new(i18n::t(lang, "defaults_label"))
                    .strong()
                    .color(theme::label()),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(i18n::t(lang, "defaults_hint"))
                        .color(theme::secondary())
                        .small(),
                )
                .wrap(),
            );
            let before = self.batch_opts;
            ui.columns(2, |cols| {
                cols[0].checkbox(&mut self.batch_opts.step, i18n::t(lang, "download_step"));
                cols[1].checkbox(&mut self.batch_opts.obj, i18n::t(lang, "download_obj"));
                cols[0].checkbox(&mut self.batch_opts.ad, i18n::t(lang, "export_ad"));
                cols[1].checkbox(&mut self.batch_opts.kicad, i18n::t(lang, "export_kicad"));
                cols[0].checkbox(&mut self.batch_opts.pads, i18n::t(lang, "export_pads"));
                cols[1].checkbox(&mut self.batch_opts.datasheet, i18n::t(lang, "datasheet"));
                cols[0].checkbox(&mut self.batch_opts.source, i18n::t(lang, "export_source"));
            });
            if before != self.batch_opts {
                persist = true;
            }
            ui.add_space(10.0);
            let before_ad = self.ad_embed_3d;
            let before_kicad = self.kicad_attach_3d;
            let before_fp = self.rename_footprint;
            ui.columns(2, |cols| {
                cols[0].checkbox(&mut self.ad_embed_3d, i18n::t(lang, "ad_embed_3d"));
                cols[1].checkbox(&mut self.kicad_attach_3d, i18n::t(lang, "kicad_attach_3d"));
                cols[0].checkbox(&mut self.rename_footprint, i18n::t(lang, "rename_footprint"));
            });
            if self.ad_embed_3d != before_ad
                || self.kicad_attach_3d != before_kicad
                || self.rename_footprint != before_fp
            {
                persist = true;
            }
        });
        persist
    }

    fn settings_desc_card(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut persist = false;
        let mut fields = self.desc_fields.clone();
        if self.desc_preview_item.is_none() && self.desc_preview_job.is_none() && self.desc_preview_note.is_none() {
            self.start_desc_preview();
        }
        theme::show_card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t(lang, "export_sec_desc"))
                    .strong()
                    .color(theme::label()),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(i18n::t(lang, "desc_hint"))
                        .color(theme::secondary())
                        .small(),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            let width = ui.available_width();
            let left_width = (width * 0.34).clamp(240.0, 290.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                ui.allocate_ui_with_layout(egui::vec2(left_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(left_width);
                    ui.spacing_mut().interact_size.y = 26.0;
                    ui.spacing_mut().button_padding.y = 4.0;
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "desc_fields_title"))
                            .strong()
                            .color(theme::label()),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(i18n::t(lang, "desc_fields_hint"))
                                .color(theme::secondary())
                                .small(),
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        for (preset, key) in [
                            (DescField::library(), "desc_preset_library"),
                            (DescField::specs_only(), "desc_preset_specs"),
                            (DescField::ids_only(), "desc_preset_ids"),
                        ] {
                            if ui
                                .selectable_label(fields == preset, i18n::t(lang, key))
                                .clicked()
                            {
                                fields = preset;
                                persist = true;
                            }
                        }
                    });
                    ui.add_space(8.0);
                    let enabled = fields.clone();
                    let disabled: Vec<DescField> = DescField::ALL
                        .into_iter()
                        .filter(|f| !enabled.contains(f))
                        .collect();
                    let mut drag_from = None;
                    let mut drag_to = None;
                    for (i, field) in enabled.iter().enumerate() {
                        let drop_frame = egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(2, 1))
                            .corner_radius(6);
                        let (_, dropped) = ui.dnd_drop_zone::<usize, _>(drop_frame, |ui| {
                            ui.horizontal(|ui| {
                                ui.dnd_drag_source(egui::Id::new(("desc_grip", field.id())), i, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new("☰")
                                                .color(theme::secondary())
                                                .size(14.0),
                                        )
                                        .sense(egui::Sense::drag()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::Grab)
                                    .on_hover_text(i18n::t(lang, "desc_drag"));
                                });
                                let mut on = true;
                                if ui
                                    .checkbox(&mut on, i18n::t(lang, field.i18n_key()))
                                    .changed()
                                    && !on
                                {
                                    fields.retain(|f| f != field);
                                    persist = true;
                                }
                            });
                        });
                        if let Some(from) = dropped {
                            drag_from = Some(*from);
                            drag_to = Some(i);
                        }
                    }
                    if let (Some(from), Some(to)) = (drag_from, drag_to) {
                        if from != to && from < fields.len() && to < fields.len() {
                            if from < to {
                                fields[from..=to].rotate_left(1);
                            } else {
                                fields[to..=from].rotate_right(1);
                            }
                            persist = true;
                        }
                    }
                    for field in disabled {
                        ui.horizontal(|ui| {
                            let mut on = false;
                            if ui
                                .checkbox(&mut on, i18n::t(lang, field.i18n_key()))
                                .changed()
                                && on
                            {
                                fields.push(field);
                                persist = true;
                            }
                        });
                    }
                });
                let right_width = ui.available_width().max(240.0);
                ui.allocate_ui_with_layout(egui::vec2(right_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(right_width);
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "desc_preview_lookup"))
                            .strong()
                            .color(theme::label()),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(i18n::t(lang, "desc_preview_hint"))
                                .color(theme::secondary())
                                .small(),
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    let mut lookup = false;
                    ui.horizontal(|ui| {
                        let btn_w = 64.0;
                        let edit_w = (ui.available_width() - btn_w - 8.0).max(120.0);
                        let r = theme::search_field(
                            ui,
                            &mut self.desc_preview_kw,
                            i18n::t(lang, "desc_preview_lookup_hint"),
                            "desc_preview_search",
                            egui::vec2(edit_w, 32.0),
                        );
                        if (r.has_focus() || r.lost_focus())
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !self.dialog_open()
                        {
                            lookup = true;
                        }
                        if theme::pill_button(
                            ui,
                            i18n::t(lang, "desc_preview_view"),
                            self.desc_preview_job.is_none(),
                            true,
                        )
                        .clicked()
                        {
                            lookup = true;
                        }
                    });
                    ui.add_space(10.0);
                    if self.desc_preview_job.is_some() {
                        ui.label(
                            egui::RichText::new(i18n::t(lang, "desc_preview_loading"))
                                .color(theme::secondary()),
                        );
                    } else if let Some(note) = &self.desc_preview_note {
                        ui.label(egui::RichText::new(note).color(theme::secondary()));
                    } else if let Some(item) = self.desc_preview_item.clone() {
                        let title = match item.lcsc_id() {
                            Some(id) => format!("{} · {id}", item.name()),
                            None => item.name().to_string(),
                        };
                        ui.label(
                            egui::RichText::new(title)
                                .color(theme::label())
                                .size(12.5),
                        );
                        ui.add_space(8.0);
                        let preview_rows = item.ad_properties(&fields, false);
                        let height = (ui.ctx().screen_rect().bottom() - ui.next_widget_position().y - 64.0).max(180.0);
                        show_ad_props(ui, lang, &preview_rows, "", Some(height), "desc_preview_props", false);
                    } else {
                        ui.label(
                            egui::RichText::new(i18n::t(lang, "desc_preview_idle"))
                                .color(theme::secondary()),
                        );
                    }
                    if self.desc_preview_item.is_none() {
                        ui.add_space(14.0);
                        show_ad_field_map(ui, lang);
                    }
                    if lookup {
                        self.start_desc_preview();
                    }
                });
            });
        });
        if persist {
            if fields.is_empty() {
                fields = DescField::library();
            }
            self.desc_fields = fields;
        }
        persist
    }

    fn start_desc_preview(&mut self) {
        let kw = self.desc_preview_kw.trim().to_string();
        if kw.is_empty() {
            self.desc_preview_note = Some(i18n::t(self.lang, "empty_keyword").into());
            return;
        }
        if self.desc_preview_job.is_some() {
            return;
        }
        if let Some(item) = self.cache.get(&kw).cloned() {
            self.desc_preview_item = Some(item);
            self.desc_preview_note = None;
            return;
        }
        self.desc_preview_note = None;
        self.desc_preview_job = Some(Promise::spawn_thread("desc-preview", move || {
            LcedaClient::new().search(&kw)
        }));
    }

    fn sch_color_card(&mut self, ui: &mut egui::Ui) -> bool {
        let lang = self.lang;
        let mut persist = false;
        theme::show_card(ui, |ui| {
            ui.label(
                egui::RichText::new(i18n::t(lang, "sch_color_label"))
                    .strong()
                    .color(theme::label()),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(i18n::t(lang, "sch_color_hint"))
                        .color(theme::secondary())
                        .small(),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for (scheme, key) in [
                    (SchColorScheme::AltiumClassic, "sch_color_altium"),
                    (SchColorScheme::EasyEda, "sch_color_easyeda"),
                    (SchColorScheme::Mono, "sch_color_mono"),
                    (SchColorScheme::Custom, "sch_color_custom"),
                ] {
                    if ui
                        .selectable_label(self.sch_scheme == scheme, i18n::t(lang, key))
                        .clicked()
                    {
                        if scheme == SchColorScheme::Custom
                            && self.sch_scheme != SchColorScheme::Custom
                        {
                            self.sch_custom = self.sch_scheme.colors(self.sch_custom);
                        }
                        self.sch_scheme = scheme;
                        persist = true;
                    }
                }
            });
            if self.sch_scheme == SchColorScheme::Custom {
                ui.add_space(8.0);
                let mut custom = self.sch_custom;
                let mut dirty = false;
                ui.columns(2, |cols| {
                    dirty |= sch_color_row(
                        &mut cols[0],
                        i18n::t(lang, "sch_color_background"),
                        &mut custom.background,
                    );
                    dirty |= sch_color_row(&mut cols[1], i18n::t(lang, "sch_color_body"), &mut custom.body);
                    dirty |= sch_color_row(&mut cols[0], i18n::t(lang, "sch_color_pin"), &mut custom.pin);
                    dirty |= sch_color_row(
                        &mut cols[1],
                        i18n::t(lang, "sch_color_pin_name"),
                        &mut custom.pin_name,
                    );
                    dirty |= sch_color_row(
                        &mut cols[0],
                        i18n::t(lang, "sch_color_pin_number"),
                        &mut custom.pin_number,
                    );
                    dirty |= sch_color_row(
                        &mut cols[1],
                        i18n::t(lang, "sch_color_designator"),
                        &mut custom.designator,
                    );
                    dirty |= sch_color_row(
                        &mut cols[0],
                        i18n::t(lang, "sch_color_comment"),
                        &mut custom.comment,
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "sch_color_pin_size"))
                            .color(theme::label())
                            .size(12.0),
                    );
                    let mut size = custom.clamped_pin_font_size();
                    let resp = ui.add(
                        egui::DragValue::new(&mut size)
                            .range(PIN_FONT_SIZE_MIN..=PIN_FONT_SIZE_MAX)
                            .suffix(" pt")
                            .speed(0.2),
                    );
                    if resp.changed() {
                        custom.pin_font_size = size;
                        dirty = true;
                    }
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "sch_color_pin_size_hint"))
                            .color(theme::secondary())
                            .size(11.0),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button(i18n::t(lang, "sch_color_apply_altium")).clicked() {
                        custom = SchColors::altium_classic();
                        dirty = true;
                    }
                    if ui.small_button(i18n::t(lang, "sch_color_apply_easyeda")).clicked() {
                        custom = SchColors::easyeda();
                        dirty = true;
                    }
                    if ui.small_button(i18n::t(lang, "sch_color_apply_mono")).clicked() {
                        custom = SchColors::mono();
                        dirty = true;
                    }
                });
                if dirty {
                    self.sch_custom = custom;
                    persist = true;
                }
            }
            ui.add_space(10.0);
            sch_color_preview(ui, self.resolved_sch_colors());
        });
        persist
    }

    fn page_sponsor(&mut self, ui: &mut egui::Ui) {
        let mut clicked: Option<usize> = None;
        let zh = self.lang == Lang::Zh;
        let lang = self.lang;
        egui::ScrollArea::vertical()
            .id_salt("sponsor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::show_card(ui, |ui| {
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "donate_title"))
                            .color(theme::label())
                            .size(16.0)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(12.0);
                    donate_para(
                        ui,
                        Color32::from_rgb(16, 185, 129),
                        i18n::t(lang, "donate_license"),
                    );
                    ui.add_space(12.0);
                    donate_para(
                        ui,
                        Color32::from_rgb(59, 130, 246),
                        i18n::t(lang, "donate_maintain"),
                    );
                    ui.add_space(12.0);
                    donate_para(
                        ui,
                        Color32::from_rgb(249, 115, 22),
                        i18n::t(lang, "donate_purpose"),
                    );
                    ui.add_space(12.0);
                    donate_para(
                        ui,
                        Color32::from_rgb(236, 72, 153),
                        i18n::t(lang, "donate_thanks"),
                    );
                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new(format!("{}:", i18n::t(lang, "sponsor_pay")))
                            .color(theme::label())
                            .strong(),
                    );
                    ui.add_space(8.0);
                    if let Some(err) = &self.sponsor_note {
                        if !self.sponsor_popup {
                            ui.label(egui::RichText::new(err).color(theme::secondary()).size(12.0));
                        }
                    } else if self.sponsor_job.is_some() {
                        ui.label(
                            egui::RichText::new(i18n::t(lang, "sponsor_loading")).color(theme::secondary()),
                        );
                    }
                    if let Some(pack) = &self.sponsor_pack {
                        if self.pay_tex.is_none() {
                            self.pay_tex = load_pay_tex(ui.ctx());
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            for (i, ch) in pack.channels.iter().enumerate() {
                                let hint = if ch.image.is_some() {
                                    i18n::t(lang, "donate_scan")
                                } else {
                                    i18n::t(lang, "donate_visit")
                                };
                                let Some(tex) = &self.pay_tex else {
                                    break;
                                };
                                let brand = match ch.id.as_str() {
                                    "alipay" => &tex.alipay,
                                    "afdian" => &tex.afdian,
                                    _ => &tex.wechat,
                                };
                                let flag = match ch.id.as_str() {
                                    "wechat" | "alipay" | "afdian" => &tex.cn,
                                    _ => &tex.world,
                                };
                                if theme::pay_icon(
                                    ui,
                                    brand,
                                    Some(flag),
                                    sponsor::title(ch, zh),
                                    hint,
                                )
                                .clicked()
                                {
                                    clicked = Some(i);
                                }
                            }
                        });
                    }
                });
            });
        if let Some(i) = clicked {
            if let Some(ch) = self.sponsor_pack.as_ref().and_then(|p| p.channels.get(i)) {
                let ch = ch.clone();
                self.open_channel_qr(&ch);
            }
        }
    }

    fn tick_nav_anim(&mut self, ctx: &egui::Context) {
        let target = if self.nav_expanded {
            NAV_W
        } else {
            NAV_COLLAPSED
        };
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.05);
        let t = 1.0 - (-12.0 * dt).exp();
        self.nav_anim_w += (target - self.nav_anim_w) * t;
        if (self.nav_anim_w - target).abs() < 0.5 {
            self.nav_anim_w = target;
        } else {
            ctx.request_repaint();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let expanded = self.nav_anim_w > 118.0;
        ui.set_min_height(ui.available_height());
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
        let tip = i18n::t(
            self.lang,
            if expanded {
                "nav_collapse"
            } else {
                "nav_expand"
            },
        );
        if self.nav_tex.is_none() {
            self.nav_tex = load_nav_tex(ui.ctx());
        }
        let icons = self.nav_tex.clone();
        if theme::menu_toggle(ui, icons.as_ref().map(|t| &t.sidebar))
            .on_hover_text(tip)
            .clicked()
        {
            self.nav_expanded = !self.nav_expanded;
            self.persist_prefs();
        }
        let go = |ui: &mut egui::Ui, icon: Option<&TextureHandle>, text: &str, on: bool| {
            theme::nav_icon_item(ui, icon, text, on, expanded).clicked()
        };
        if go(
            ui,
            icons.as_ref().map(|t| &t.search),
            i18n::t(self.lang, "search"),
            self.page == NavPage::Search,
        ) {
            self.goto(NavPage::Search);
        }
        if go(
            ui,
            icons.as_ref().map(|t| &t.fav),
            i18n::t(self.lang, "favorites"),
            self.page == NavPage::Favorites,
        ) {
            self.goto(NavPage::Favorites);
        }
        let queue_label = if self.lib.queue.is_empty() {
            i18n::t(self.lang, "queue").to_string()
        } else {
            format!("{} ({})", i18n::t(self.lang, "queue"), self.lib.queue.len())
        };
        if go(
            ui,
            icons.as_ref().map(|t| &t.queue),
            &queue_label,
            self.page == NavPage::Queue,
        ) {
            self.goto(NavPage::Queue);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if go(
                ui,
                icons.as_ref().map(|t| &t.about),
                i18n::t(self.lang, "about"),
                self.page == NavPage::About,
            ) {
                self.about_note = None;
                self.goto(NavPage::About);
            }
            if go(
                ui,
                icons.as_ref().map(|t| &t.settings),
                i18n::t(self.lang, "settings"),
                self.page == NavPage::Settings,
            ) {
                self.goto(NavPage::Settings);
            }
            ui.add_space(6.0);
            ui.add(egui::Separator::default().spacing(8.0));
            if go(
                ui,
                icons.as_ref().map(|t| &t.sponsor),
                i18n::t(self.lang, "sponsor"),
                self.page == NavPage::Sponsor,
            ) {
                self.goto(NavPage::Sponsor);
            }
        });
    }

    fn pending_can_merge(&self) -> bool {
        let parts: Vec<_> = if let Some(items) = &self.export_items_pending {
            items.iter().map(|item| {
                let id = item.lcsc_id().unwrap_or_else(|| item.name().to_string());
                (id.to_ascii_uppercase(), item.has_symbol_or_footprint(), false, false)
            }).collect()
        } else if let Some(ids) = &self.export_ids_pending {
            ids.iter().map(|id| {
                let cad = self.lib.cached_item(id).map(|item| item.has_symbol_or_footprint()).unwrap_or(true);
                (id.to_ascii_uppercase(), cad, false, false)
            }).collect()
        } else { Vec::new() };
        can_merge_library_parts(self.batch_opts.ad, self.batch_opts.kicad, parts)
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
            let count = self.export_items_pending.as_ref().map(Vec::len)
                .or_else(|| self.export_ids_pending.as_ref().map(|ids| ids.len().max(1))).unwrap_or(1);
            ui.label(theme::heading(format!("{count} {}", i18n::t(lang, "components"))));
            ui.label(egui::RichText::new(i18n::t(lang, "batch_hint")).color(theme::secondary()));
            ui.add(egui::Label::new(egui::RichText::new(i18n::t(lang, "export_copy_hint")).small().color(theme::secondary())).wrap());
            ui.add_space(8.0);
            let before = self.batch_opts;
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
            if before != self.batch_opts {
                self.persist_prefs();
            }
            if self.pending_can_merge() {
                ui.add_space(4.0);
                let before = self.batch_merge;
                ui.checkbox(&mut self.batch_merge, i18n::t(lang, "batch_merge"));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(i18n::t(lang, "batch_merge_hint"))
                            .color(theme::secondary())
                            .small(),
                    )
                    .wrap(),
                );
                if self.batch_merge != before {
                    self.persist_prefs();
                }
            }
            ui.add_space(10.0);
            ui.separator();
            ui.label(egui::RichText::new(i18n::t(lang, "output")).small().color(theme::secondary()));
            ui.add(egui::Label::new(egui::RichText::new(&self.out_dir).small()).truncate()).on_hover_text(&self.out_dir);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::pill_button(ui, i18n::t(lang, "batch_start"), !self.busy() && self.batch_opts.any(), true).clicked() {
                        start = true;
                    }
                    if theme::pill_button(ui, i18n::t(lang, "batch_cancel"), true, false).clicked() {
                        close = true;
                    }
                    if consume_dialog_confirm(ui, true) {
                        start = true;
                    }
                });
            });
        });
        if start {
            if !self.batch_opts.any() {
                self.alert(i18n::t(self.lang, "batch_none"));
            } else {
                self.persist_prefs();
                if let Some(items) = self.export_items_pending.take() { self.export_exact_items(items); }
                else { match self.export_ids_pending.take() {
                    Some(ids) if ids.is_empty() => self.export_selected(),
                    Some(ids) => self.export_ids(ids),
                    None => {}
                } }
                close = true;
            }
        }
        close |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if close {
            self.show_batch = false;
            self.export_ids_pending = None;
            self.export_items_pending = None;
        }
    }

    fn request_update_check(&mut self, manual: bool) {
        if self.update.is_some() || self.update_job.is_some() {
            if manual {
                self.about_note = None;
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
        let notes = info.notes.trim().to_string();
        let width = if notes.is_empty() { 400.0 } else { 500.0 };
        let tall = notes.lines().count() > 8 || notes.chars().count() > 240;
        let lang = self.lang;
        let progress = self.update_progress.clone();
        fit_window(i18n::t(lang, "update_title"), "lceda_update_fit", width)
            .max_height(if tall { 520.0 } else { 800.0 })
            .show(ctx, |ui| {
                ui.set_width(width - 8.0);
                ui.add(egui::Label::new(egui::RichText::new(body).color(theme::label())).wrap());
                if !notes.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(i18n::t(lang, "update_notes"))
                            .color(theme::secondary())
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                    let cache = &mut self.md_cache;
                    if tall {
                        egui::ScrollArea::vertical()
                            .id_salt("update_notes")
                            .max_height(280.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                theme::show_markdown(ui, |ui| {
                                    CommonMarkViewer::new().show(ui, cache, &notes);
                                });
                            });
                    } else {
                        theme::show_markdown(ui, |ui| {
                            CommonMarkViewer::new().show(ui, cache, &notes);
                        });
                    }
                }
                if downloading {
                    ui.add_space(8.0);
                    let (label, fraction, indeterminate) = progress
                        .as_ref()
                        .and_then(|p| p.lock().ok())
                        .map(|g| {
                            let key = match g.phase {
                                UpdatePhase::Extracting => "update_extracting",
                                UpdatePhase::Installing => "update_installing",
                                _ => "update_downloading",
                            };
                            (i18n::t(lang, key), g.fraction, g.indeterminate)
                        })
                        .unwrap_or((i18n::t(lang, "update_downloading"), 0.0, true));
                    ui.label(egui::RichText::new(label).color(theme::secondary()));
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
                                    i18n::t(lang, "update_now"),
                                    true,
                                    true,
                                )
                                .clicked()
                            {
                                now = true;
                            }
                            if theme::pill_button(ui, i18n::t(lang, "update_open"), true, false)
                                .clicked()
                            {
                                open = true;
                            }
                            if theme::pill_button(
                                ui,
                                i18n::t(lang, "update_later"),
                                true,
                                false,
                            )
                            .clicked()
                            {
                                later = true;
                            }
                            if consume_dialog_confirm(ui, true) {
                                if info.zip_url.is_some() {
                                    now = true;
                                } else {
                                    open = true;
                                }
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
        self.search.is_some() || self.job.is_some() || self.resolve.is_some() || self.bom_job.is_some() || self.bind_job.is_some() || self.native_load.is_some() || self.native_save.is_some() || self.resource_assist.as_ref().is_some_and(ResourceAssist::is_busy)
    }

    fn selected_item(&self) -> Option<&SearchItem> {
        self.detail.as_ref()
    }


}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        // Isolated screenshot runs can exercise real widgets without sending
        // mouse/keyboard input to the user's desktop or requiring OS focus.
        let Some(path) = &self.shot_input else { return; };
        input.events.clear(); input.focused = true; input.modifiers = egui::Modifiers::NONE;
        for viewport in input.viewports.values_mut() { viewport.focused = Some(true); }
        if let Some(pos) = self.shot_pointer_release.take() {
            input.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            return;
        }
        let Some(command) = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()) else { return; };
        let Some(id) = command["id"].as_u64().filter(|id| *id > self.shot_input_id) else { return; };
        self.shot_input_id = id;
        if let Some(key) = command["key"].as_str().and_then(|key| match key { "Escape" => Some(egui::Key::Escape), "Tab" => Some(egui::Key::Tab), "Enter" => Some(egui::Key::Enter), _ => None }) {
            for pressed in [true, false] { input.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }); }
        } else if let (Some(x), Some(y)) = (command["x"].as_f64(), command["y"].as_f64()) {
            let pos = egui::pos2(x as f32, y as f32); input.events.push(egui::Event::PointerMoved(pos));
            if command["click"].as_bool() == Some(true) {
                input.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
                self.shot_pointer_release = Some(pos); ctx.request_repaint();
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.win_max {
            let c = theme::window_bg();
            [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
                1.0,
            ]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_theme(ctx);
        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let screen = ctx.input(|i| i.screen_rect());
        let sig = (
            screen.width().round() as i32,
            screen.height().round() as i32,
            maxed,
        );
        if self.win_round_sig != Some(sig) {
            if crate::instance::apply_window_rounding(maxed, screen.width()) {
                self.win_round_sig = Some(sig);
            } else {
                ctx.request_repaint();
            }
        }
        paint_window_shell(ctx, maxed);
        handle_window_resize(ctx, maxed);
        if self.instance.wakeup.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        ctx.request_repaint_after(Duration::from_millis(300));
        self.poll_jobs(ctx);
        self.show_title_bar(ctx);
        self.tick_nav_anim(ctx);
        self.keyboard_shortcuts(ctx);
        self.status_bar(ctx);

        let nav_w = self.nav_anim_w;
        let nav_pad = if self.nav_expanded { 10 } else { 8 };
        egui::SidePanel::left("nav")
            .exact_width(12.0 + nav_w)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(shell_panel_fill(maxed))
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 0,
                        top: 12,
                        bottom: 12,
                    }),
            )
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::fill())
                    .stroke(egui::Stroke::new(1.0_f32, theme::hairline()))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::same(nav_pad))
                    .show(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        self.sidebar(ui);
                    });
            });

        if matches!(self.page, NavPage::Search | NavPage::Favorites | NavPage::Queue) {
            let win_w = ctx.input(|i| i.screen_rect().width());
            let parts_max = clamp_parts_max(win_w, 12.0 + nav_w);
            let parts_min = PARTS_MIN.min(parts_max);
            let parts_w = self.parts_width.clamp(parts_min, parts_max);
            let parts = egui::SidePanel::left("parts")
                .resizable(false)
                .exact_width(parts_w)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(shell_panel_fill(maxed))
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 4,
                            top: 12,
                            bottom: 12,
                        }),
                )
                .show(ctx, |ui| {
                    self.left_pane(ui);
                });
            let (dx, done) = parts_split_drag(ctx, parts.response.rect);
            if dx != 0.0 {
                self.parts_width = (parts_w + dx).clamp(parts_min, parts_max);
                ctx.request_repaint();
            }
            if done {
                self.persist_prefs();
            }
        }

        let center_margin = if matches!(self.page, NavPage::Search | NavPage::Favorites | NavPage::Queue)
        {
            egui::Margin {
                left: 4,
                right: 12,
                top: 12,
                bottom: 12,
            }
        } else {
            egui::Margin::same(12)
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .inner_margin(center_margin)
                    .fill(shell_panel_fill(maxed)),
            )
            .show(ctx, |ui| match self.page {
                NavPage::Search | NavPage::Favorites | NavPage::Queue => self.right_pane(ui),
                NavPage::Sponsor => self.page_sponsor(ui),
                NavPage::Settings => self.page_settings(ui),
                NavPage::About => self.page_about(ui),
            });
        self.show_modal_scrim(ctx);
        self.bom_import_dialog(ctx);
        self.native_import_dialog(ctx);
        self.native_export_dialog(ctx);
        self.resource_assist_dialog(ctx);
        self.bind_library_dialog(ctx);
        self.show_welcome(ctx);
        self.show_detail(ctx);
        self.show_batch_dialog(ctx);
        self.show_sponsor_qr(ctx);
        self.show_fav_pick(ctx);
        self.show_cat_dialog(ctx);
        self.activity_dialog(ctx);
        self.library_sync_dialog(ctx);
        self.clear_queue_dialog(ctx);
        self.show_alert(ctx);
        if !self.show_welcome {
            self.show_update(ctx);
        }
        self.tick_debug_shot(ctx);
        self.persist_window(ctx);
        handle_window_resize(ctx, maxed);
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.persist_prefs();
        if let (Some(gl), Some(gpu)) = (gl, self.gpu.take()) {
            gpu.lock().destroy(gl);
        }
    }
}

impl App {
    fn poll_jobs(&mut self, ctx: &egui::Context) {
        self.poll_native_jobs(ctx);
        self.poll_resource_assist(ctx);
        self.poll_binding_validation();
        if !self.busy() && !self.pending_library_sync.is_empty() && self.library_sync_after.is_none_or(|when| Instant::now() >= when) { self.start_library_sync(); }
        if self.bom_job.as_ref().is_some_and(|p| p.ready().is_some()) {
            match self.bom_job.take().unwrap().block_and_take() {
                Ok(doc) => { self.log(format!("BOM: {} ({} rows)", doc.source, doc.rows.len())); self.bom = Some(doc); }
                Err(e) => self.alert(e),
            }
        }

        if let Some(p) = &self.search {
            if p.ready().is_some() {
                match self.search.take().unwrap().block_and_take() {
                    Ok(items) => {
                        self.log(format!("{}: {}", i18n::t(self.lang, "search"), items.len()));
                        self.items = items;
                        self.selected = if self.items.is_empty() { None } else { Some(0) };
                        self.detail = self.items.first().cloned();
                        if let Some(item) = &self.detail {
                            if let Some(id) = item.lcsc_id() {
                                self.cache.insert(id, item.clone());
                self.lib.enrich_from_item(item);
                            }
                        }
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
                let syncing = std::mem::take(&mut self.library_sync_job);
                match self.job.take().unwrap().block_and_take() {
                    Ok(msg) => {
                        for (item, paths) in &msg.exported {
                            self.lib.enrich_from_item(item);
                            if let Some(id) = item.lcsc_id() { self.lib.record_export(&id, paths); }
                        }
                        if let Err(e) = self.lib.try_save() {
                            self.log(format!("本地库保存失败: {e}"));
                            if syncing { if let Some(task) = &self.library_sync_task { task.fail(format!("库文件已处理，但本地记录保存失败: {e}")); } self.show_library_sync = true; }
                        }
                        if syncing {
                            self.log(msg.message);
                            if msg.failed > 0 { self.show_library_sync = true; }
                        } else if msg.notify || msg.failed > 0 { self.alert(msg.message); } else { self.log(msg.message); }
                    }
                    Err(e) => {
                        if syncing {
                            self.log(e.to_string());
                            if let Some(task) = &self.library_sync_task { task.fail(e.to_string()); }
                            self.show_library_sync = true;
                        } else { self.alert(format!("{}: {e}", i18n::t(self.lang, "error"))); }
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.preview {
            if p.ready().is_some() {
                let data = self.preview.take().unwrap().block_and_take();
                if let Some(id) = &data.lcsc {
                    let record = self.lib.records.entry(id.clone()).or_default();
                    for (kind, path) in &data.files { record.resources.insert(kind.clone(), path.clone()); }
                    self.lib.save();
                }
                self.image_tex = data.image.as_ref().and_then(|b| load_texture(ctx, b));
                self.mesh = data.mesh.map(Arc::new);
                self.mesh_note = data.mesh_note;
                self.symbol = data.symbol;
                self.native_scene = data.native_scene;
                self.symbol_note = data.symbol_note;
                self.footprint = data.footprint;
                self.footprint_note = data.footprint_note;
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
                        self.about_note = None;
                    }
                    CheckResult::UpToDate if manual => {
                        let note = i18n::t(self.lang, "already_latest")
                            .replace("{ver}", update::current_version());
                        if self.page == NavPage::About {
                            self.about_note = Some(note);
                        } else {
                            self.alert(note);
                        }
                    }
                    CheckResult::Failed if manual => {
                        let note = i18n::t(self.lang, "update_check_fail").to_string();
                        if self.page == NavPage::About {
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
        if let Some(p) = &self.sponsor_job {
            if p.ready().is_some() {
                match self.sponsor_job.take().unwrap().block_and_take() {
                    Ok(pack) => {
                        self.sponsor_note = None;
                        self.sponsor_pack = Some(pack);
                    }
                    Err(e) => {
                        self.sponsor_note = Some(e);
                        self.sponsor_pack = None;
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.sponsor_qr_job {
            if p.ready().is_some() {
                match self.sponsor_qr_job.take().unwrap().block_and_take() {
                    Ok((title, bytes)) => {
                        self.sponsor_qr = load_named_texture(ctx, "sponsor-qr", &bytes)
                            .map(|tex| (title, tex));
                        if self.sponsor_qr.is_none() {
                            self.sponsor_note = Some("无法显示收款码".into());
                        }
                    }
                    Err(e) => {
                        self.sponsor_note = Some(e);
                        self.sponsor_qr = None;
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.resolve {
            if p.ready().is_some() {
                match self.resolve.take().unwrap().block_and_take() {
                    Ok((lcsc, mut item)) => {
                        if item.manufacturer.is_empty() {
                            if let Some(mfr) = self.lib.saved_manufacturer(&lcsc) {
                                item.manufacturer = mfr.to_string();
                            }
                        }
                        self.lib.enrich_from_item(&item);
                        if let Err(e) = self.lib.try_save() { self.alert(format!("保存器件资料失败: {e}")); }
                        self.cache.insert(lcsc, item.clone());
                        self.detail = Some(item);
                        self.queue_preview();
                    }
                    Err(e) => self.alert(format!("{}: {e}", i18n::t(self.lang, "error"))),
                }
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(p) = &self.desc_preview_job {
            if p.ready().is_some() {
                match self.desc_preview_job.take().unwrap().block_and_take() {
                    Ok(items) => {
                        let kw = self.desc_preview_kw.trim();
                        let item = items
                            .iter()
                            .find(|i| {
                                i.lcsc_id().as_deref() == Some(kw)
                                    || i.name().eq_ignore_ascii_case(kw)
                            })
                            .cloned()
                            .or_else(|| items.into_iter().next());
                        if let Some(item) = item {
                            if let Some(id) = item.lcsc_id() {
                                self.cache.insert(id, item.clone());
                            }
                            self.desc_preview_item = Some(item);
                            self.desc_preview_note = None;
                        } else {
                            self.desc_preview_item = None;
                            self.desc_preview_note =
                                Some(i18n::t(self.lang, "desc_preview_none").into());
                        }
                    }
                    Err(e) => {
                        self.desc_preview_item = None;
                        self.desc_preview_note = Some(e.to_string());
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
    }

    fn left_pane(&mut self, ui: &mut egui::Ui) {
        match self.page {
            NavPage::Search => self.left_search(ui),
            NavPage::Favorites => self.left_favorites(ui),
            NavPage::Queue => self.left_queue(ui),
            _ => {}
        }
    }

    fn left_search(&mut self, ui: &mut egui::Ui) {
        let mut submitted = false;
        let mut clicked = None;
        let mut ctx_idx = None;
        let mut ctx_act = PartAct::None;
        theme::card_frame().show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.label(theme::heading(i18n::t(self.lang, "search")).size(18.0));
            ui.add_space(6.0);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let btn_w = 64.0;
                let edit_w = (ui.available_width() - btn_w - 8.0).max(80.0);
                let r = theme::search_field(
                    ui,
                    &mut self.keyword,
                    i18n::t(self.lang, "keyword_hint"),
                    "mid_search",
                    egui::vec2(edit_w, 32.0),
                );
                if (r.has_focus() || r.lost_focus())
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.dialog_open()
                {
                    submitted = true;
                }
                if theme::pill_button(ui, i18n::t(self.lang, "search"), !self.busy(), true).clicked()
                {
                    submitted = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(theme::heading(i18n::t(self.lang, "components")));
                ui.label(egui::RichText::new(self.items.len().to_string()).small().color(theme::secondary()));
                if self.search.is_some() { ui.spinner(); }
            });
            if self.items.is_empty() && self.search.is_none() {
                ui.add_space(20.0);
                ui.label(theme::heading(i18n::t(self.lang, "search_empty")));
                ui.label(egui::RichText::new(i18n::t(self.lang, "search_empty_hint")).small().color(theme::secondary()));
            }
            egui::ScrollArea::vertical().id_salt("part_list").show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (idx, item) in self.items.iter().enumerate() {
                    let selected = self.selected == Some(idx);
                    let mut sub = item.lcsc_id().unwrap_or_default();
                    let mfr = item.manufacturer_label();
                    if !mfr.is_empty() {
                        if !sub.is_empty() {
                            sub.push_str("  ");
                        }
                        sub.push_str(&mfr);
                    }
                    if item.model_uuid.is_some() {
                        if !sub.is_empty() {
                            sub.push_str("  ·  ");
                        }
                        sub.push_str("3D");
                    }
                    let resp = paint_part_row(
                        ui,
                        &format!("{}.  {}", item.index, item.name()),
                        &sub,
                        selected,
                    );
                    if resp.double_clicked() {
                        clicked = Some(idx);
                        ctx_act = PartAct::Queue;
                    } else if resp.clicked() {
                        clicked = Some(idx);
                    }
                    resp.context_menu(|ui| {
                        theme::prepare_menu(ui);
                        if ui.button(i18n::t(self.lang, "export_default")).clicked() {
                            ctx_idx = Some(idx);
                            ctx_act = PartAct::Export;
                            ui.close();
                        }
                        if ui.button(i18n::t(self.lang, "add_queue")).clicked() {
                            ctx_idx = Some(idx);
                            ctx_act = PartAct::Queue;
                            ui.close();
                        }
                        if ui.button(i18n::t(self.lang, "add_fav")).clicked() {
                            ctx_idx = Some(idx);
                            ctx_act = PartAct::Fav;
                            ui.close();
                        }
                    });
                }
            });
        });
        if submitted && !self.busy() {
            self.do_search();
        }
        if let Some(idx) = clicked.or(ctx_idx) {
            self.select_search(idx);
        }
        match ctx_act {
            PartAct::Export => self.ask_export_selected(),
            PartAct::Queue => self.add_to_queue(),
            PartAct::Fav => self.add_to_fav(),
            PartAct::None => {}
        }
    }

    fn left_favorites(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let mut persist = false;
        let mut export_cat = false;
        let mut export_part: Option<String> = None;
        let mut pick: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut select_cat: Option<String> = None;
        let mut enter_cat: Option<String> = None;
        let mut leave = false;
        let mut open_new = false;
        let mut open_import = false;
        let mut open_rename = false;
        let mut delete = false;
        let mut import_bom = false;
        let mut import_format = None;
        let mut library_action = None;
        let mut resource_ids = None;
        let cats: Vec<(String, String, usize)> = self
            .lib
            .cats
            .iter()
            .map(|c| {
                let name = if c.id == UNCAT {
                    i18n::t(lang, "uncategorized").to_string()
                } else {
                    c.name.clone()
                };
                (c.id.clone(), name, self.lib.favs_in(&c.id).len())
            })
            .collect();
        let parts: Vec<(String, String, String)> = self
            .lib
            .favs_in(&self.fav_cat)
            .into_iter()
            .map(|p| (p.lcsc.clone(), p.name.clone(), p.manufacturer.clone()))
            .collect();
        let selected_lcsc = self.native_selected.clone().or_else(||self.detail.as_ref().and_then(|d| d.lcsc_id()));
        let cat_title = cats
            .iter()
            .find(|(id, _, _)| id == &self.fav_cat)
            .map(|(_, name, _)| name.clone())
            .unwrap_or_else(|| i18n::t(lang, "favorites").to_string());
        let unit = i18n::t(lang, "fav_item_unit");
        theme::card_frame().show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.horizontal_wrapped(|ui| {
                dropdown_menu(ui,i18n::t(lang,"library_import_menu"),|ui|{
                    theme::prepare_menu(ui);
                    if ui.add_enabled(!self.busy(),egui::Button::new(i18n::t(lang,"import_bom"))).clicked(){import_bom=true;ui.close();}
                    for format in LibraryFormat::ALL {
                        let text=i18n::t(lang,if format==LibraryFormat::Ad{"import_ad_library"}else{"import_kicad_library"});
                        if ui.add_enabled(!self.busy(),egui::Button::new(text)).clicked(){import_format=Some(format);ui.close();}
                    }
                });
                if ui.button(i18n::t(lang, "new_cat")).clicked() { open_new = true; }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                dropdown_menu(ui, i18n::t(lang, "library_export_menu"), |ui| {
                    theme::prepare_menu(ui);
                    let selected = selected_lcsc.as_ref().filter(|id| parts.iter().any(|part| &part.0 == *id));
                    if ui.add_enabled(!self.busy() && selected.is_some(), egui::Button::new(i18n::t(lang, "export_library_part"))).clicked() {
                        export_part = selected.cloned(); ui.close();
                    }
                    if ui.add_enabled(!self.busy() && !parts.is_empty(), egui::Button::new(i18n::t(lang, "export_library_all"))).clicked() {
                        export_cat = true; ui.close();
                    }
                });
                dropdown_menu(ui, i18n::t(lang, "library_sync_menu"), |ui| {
                    library_action = library_menu(ui, &self.lib, &self.fav_cat, lang, self.busy(), self.library_sync_task.is_some());
                });
                dropdown_menu(ui,i18n::t(lang,"resource_menu"),|ui|{
                    theme::prepare_menu(ui);
                    if ui.add_enabled(!self.busy()&&selected_lcsc.as_ref().is_some_and(|id|self.lib.native(id).is_some()),egui::Button::new(i18n::t(lang,"resource_current"))).clicked(){resource_ids=selected_lcsc.clone().map(|id|vec![id]);ui.close();}
                    let ids:Vec<_>=parts.iter().filter(|p|self.lib.native(&p.0).is_some()).map(|p|p.0.clone()).collect();
                    if ui.add_enabled(!self.busy()&&!ids.is_empty(),egui::Button::new(i18n::t(lang,"resource_library"))).clicked(){resource_ids=Some(ids);ui.close();}
                });
            });
            ui.add_space(8.0);
            if self.fav_inside {
                ui.horizontal(|ui| {
                    if theme::pill_button(ui, i18n::t(lang, "fav_back"), true, false).clicked() {
                        leave = true;
                    }
                    ui.add(egui::Label::new(theme::heading(cat_title)).truncate());
                });
                for format in LibraryFormat::ALL {
                    let caption = self.lib.binding_for(&self.fav_cat, format).map(|p| format!("{} · {}", format.name(), p.display()));
                    if let Some(caption) = caption {
                        let response = ui.add(egui::Label::new(egui::RichText::new(&caption).small().color(theme::secondary())).truncate()).on_hover_text(&caption);
                        response.context_menu(|ui| { library_action = library_menu(ui, &self.lib, &self.fav_cat, lang, self.busy(), self.library_sync_task.is_some()); });
                    }
                }
                if LibraryFormat::ALL.into_iter().all(|f| self.lib.binding_for(&self.fav_cat, f).is_none()) {
                    ui.label(egui::RichText::new(i18n::t(lang, "unbound_library")).small().color(theme::secondary()));
                }
                ui.add_space(8.0);
                theme::search_field(ui,&mut self.library_filter,i18n::t(lang,"native_filter"),"library_part_filter",egui::vec2(ui.available_width(),30.0));
                ui.add_space(6.0);
                if parts.is_empty() {
                    ui.label(egui::RichText::new(i18n::t(lang, "fav_empty")).color(theme::secondary()));
                }
                egui::ScrollArea::vertical().id_salt("fav_list").show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (lcsc, name, mfr) in &parts {
                        let native=self.lib.native(lcsc);
                        let subtitle=if let Some(record)=native{format!("{} · {} {} · {}",record.format.name(),record.part.pins,i18n::t(lang,"native_pins"),record.part.description)}else{format!("{lcsc}  {mfr}")};
                        let filter=self.library_filter.trim().to_lowercase();
                        let footprints=native.map(|record|record.part.footprints.join(" ")).unwrap_or_default();
                        if !filter.is_empty()&&!format!("{name} {subtitle} {mfr} {footprints}").to_lowercase().contains(&filter){continue;}
                        let selected = selected_lcsc.as_deref() == Some(lcsc.as_str());
                        let resp = paint_part_row(ui, name, &subtitle, selected);
                        if resp.clicked() {
                            pick = Some(lcsc.clone());
                        }
                        resp.context_menu(|ui| {
                            theme::prepare_menu(ui);
                            if self.lib.native(lcsc).is_some()&&ui.add_enabled(!self.busy(),egui::Button::new(i18n::t(lang,"resource_current"))).clicked(){resource_ids=Some(vec![lcsc.clone()]);ui.close();}
                            if ui
                                .add_enabled(!self.busy(), egui::Button::new(i18n::t(lang, "export_default")))
                                .clicked()
                            {
                                export_part = Some(lcsc.clone());
                                ui.close();
                            }
                            if ui.button(i18n::t(lang, "remove_item")).clicked() {
                                remove = Some(lcsc.clone());
                                ui.close();
                            }
                        });
                    }
                });
            } else {
                ui.label(theme::heading(i18n::t(lang, "favorites")));
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("fav_tree").show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (id, name, n) in &cats {
                        let selected = self.fav_cat == *id;
                        let formats: Vec<_> = LibraryFormat::ALL.into_iter().filter(|f| self.lib.binding_for(id, *f).is_some()).map(LibraryFormat::name).collect();
                        let subtitle = if formats.is_empty() { format!("{n} {unit}") } else { format!("{n} {unit} · {}", formats.join(" + ")) };
                        let r = paint_part_row(ui, name, &subtitle, selected);
                        if r.double_clicked() {
                            enter_cat = Some(id.clone());
                        } else if r.clicked() {
                            select_cat = Some(id.clone());
                        }
                        r.context_menu(|ui| {
                            theme::prepare_menu(ui);
                            if let Some(action) = library_menu(ui, &self.lib, id, lang, self.busy(), self.library_sync_task.is_some()) { library_action = Some(action); }
                            ui.separator();
                            if ui.button(i18n::t(lang, "new_cat")).clicked() {
                                open_new = true;
                                ui.close();
                            }
                            if ui.button(i18n::t(lang, "import_cat")).clicked() {
                                open_import = true;
                                ui.close();
                            }
                            if ui
                                .add_enabled(!self.busy(), egui::Button::new(i18n::t(lang, "export_cat")))
                                .clicked()
                            {
                                select_cat = Some(id.clone());
                                export_cat = true;
                                ui.close();
                            }
                            if id != UNCAT {
                                if ui.button(i18n::t(lang, "rename_cat")).clicked() {
                                    select_cat = Some(id.clone());
                                    open_rename = true;
                                    ui.close();
                                }
                                if ui.button(i18n::t(lang, "del_cat")).clicked() {
                                    select_cat = Some(id.clone());
                                    delete = true;
                                    ui.close();
                                }
                            }
                        });
                    }
                    let leftover = ui.allocate_response(
                        egui::vec2(ui.available_width(), ui.available_height().max(24.0)),
                        egui::Sense::click(),
                    );
                    leftover.context_menu(|ui| {
                        theme::prepare_menu(ui);
                        if ui.button(i18n::t(lang, "new_cat")).clicked() {
                            open_new = true;
                            ui.close();
                        }
                        if ui.button(i18n::t(lang, "import_cat")).clicked() {
                            open_import = true;
                            ui.close();
                        }
                    });
                });
            }
        });
        if let Some(action) = library_action { self.handle_library_action(action); }
        if import_bom { self.import_bom(); }
        if let Some(format)=import_format{self.import_native_library(format);}
        if let Some(ids)=resource_ids{self.open_resource_assist(ids,Default::default());}
        if let Some(id) = select_cat {
            self.fav_cat = id;
        }
        if let Some(id) = enter_cat {
            self.fav_cat = id;
            self.library_filter.clear();
            self.fav_inside = true;
        }
        if leave {
            self.fav_inside = false;
        }
        if open_new {
            self.new_cat_name.clear();
            self.show_new_cat = true;
        }
        if open_import {
            self.new_cat_name.clear();
            self.show_import_cat = true;
        }
        if open_rename && self.fav_cat != UNCAT {
            self.new_cat_name = self
                .lib
                .cat(&self.fav_cat)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            self.show_rename_cat = true;
        }
        if delete && self.fav_cat != UNCAT {
            self.lib.remove_cat(&self.fav_cat.clone());
            self.fav_cat = UNCAT.into();
            self.fav_inside = false;
            persist = true;
        }
        if let Some(lcsc) = remove {
            self.lib.remove_fav(&lcsc);
            persist = true;
        }
        if persist {
            self.lib.save();
        }
        if export_cat {
            let ids = self
                .lib
                .favs_in(&self.fav_cat)
                .into_iter()
                .map(|p| p.lcsc.clone())
                .collect();
            self.ask_export_ids(ids);
        }
        if let Some(lcsc) = export_part {
            self.ask_export_ids(vec![lcsc]);
        }
        if let Some(lcsc) = pick {
            self.select_saved(&lcsc);
        }
    }

    fn left_queue(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let mut export = false;
        let mut clear = false;
        let mut import = false;
        let mut pick: Option<String> = None;
        let mut remove: Option<String> = None;
        let parts: Vec<(String, String, String)> = self
            .lib
            .queue
            .iter()
            .map(|p| (p.lcsc.clone(), p.name.clone(), p.manufacturer.clone()))
            .collect();
        let selected_lcsc = self.detail.as_ref().and_then(|d| d.lcsc_id());
        theme::card_frame().show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.horizontal_wrapped(|ui| {
                if theme::pill_button(ui, i18n::t(lang, "queue_import"), !self.busy(), false).clicked()
                {
                    import = true;
                }
                if theme::pill_button(
                    ui,
                    i18n::t(lang, "export_default"),
                    !parts.is_empty() && !self.busy(),
                    true,
                )
                .clicked()
                {
                    export = true;
                }
                if theme::pill_button(ui, i18n::t(lang, "queue_clear"), !parts.is_empty(), false)
                    .clicked()
                {
                    clear = true;
                }
            });
            ui.add_space(8.0);
            if parts.is_empty() {
                ui.label(egui::RichText::new(i18n::t(lang, "queue_empty")).color(theme::secondary()));
            }
            egui::ScrollArea::vertical().id_salt("queue_list").show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (lcsc, name, mfr) in &parts {
                    let selected = selected_lcsc.as_deref() == Some(lcsc.as_str());
                    let title = if name.is_empty() { lcsc.as_str() } else { name.as_str() };
                    let resp = paint_part_row(ui, title, &format!("{lcsc}  {mfr}"), selected);
                    if resp.clicked() {
                        pick = Some(lcsc.clone());
                    }
                    resp.context_menu(|ui| {
                        theme::prepare_menu(ui);
                        if ui.button(i18n::t(lang, "remove_item")).clicked() {
                            remove = Some(lcsc.clone());
                            ui.close();
                        }
                    });
                }
            });
        });
        if clear { self.confirm_clear_queue = true; }
        if let Some(lcsc) = remove {
            self.lib.remove_queue(&lcsc);
            self.lib.save();
        }
        if import {
            self.import_queue_file();
        }
        if export {
            let ids = self.lib.queue.iter().map(|p| p.lcsc.clone()).collect();
            self.ask_export_ids(ids);
        }
        if let Some(lcsc) = pick {
            self.select_saved(&lcsc);
        }
    }

    fn right_pane(&mut self, ui: &mut egui::Ui) {
        self.part_header(ui);
        ui.add_space(GAP);
        let area = ui.available_rect_before_wrap();
        let previous = self.preview_mode;
        let previous_unit=self.native_unit;
        card_shell(ui, area, "part_preview", |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("preview_mode")
                    .selected_text(i18n::t(self.lang, self.preview_mode.i18n_key()))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for mode in [PreviewMode::Schematic, PreviewMode::Model3d, PreviewMode::Footprint] {
                            ui.selectable_value(&mut self.preview_mode, mode, i18n::t(self.lang, mode.i18n_key()));
                        }
                    });
                if let Some(record)=self.native_selected.as_ref().and_then(|id|self.lib.native(id)).filter(|r|r.part.units>1){
                    egui::ComboBox::from_id_salt("native_unit").selected_text(format!("{} {}/{}",i18n::t(self.lang,"native_unit"),self.native_unit,record.part.units)).show_ui(ui,|ui|{
                        for unit in 1..=record.part.units{ui.selectable_value(&mut self.native_unit,unit,format!("{} {unit}",i18n::t(self.lang,"native_unit")));}
                    });
                }
            });
            ui.add_space(6.0);
            match self.preview_mode {
                PreviewMode::Schematic => self.schematic_canvas(ui),
                PreviewMode::Model3d => self.mesh_canvas(ui),
                PreviewMode::Footprint => self.footprint_canvas(ui),
            }
        });
        if previous != self.preview_mode { self.persist_prefs(); }
        if previous_unit!=self.native_unit{self.queue_native_preview();}
        ui.advance_cursor_after_rect(area);
    }

    fn part_header(&mut self, ui: &mut egui::Ui) {
        if self.native_selected.is_some(){self.native_header(ui);return;}
        let item = self.selected_item().cloned();
        let lang = self.lang;
        let enabled = !self.busy() && item.is_some();
        let mut export = false;
        let mut queue = false;
        let mut fav = false;
        let mut details = false;
        let mut open_page = false;
        let in_queue = item.as_ref().and_then(SearchItem::lcsc_id).is_some_and(|id| self.lib.in_queue(&id));
        let is_favorite = item.as_ref().and_then(SearchItem::lcsc_id).is_some_and(|id| self.lib.favs.iter().any(|part| part.lcsc == id));
        theme::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                if let Some(tex) = &self.image_tex {
                    let side = (ui.available_width() * 0.25).clamp(128.0, 164.0);
                    let response = ui.add(egui::Image::new((tex.id(), tex.size_vec2())).fit_to_exact_size(egui::Vec2::splat(side)).corner_radius(8));
                    if let Some(pointer) = response.hover_pos() {
                        if !self.dialog_open() { paint_photo_zoom(ui, tex, response.rect, pointer, ui.max_rect()); }
                    }
                    ui.add_space(4.0);
                }
                let width = ui.available_width();
                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(egui::Label::new(theme::heading(item.as_ref().map(|i| i.name()).unwrap_or(i18n::t(lang, "part_workspace"))).size(18.0)).truncate())
                        .on_hover_text(item.as_ref().map(|i| i.name()).unwrap_or_default());
                    if let Some(it) = &item {
                        ui.add(egui::Label::new(egui::RichText::new(format!("{}  ·  {}", it.lcsc_id().unwrap_or_default(), it.manufacturer_label())).color(theme::secondary()).size(12.0)).truncate());
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().interact_size.y = 18.0;
                            theme::badge(ui, i18n::t(lang, "cad_assets"), it.has_symbol_or_footprint());
                            theme::badge(ui, "3D", it.model_uuid.is_some());
                            theme::badge(ui, "PDF", it.datasheet_url().is_some());
                        });
                    } else {
                        ui.label(egui::RichText::new(i18n::t(lang, "workspace_hint")).color(theme::secondary()).size(12.0));
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        export = theme::pill_button(ui, i18n::t(lang, "export_choose"), enabled, true)
                            .on_disabled_hover_text(i18n::t(lang, if item.is_some() { "working" } else { "select_first" }))
                            .on_hover_text(i18n::t(lang, "export_unified_hint")).clicked();
                        queue = theme::pill_button(ui, i18n::t(lang, if in_queue { "added_queue" } else { "add_queue" }), enabled && !in_queue, false).clicked();
                        fav = theme::pill_button(ui, i18n::t(lang, if is_favorite { "added_fav" } else { "add_fav" }), enabled, false).on_hover_text(i18n::t(lang, "fav_manage_hint")).clicked();
                        details = theme::pill_button(ui, i18n::t(lang, "details"), item.is_some(), false).clicked();
                        open_page = theme::pill_button(ui, i18n::t(lang, "open_page"), item.is_some(), false).clicked();
                    });
                });
            });
            if let Some(id) = item.as_ref().and_then(SearchItem::lcsc_id) {
                let bindings: Vec<_> = LibraryFormat::ALL.into_iter().filter_map(|format| {
                    self.lib.binding_for_part_format(&id, format).map(|path| (format, path))
                }).collect();
                if !bindings.is_empty() {
                    let caption = bindings.iter().map(|(format, path)| format!("{} · {}", format.name(), path.file_name().unwrap_or_default().to_string_lossy())).collect::<Vec<_>>().join("  /  ");
                    let tooltip = bindings.iter().map(|(format, path)| format!("{}: {}", format.name(), path.display())).collect::<Vec<_>>().join("\n");
                    ui.add(egui::Label::new(egui::RichText::new(caption).small().color(theme::secondary())).truncate()).on_hover_text(tooltip);
                }
            }
        });
        if export { self.ask_export_selected(); }
        if queue { self.add_to_queue(); }
        if fav { self.add_to_fav(); }
        if details { self.detail_query.clear(); self.show_detail = true; }
        if open_page { if let Some(item) = item { let _ = webbrowser::open(&item.product_url()); } }
    }

    fn schematic_canvas(&mut self, ui: &mut egui::Ui) {
        let (well, response) = ui.allocate_exact_size(ui.available_size().max(egui::vec2(1.0, 1.0)), egui::Sense::hover());
        response.on_hover_text(i18n::t(self.lang, "schematic_gestures"));
        if let Some(scene)=&self.native_scene{
            self.symbol_view.show_native(ui,well,scene,self.dialog_open());
        } else if let Some(symbol) = &self.symbol {
            self.symbol_view.show(ui, well, symbol, self.resolved_sch_colors(), self.dialog_open());
        } else {
            paint_well(ui, well);
            let message = if self.preview.is_some() { i18n::t(self.lang, "loading") }
                else { self.symbol_note.as_deref().unwrap_or(i18n::t(self.lang, "no_symbol_preview")) };
            let galley = ui.painter().layout(message.into(), egui::FontId::proportional(13.0), theme::secondary(), (well.width() - 24.0).max(1.0));
            ui.painter_at(well).galley(well.center() - galley.size() * 0.5, galley, theme::secondary());
        }
    }

    fn footprint_canvas(&mut self, ui: &mut egui::Ui) {
        let (well, response) = ui.allocate_exact_size(ui.available_size().max(egui::vec2(1.0, 1.0)), egui::Sense::hover());
        response.on_hover_text(i18n::t(self.lang, "schematic_gestures"));
        let locked = self.dialog_open();
        if let Some(footprint) = &self.footprint {
            self.footprint_view.show(ui, well, footprint, locked);
        } else {
            paint_well(ui, well);
            let message = if self.preview.is_some() { i18n::t(self.lang, "loading") }
                else { self.footprint_note.as_deref().unwrap_or(i18n::t(self.lang, "no_footprint_preview")) };
            let galley = ui.painter().layout(message.into(), egui::FontId::proportional(13.0), theme::secondary(), (well.width() - 24.0).max(1.0));
            ui.painter_at(well).galley(well.center() - galley.size() * 0.5, galley, theme::secondary());
        }
    }

    fn mesh_canvas(&mut self, ui: &mut egui::Ui) {
        let well_h = ui.available_height().max(1.0);
        let well_w = ui.max_rect().width();
        let locked = self.dialog_open();
        let (well, resp) = ui.allocate_exact_size(
            egui::vec2(well_w, well_h),
            if locked {
                egui::Sense::hover()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        paint_well(ui, well);
        if !locked && resp.dragged() {
            let d = resp.drag_delta();
            self.yaw += d.x * 0.01;
            self.pitch += d.y * 0.01;
        }
        if !locked && resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = scroll_zoom(self.zoom, scroll, 0.6, 5.0);
            }
        }
        if !locked && resp.double_clicked() {
            self.yaw = 0.7;
            self.pitch = 0.55;
            self.zoom = 1.0;
        }
        resp.on_hover_text(i18n::t(self.lang, "drag_orbit"));
        let painter = ui.painter_at(well);
        let inner = well.shrink(8.0);
        if self.preview.is_some() && self.mesh.is_none() {
            painter.text(
                well.center(),
                egui::Align2::CENTER_CENTER,
                i18n::t(self.lang, "loading"),
                egui::FontId::proportional(14.0),
                theme::secondary(),
            );
        } else if let (Some(gpu), Some(mesh)) = (self.gpu.clone(), self.mesh.clone()) {
            ui.painter().add(preview3d::paint_callback(
                inner,
                gpu,
                mesh,
                self.yaw,
                self.pitch,
                self.zoom,
                theme::well(),
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
            let msg = if self.native_selected.is_some() || self.selected_item().map(|i| i.model_uuid.is_some()).unwrap_or(false) {
                self.mesh_note
                    .as_deref()
                    .unwrap_or(i18n::t(self.lang, "mesh_fail"))
            } else {
                i18n::t(self.lang, "no_3d")
            };
            let galley = painter.layout(msg.into(), egui::FontId::proportional(13.0), theme::secondary(), (well.width() - 24.0).max(1.0));
            painter.galley(well.center() - galley.size() * 0.5, galley, theme::secondary());
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
        self.search = Some(Promise::spawn_thread("search", move || {
            let mut items = LcedaClient::new().search(&kw)?;
            if lceda_core::models::looks_like_lcsc_token(&kw) {
                items.sort_by_key(|item| !item.lcsc_id().is_some_and(|id| id.eq_ignore_ascii_case(&kw)));
            }
            Ok(items)
        }));
    }

    fn select_search(&mut self, idx: usize) {
        self.selected = Some(idx);
        self.detail = self.items.get(idx).cloned();
        if let Some(item) = &self.detail {
            if let Some(id) = item.lcsc_id() {
                self.cache.insert(id, item.clone());
                self.lib.enrich_from_item(item);
                self.lib.save();
            }
        }
        self.queue_preview();
    }

    fn select_saved(&mut self, lcsc: &str) {
        if self.lib.native(lcsc).is_some(){
            self.native_selected=Some(lcsc.to_string());self.detail=None;self.resolve=None;self.native_unit=1;self.queue_native_preview();return;
        }
        let normalized = if lceda_core::models::looks_like_lcsc_token(lcsc) { lcsc.to_ascii_uppercase() } else { lcsc.to_string() };
        let lcsc = normalized.as_str();
        if let Some(item) = self.cache.get(lcsc).cloned().or_else(|| self.lib.cached_item(lcsc)) {
            self.detail = Some(item);
            self.queue_preview();
            return;
        }
        if self.resolve.is_some() {
            return;
        }
        let q = lcsc.to_string();
        self.resolve = Some(Promise::spawn_thread("resolve", move || {
            let items = LcedaClient::new().search(&q).map_err(|e| e.to_string())?;
            let item = items.into_iter().find(|item| item.lcsc_id().as_deref() == Some(q.as_str()))
                .ok_or_else(|| "没有找到对应编号的器件".to_string())?;
            Ok((q, item))
        }));
    }

    fn ask_export_selected(&mut self) {
        if let Some(id)=self.native_selected.clone(){self.ask_export_ids(vec![id]);return;}
        if self.selected_item().is_none() {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        }
        self.export_ids_pending = Some(Vec::new());
        self.show_batch = true;
    }

    fn ask_export_ids(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            self.alert(i18n::t(self.lang, "batch_none"));
            return;
        }
        if ids.iter().any(|id|self.lib.native(id).is_some()){self.native_export=Some(ids);return;}
        self.export_ids_pending = Some(ids);
        self.show_batch = true;
    }

    fn export_selected(&mut self) {
        let Some(item) = self.selected_item() else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        let has3d = item.model_uuid.is_some();
        let has_ad = item.has_symbol_or_footprint();
        let has_ds = item.datasheet_url().is_some();
        let mut opts = self.batch_opts;
        if !has3d {
            opts.step = false;
            opts.obj = false;
        }
        if !has_ad {
            opts.ad = false;
            opts.kicad = false;
            opts.pads = false;
        }
        if !has_ds {
            opts.datasheet = false;
        }
        if !opts.any() {
            self.alert(i18n::t(self.lang, "batch_none"));
            return;
        }
        self.run_export(|req| {
            req.step = opts.step;
            req.obj = opts.obj;
            req.ad = opts.ad;
            req.kicad = opts.kicad;
            req.pads = opts.pads;
            req.datasheet = opts.datasheet;
            req.source_json = opts.source || opts.ad || opts.kicad || opts.pads;
        });
    }

    fn add_to_queue(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        let Some(part) = part_from_item(&item, UNCAT) else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        if self.lib.add_queue(part) {
            self.lib.save();
        }
    }

    fn add_to_fav(&mut self) {
        if self.selected_item().is_none() {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        }
        self.fav_pick = true;
    }

    fn save_fav(&mut self, cat_id: &str) {
        let Some(item) = self.selected_item().cloned() else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        let Some(part) = part_from_item(&item, cat_id) else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        let mut next = self.lib.clone();
        next.add_fav(part);
        next.enrich_from_item(&item);
        match next.try_save() {
            Ok(()) => {
                self.lib = next;
                self.queue_library_sync(cat_id, None);
                self.log(i18n::t(self.lang, "added_fav"));
            }
            Err(error) => self.alert(format!("保存器件库失败: {error}")),
        }
    }

    fn export_ids(&mut self, ids: Vec<String>) {
        if self.busy() || ids.is_empty() { return; }
        if !self.validate_export_target() { return; }
        let library = self.lib.clone();
        let mut req = self.batch_opts.request(PathBuf::from(self.out_dir.trim()));
        self.apply_export_opts(&mut req);
        req.merge = self.batch_merge;
        self.job = Some(Promise::spawn_thread("export-list", move || {
            let client = LcedaClient::new();
            let mut items = Vec::new();
            let mut errors = Vec::new();
            for id in ids {
                let id = if lceda_core::models::looks_like_lcsc_token(&id) { id.to_ascii_uppercase() } else { id };
                if let Some(item) = library.cached_item(&id) { items.push(item); continue; }
                match client.search(&id) {
                    Ok(found) => {
                        let exact: Vec<_> = found.into_iter().filter(|i| i.lcsc_id().as_deref() == Some(id.as_str()) || i.name().eq_ignore_ascii_case(&id)).collect();
                        if exact.len() == 1 { items.push(exact.into_iter().next().unwrap()); }
                        else { errors.push(format!("FAIL {id}: 请在搜索页或 BOM 工作台确认具体器件")); }
                    }
                    Err(e) => errors.push(format!("FAIL {id}: {e}")),
                }
            }
            let mut report = export_report(&client, &items, &req);
            if !errors.is_empty() { report.message = format!("{}\nResolution failed: {}\n{}", errors.join("\n"), errors.len(), report.message); }
            Ok(report)
        }));
    }

    fn import_queue_file(&mut self) {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("text", &["txt", "csv", "list"])
            .pick_file()
        else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            self.alert(i18n::t(self.lang, "error"));
            return;
        };
        let ids = lceda_core::models::parse_id_list(&text);
        let mut added = 0;
        for id in ids {
            if self.lib.add_queue(SavedPart {
                lcsc: id.clone(),
                name: id,
                manufacturer: String::new(),
                cat_id: UNCAT.into(),
            }) {
                added += 1;
            }
        }
        self.lib.save();
        self.alert(format!("{}: {added}", i18n::t(self.lang, "added_queue")));
    }

    fn show_sponsor_qr(&mut self, ctx: &egui::Context) {
        if !self.sponsor_popup {
            return;
        }
        let mut close = false;
        let title = if self.sponsor_qr_title.is_empty() {
            i18n::t(self.lang, "donate_scan").to_string()
        } else {
            self.sponsor_qr_title.clone()
        };
        let size = 288.0;
        let shown = egui::Window::new(title)
            .id(egui::Id::new("lceda_sponsor_qr"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .default_width(size)
            .min_width(size)
            .max_width(size)
            .frame(
                egui::Frame::new()
                    .fill(theme::fill())
                    .stroke(egui::Stroke::new(1.0_f32, theme::hairline()))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::same(8))
                    .shadow(egui::Shadow::NONE),
            )
            .show(ctx, |ui| {
                ui.set_width(size - 16.0);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                if let Some((_, tex)) = &self.sponsor_qr {
                    let side = size - 16.0;
                    let r = ui.add(egui::Image::new((tex.id(), egui::vec2(side, side))));
                    if r.clicked() {
                        close = true;
                    }
                } else if let Some(err) = &self.sponsor_note {
                    ui.add(egui::Label::new(egui::RichText::new(err).color(theme::secondary())).wrap());
                } else {
                    ui.allocate_ui(egui::vec2(size - 16.0, size - 16.0), |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new(i18n::t(self.lang, "sponsor_loading"))
                                    .color(theme::secondary()),
                            );
                        });
                    });
                }
                if consume_dialog_confirm(ui, true) {
                    close = true;
                }
            });
        if let Some(inner) = shown {
            if self.sponsor_click_guard {
                self.sponsor_click_guard = false;
            } else if inner.response.clicked_elsewhere() {
                close = true;
            }
        }
        if close {
            self.sponsor_popup = false;
        }
    }

    fn show_fav_pick(&mut self, ctx: &egui::Context) {
        if !self.fav_pick {
            return;
        }
        let mut close = false;
        let mut chosen: Option<String> = None;
        let cats: Vec<(String, String)> = self
            .lib
            .cats
            .iter()
            .map(|c| {
                let name = if c.id == UNCAT {
                    i18n::t(self.lang, "uncategorized").to_string()
                } else {
                    self.lib.cat_path(&c.id)
                };
                (c.id.clone(), name)
            })
            .collect();
        let lang = self.lang;
        let width = 360.0;
        fit_window(i18n::t(lang, "pick_cat"), "lceda_fav_pick", width).show(ctx, |ui| {
            ui.set_width(width - 8.0);
            if let Some(item) = self.selected_item() {
                ui.add(egui::Label::new(theme::heading(item.name())).truncate());
            }
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for (id, name) in &cats {
                    if theme::pill_button(ui, name, true, false).clicked() {
                        chosen = Some(id.clone());
                    }
                }
            });
            ui.add_space(8.0);
            if theme::pill_button(ui, i18n::t(lang, "batch_cancel"), true, false).clicked() {
                close = true;
            }
            close |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        });
        if let Some(id) = chosen {
            self.save_fav(&id);
            self.fav_pick = false;
        }
        if close {
            self.fav_pick = false;
        }
    }

    fn show_cat_dialog(&mut self, ctx: &egui::Context) {
        let creating = self.show_new_cat || self.show_import_cat;
        if !creating && !self.show_rename_cat {
            return;
        }
        let mut close = false;
        let mut ok = false;
        let lang = self.lang;
        let title = if self.show_import_cat {
            i18n::t(lang, "import_cat")
        } else if self.show_rename_cat {
            i18n::t(lang, "rename_cat")
        } else {
            i18n::t(lang, "new_cat")
        };
        let width = 360.0;
        fit_window(title, "lceda_cat_dlg", width).show(ctx, |ui| {
            ui.set_width(width - 8.0);
            if self.show_import_cat {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(i18n::t(lang, "import_cat_hint")).color(theme::secondary()),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
            }
            ui.add_sized(
                egui::vec2(ui.available_width(), 30.0),
                theme::edit_single(&mut self.new_cat_name, i18n::t(lang, "cat_name")),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::pill_button(ui, i18n::t(lang, "create"), true, true).clicked() {
                        ok = true;
                    }
                    if theme::pill_button(ui, i18n::t(lang, "batch_cancel"), true, false).clicked() {
                        close = true;
                    }
                });
            });
            if consume_dialog_confirm(ui, false) {
                ok = true;
            }
        });
        if ok {
            if self.show_rename_cat {
                self.lib.rename_cat(&self.fav_cat.clone(), &self.new_cat_name);
                self.lib.save();
            } else if let Some(id) = self.lib.add_root(&self.new_cat_name) {
                self.fav_cat = id.clone();
                if self.show_import_cat {
                    self.import_ids_to_fav(&id);
                }
                self.lib.save();
            }
            self.new_cat_name.clear();
            self.show_new_cat = false;
            self.show_rename_cat = false;
            self.show_import_cat = false;
        }
        if close {
            self.show_new_cat = false;
            self.show_rename_cat = false;
            self.show_import_cat = false;
            self.new_cat_name.clear();
        }
    }

    fn import_ids_to_fav(&mut self, cat_id: &str) {
        let Some(file) = rfd::FileDialog::new()
            .add_filter("text", &["txt", "csv", "list"])
            .pick_file()
        else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            self.alert(i18n::t(self.lang, "error"));
            return;
        };
        for id in lceda_core::models::parse_id_list(&text) {
            self.lib.add_fav(SavedPart {
                lcsc: id.clone(),
                name: id,
                manufacturer: String::new(),
                cat_id: cat_id.into(),
            });
        }
    }

    fn dialog_open(&self) -> bool {
        self.alert.is_some()
            || self.show_welcome
            || self.show_detail
            || self.show_batch
            || self.sponsor_popup
            || self.fav_pick
            || self.show_new_cat
            || self.show_rename_cat
            || self.show_import_cat
            || self.update.is_some()
            || self.show_activity
            || self.show_library_sync
            || self.confirm_clear_queue
            || self.show_bom_import
            || self.bind_cat.is_some()
            || self.native_import.is_some()
            || self.native_load.is_some()
            || self.native_save.is_some()
            || self.native_export.is_some()
            || self.resource_assist.is_some()
    }

    fn queue_preview(&mut self) {
        self.native_selected=None;self.native_scene=None;
        let Some(item) = self.selected_item().cloned() else { return; };
        self.image_tex = None;
        self.mesh = None;
        self.mesh_tex = None;
        self.mesh_note = None;
        self.symbol = None;
        self.symbol_note = None;
        self.symbol_view.reset();
        self.footprint = None;
        self.footprint_note = None;
        self.footprint_view.reset();
        self.yaw = 0.7;
        self.pitch = 0.55;
        self.zoom = 1.0;
        let id = item.lcsc_id();
        let record = id.as_ref().and_then(|id| self.lib.record(id)).cloned().unwrap_or_default();
        let desc_fields = self.desc_fields.clone();
        self.preview = Some(Promise::spawn_thread("preview", move || {
            let client = LcedaClient::new();
            let mut data = PreviewData { native_scene:None,lcsc: id.clone(), files: Vec::new(), image: None, mesh: None, mesh_note: None, symbol: None, symbol_note: None, footprint: None, footprint_note: None };
            let cache_dir = if env::var_os("LCEDA_SHOT").is_some() {
                crate::profile_dir().map(|d| d.join("cache"))
            } else {
                directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant").map(|d| d.cache_dir().to_path_buf())
            }.map(|d| d.join("preview").join(id.as_deref().unwrap_or("unknown")));
            if let Some(path) = record.resources.get("preview_image") {
                data.image = std::fs::read(path).ok();
            }
            if data.image.is_none() {
                for url in item.image_urls() {
                    if let Ok(bytes) = client.get_bytes(&url) {
                        if image::load_from_memory(&bytes).is_err() { continue; }
                        if let Some(dir) = &cache_dir {
                            if std::fs::create_dir_all(dir).is_ok() {
                                let path = dir.join("photo.image");
                                if std::fs::write(&path, &bytes).is_ok() { data.files.push(("preview_image".into(), path)); }
                            }
                        }
                        data.image = Some(bytes); break;
                    }
                }
            }
            if let Some(uuid) = item.symbol_uuid() {
                let cached = record.resources.get("symbol_json").or_else(|| record.resources.get("preview_symbol"))
                    .and_then(|p| std::fs::read(p).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok());
                let json = cached.map(Ok).unwrap_or_else(|| client.component_json(&uuid));
                match json {
                    Ok(json) => {
                        if let Some(dir) = &cache_dir {
                            if std::fs::create_dir_all(dir).is_ok() {
                                let path = dir.join("symbol.json");
                                if let Ok(bytes) = serde_json::to_vec(&json) { if std::fs::write(&path, bytes).is_ok() { data.files.push(("preview_symbol".into(), path)); } }
                            }
                        }
                        match lceda_core::export::symbol_for_item(&item, item.name(), &json, &desc_fields) {
                            Ok(symbol) => data.symbol = Some(symbol),
                            Err(error) => data.symbol_note = Some(format!("原理图预览: {error}")),
                        }
                    },
                    Err(error) => data.symbol_note = Some(format!("原理图预览: {error}")),
                }
            }
            if let Some(uuid) = item.footprint_uuid() {
                let cached = record.resources.get("footprint_json").or_else(|| record.resources.get("preview_footprint"))
                    .and_then(|p| std::fs::read(p).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok());
                let json = cached.map(Ok).unwrap_or_else(|| client.component_json(&uuid));
                match json {
                    Ok(json) => {
                        if let Some(dir) = &cache_dir {
                            if std::fs::create_dir_all(dir).is_ok() {
                                let path = dir.join("footprint.json");
                                if let Ok(bytes) = serde_json::to_vec(&json) { if std::fs::write(&path, bytes).is_ok() { data.files.push(("preview_footprint".into(), path)); } }
                            }
                        }
                        match lceda_core::easyeda::parse_footprint(&json) {
                            Ok(source) => data.footprint = Some(lceda_core::ir::footprint_ir(item.name(), "", source, Default::default())),
                            Err(error) => data.footprint_note = Some(format!("封装预览: {error}")),
                        }
                    },
                    Err(error) => data.footprint_note = Some(format!("封装预览: {error}")),
                }
            }
            let local_obj = record.resources.get("obj").or_else(|| record.resources.get("preview_obj"));
            if let Some(path) = local_obj {
                data.mesh = std::fs::read(path).ok().and_then(|b| load_preview_mesh(&b).ok());
            }
            if data.mesh.is_none() && item.model_uuid.is_some() {
                match client.download_obj_bytes(&item) {
                    Ok(bytes) => match load_preview_mesh(&bytes) {
                        Ok(mesh) => {
                            if let Some(dir) = &cache_dir {
                                if std::fs::create_dir_all(dir).is_ok() {
                                    let path = dir.join("preview.obj");
                                    if std::fs::write(&path, &bytes).is_ok() { data.files.push(("preview_obj".into(), path)); }
                                }
                            }
                            data.mesh = Some(mesh);
                        },
                        Err(note) => data.mesh_note = Some(note),
                    },
                    Err(e) => data.mesh_note = Some(format!("3D: {e}")),
                }
            }
            data
        }));
    }


    fn run_export(&mut self, mutate: impl FnOnce(&mut ExportRequest)) {
        if self.busy() { return; }
        let Some(item) = self.selected_item().cloned() else {
            self.alert(i18n::t(self.lang, "select_first"));
            return;
        };
        let out_dir = PathBuf::from(self.out_dir.trim());
        let mut req = ExportRequest {
            force: true,
            out_dir: out_dir.clone(),
            ..Default::default()
        };
        mutate(&mut req);
        if !self.validate_export_target() { return; }
        self.apply_export_opts(&mut req);
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.alert(format!("{}: {e}", i18n::t(self.lang, "error")));
            return;
        }
        self.log(format!("{}  {}", i18n::t(self.lang, "saving_to"), out_dir.display()));
        self.job = Some(Promise::spawn_thread("export", move || {
            let client = LcedaClient::new();
            let paths = export(&client, &item, &req)?;
            Ok(JobReport { message: format_paths(&paths, &out_dir), exported: vec![(item, paths)], notify: true, failed: 0 })
        }));
    }

    fn show_title_bar(&mut self, ctx: &egui::Context) {
        let dark = self.want_dark(ctx);
        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let mut persist = false;
        let mut theme_changed = false;
        let mut pin_changed = false;
        egui::TopBottomPanel::top("caption")
            .exact_height(36.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::fill())
                    .corner_radius(title_bar_radius(maxed))
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ctx, |ui| {
                if self.brand_tex.is_none() {
                    self.brand_tex = load_named_texture(ui.ctx(), "brand", ICON_PNG);
                }
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.horizontal(|ui| {
                    ui.set_height(32.0);
                    if let Some(tex) = &self.brand_tex {
                        ui.add(
                            egui::Image::new((tex.id(), egui::vec2(18.0, 18.0)))
                                .fit_to_exact_size(egui::vec2(18.0, 18.0)),
                        );
                    }
                    ui.label(
                        egui::RichText::new(i18n::t(self.lang, "app_title"))
                            .color(theme::label())
                            .size(13.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("v{}", update::current_version()))
                            .color(theme::secondary())
                            .size(11.0),
                    );
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let controls = 40.0 + 32.0 * 4.0 + 8.0;
                    let drag_w = (ui.available_width() - controls).max(16.0);
                    let (_, drag) = ui.allocate_exact_size(egui::vec2(drag_w, 32.0), egui::Sense::click_and_drag());
                    if drag.drag_started()
                        && resize_dir_at(
                            ui.ctx().screen_rect(),
                            drag.interact_pointer_pos().unwrap_or(drag.rect.center()),
                            RESIZE_EDGE,
                            RESIZE_CORNER,
                        )
                        .is_none()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag.double_clicked() {
                        let maxed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                    }
                    if theme::theme_switch(ui, dark)
                        .on_hover_text(i18n::t(self.lang, "theme_toggle"))
                        .clicked()
                    {
                        self.theme = if dark {
                            ThemeMode::Light
                        } else {
                            ThemeMode::Dark
                        };
                        theme_changed = true;
                        persist = true;
                    }
                    let tip = if self.always_on_top {
                        i18n::t(self.lang, "win_unpin")
                    } else {
                        i18n::t(self.lang, "win_pin")
                    };
                    if theme::pin_button(ui, self.always_on_top)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.always_on_top = !self.always_on_top;
                        pin_changed = true;
                        persist = true;
                    }
                    if theme::caption_min(ui)
                        .on_hover_text(i18n::t(self.lang, "win_min"))
                        .clicked()
                    {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    let max_tip = if maxed {
                        i18n::t(self.lang, "win_restore")
                    } else {
                        i18n::t(self.lang, "win_max")
                    };
                    if theme::caption_max(ui, maxed)
                        .on_hover_text(max_tip)
                        .clicked()
                    {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
                    }
                    if theme::caption_close(ui)
                        .on_hover_text(i18n::t(self.lang, "win_close"))
                        .clicked()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        if theme_changed {
            self.dark_applied = None;
        }
        if pin_changed {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
        }
        if persist {
            self.persist_prefs();
        }
    }

    fn persist_prefs(&self) {
        crate::prefs::save(&crate::prefs::Prefs {
            ad_embed_3d: self.ad_embed_3d,
            kicad_attach_3d: self.kicad_attach_3d,
            rename_footprint: self.rename_footprint,
            batch_merge: self.batch_merge,
            preview_mode: self.preview_mode,
            hide_welcome: self.hide_welcome,
            lang: if self.lang_pinned {
                Some(self.lang.as_code().into())
            } else {
                None
            },
            theme: self.theme,
            always_on_top: self.always_on_top,
            out_dir: Some(self.out_dir.clone()).filter(|s| !s.is_empty()),
            export_step: self.batch_opts.step,
            export_obj: self.batch_opts.obj,
            export_ad: self.batch_opts.ad,
            export_kicad: self.batch_opts.kicad,
            export_pads: self.batch_opts.pads,
            export_datasheet: self.batch_opts.datasheet,
            export_source: self.batch_opts.source,
            win_x: None,
            win_y: None,
            win_w: self.win_w,
            win_h: self.win_h,
            win_max: self.win_max,
            parts_width: self.parts_width,
            photo_frac: self.photo_frac,
            top_frac: self.top_frac,
            sch_scheme: self.sch_scheme,
            sch_custom: self.sch_custom,
            settings_tab: self.settings_tab,
            export_section: self.export_section,
            desc_fields: self.desc_fields.clone(),
            nav_expanded: self.nav_expanded,
        });
    }

    fn resolved_sch_colors(&self) -> SchColors {
        self.sch_scheme.colors(self.sch_custom)
    }

    fn validate_export_target(&mut self) -> bool {
        if self.out_dir.trim().is_empty() { self.alert(i18n::t(self.lang, "choose_output")); return false; }
        true
    }

    fn apply_export_opts(&self, req: &mut ExportRequest) {
        req.ad_embed_3d = self.ad_embed_3d;
        req.kicad_attach_3d = self.kicad_attach_3d;
        req.rename_footprint = self.rename_footprint;
        req.sch_colors = self.resolved_sch_colors();
        req.desc_fields = self.desc_fields.clone();
        req.ad_library_dir = None;
        req.sync_bindings = false;
        for part in &self.lib.favs {
            if let Some(path) = self.lib.binding_for_category(&part.cat_id) { req.ad_library_files.insert(part.lcsc.to_ascii_uppercase(), path.to_path_buf()); }
            if let Some(path) = self.lib.binding_for(&part.cat_id, LibraryFormat::KiCad) { req.kicad_library_files.insert(part.lcsc.to_ascii_uppercase(), path.to_path_buf()); }
        }
    }

    fn persist_window(&mut self, ctx: &egui::Context) {
        let (inner, maxed) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect, v.maximized.unwrap_or(false))
        });
        let mut dirty = false;
        if self.win_max != maxed {
            self.win_max = maxed;
            dirty = true;
        }
        if !maxed {
            if let Some(inner) = inner {
                if inner.width() >= 960.0 && inner.height() >= 620.0 {
                    let w = inner.width().min(4000.0);
                    let h = inner.height().min(3000.0);
                    if (self.win_w - w).abs() > 4.0 || (self.win_h - h).abs() > 4.0 {
                        self.win_w = w;
                        self.win_h = h;
                        dirty = true;
                    }
                }
            }
        }
        if dirty {
            self.persist_prefs();
        }
    }

    #[allow(dead_code)]

    fn tick_debug_shot(&mut self, ctx: &egui::Context) {
        if let Some(kw) = self.pending_search.take() {
            self.keyword = kw;
            self.do_search();
        }
        if self.shot_path.is_none() {
            return;
        }
        self.apply_shot_camera();
        if !self.desc_preview_kw.trim().is_empty()
            && self.desc_preview_item.is_none()
            && self.desc_preview_job.is_none()
            && self.desc_preview_note.is_none()
        {
            self.start_desc_preview();
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

        let jobs_idle = self.search.is_none()
            && self.preview.is_none()
            && self.job.is_none()
            && self.native_load.is_none()
            && self.native_save.is_none()
            && self.resource_assist.as_ref().is_none_or(|s|!s.is_busy())
            && self.desc_preview_job.is_none();
        let preview_ready =
            self.mesh.is_some() || self.mesh_note.is_some() || self.image_tex.is_some();
        let want_batch = env::var("LCEDA_BATCH").is_ok();
        let want_detail = env::var("LCEDA_DETAIL").is_ok();
        let want_library_picker = env::var("LCEDA_PICK_LIBRARY").is_ok();
        if jobs_idle && self.selected_item().is_some() {
            if want_batch && !self.show_batch {
                self.show_batch = true;
            }
            if want_detail && !self.show_detail {
                self.show_detail = true;
            }
            if want_library_picker && !self.fav_pick {
                self.add_to_fav();
            }
        }
        let desc_ready = self.desc_preview_kw.trim().is_empty()
            || self.desc_preview_item.is_some()
            || self.desc_preview_note.is_some();
        let search_ready = self.keyword.is_empty() || (jobs_idle && preview_ready);
        let dialog_ready = (!want_batch || self.show_batch) && (!want_detail || self.show_detail)
            && (!want_library_picker || self.fav_pick);
        let waited = self.frame >= 40 && jobs_idle && desc_ready && search_ready && dialog_ready;
        if waited {
            self.shot_settle = self.shot_settle.saturating_add(1);
        }
        if self.shot_settle >= 18 && !self.shot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.shot_requested = true;
        }
        if self.frame > 3600 {
            eprintln!("LCEDA_SHOT timed out");
            std::process::exit(3);
        }
    }

    fn apply_shot_camera(&mut self) {
        if let Some(v) = env_f32("LCEDA_YAW") {
            self.yaw = v;
        }
        if let Some(v) = env_f32("LCEDA_PITCH") {
            self.pitch = v;
        }
        if let Some(v) = env_f32("LCEDA_ZOOM") {
            self.zoom = v;
        }
    }
}

impl App {
    fn import_bom(&mut self) {
        if self.busy() { return; }
        let Some(path) = rfd::FileDialog::new().add_filter("BOM", &["csv", "tsv", "xlsx", "xls", "xlsb", "ods"]).pick_file() else { return; };
        self.goto(NavPage::Favorites);
        self.show_bom_import = true;
        *self.bom_progress.lock().unwrap() = (0, 0);
        self.bom_cancel.store(false, Ordering::SeqCst);
        self.bom_job = Some(Promise::spawn_thread("bom-import", move || BomDocument::load(&path)));
    }

    fn match_bom(&mut self) {
        if self.busy() { return; }
        let Some(mut doc) = self.bom.clone() else { return; };
        let library = self.lib.clone();
        self.bom_cancel.store(false, Ordering::SeqCst);
        let cancel = self.bom_cancel.clone();
        let progress = self.bom_progress.clone();
        *progress.lock().unwrap() = (0, doc.rows.len());
        self.bom_job = Some(Promise::spawn_thread("bom-match", move || {
            doc.resolve_with_control(&LcedaClient::new(), &library, &cancel, |done, total| {
                *progress.lock().unwrap() = (done, total);
            });
            Ok(doc)
        }));
    }

    fn bom_import_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_bom_import { return; }
        let width = (ctx.screen_rect().width() - 72.0).min(1040.0).max(740.0);
        let height = (ctx.screen_rect().height() - 132.0).max(380.0);
        let mut open = true;
        egui::Window::new(i18n::t(self.lang, "import_bom")).id(egui::Id::new("library_bom_import"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground)
            .collapsible(false).resizable(true).open(&mut open)
            .default_size(egui::vec2(width, height)).min_width(740.0)
            .max_size(egui::vec2(width, height)).frame(theme::card_frame())
            .show(ctx, |ui| {
                ui.set_width(width - 24.0);
                ui.set_min_height(height - 30.0);
                ui.horizontal(|ui| {
                    ui.label(i18n::t(self.lang, "bom_target_library"));
                    let current = if self.fav_cat == UNCAT { i18n::t(self.lang, "uncategorized").to_string() }
                        else { self.lib.cat(&self.fav_cat).map(|c| c.name.clone()).unwrap_or_default() };
                    ui.add_enabled_ui(!self.busy(), |ui| {
                        egui::ComboBox::from_id_salt("bom_target_library").selected_text(current).show_ui(ui, |ui| {
                            for category in &self.lib.cats {
                                let label = if category.id == UNCAT { i18n::t(self.lang, "uncategorized") } else { &category.name };
                                ui.selectable_value(&mut self.fav_cat, category.id.clone(), label);
                            }
                        });
                    });
                });
                ui.add_space(6.0);
                ui.add_enabled_ui(!self.show_detail && self.alert.is_none(), |ui| self.page_bom(ui));
            });
        if !open || (!self.show_detail && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))) {
            self.show_bom_import = false;
            self.bom_cancel.store(true, Ordering::SeqCst);
        }
    }

    fn bind_library_dialog(&mut self, ctx: &egui::Context) {
        let Some(cat_id) = self.bind_cat.clone() else { return; };
        let lang = self.lang;
        let mut open = true;
        let mut bind = false;
        let mut cancel = false;
        let format = self.bind_format;
        let extension = format.extension();
        let name = if cat_id == UNCAT { i18n::t(lang, "uncategorized").to_string() }
            else { self.lib.cat(&cat_id).map(|c| c.name.clone()).unwrap_or_default() };
        let title = format!("{} · {}", i18n::t(lang, "bind_library"), format.name());
        fit_window(&title, "bind_library_dialog", 600.0).open(&mut open).show(ctx, |ui| {
            ui.label(theme::heading(name));
            ui.add(egui::Label::new(i18n::t(lang, if format == LibraryFormat::Ad { "bind_library_hint" } else { "bind_kicad_hint" })).wrap());
            ui.add_space(10.0);
            ui.add_sized([ui.available_width(), 34.0], theme::edit_single(&mut self.bind_path, &format!("Library.{extension}")));
            let path = PathBuf::from(self.bind_path.trim());
            if format == LibraryFormat::KiCad {
                if let Ok(paths) = lceda_core::bound_kicad::targets(&path) {
                    for path in [&paths.pretty, &paths.shapes] {
                        ui.add(egui::Label::new(egui::RichText::new(path.display().to_string()).small().color(theme::secondary())).truncate()).on_hover_text(path.display().to_string());
                    }
                }
            } else {
                let paired = path.with_extension("PcbLib").display().to_string();
                ui.add(egui::Label::new(egui::RichText::new(&paired).small().color(theme::secondary())).truncate()).on_hover_text(paired);
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button(i18n::t(lang, "choose_library_file")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter(format.name(), &[extension]).pick_file() { self.bind_path = path.display().to_string(); }
                }
                if ui.button(i18n::t(lang, "new_library_file")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter(format.name(), &[extension]).set_file_name(format!("Library.{extension}")).save_file() { self.bind_path = path.display().to_string(); }
                }
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                cancel = theme::pill_button(ui, i18n::t(lang, "cancel"), true, false).clicked();
                bind = theme::pill_button(ui, i18n::t(lang, "bind_and_sync"), !self.busy() && !self.bind_path.trim().is_empty(), true).clicked();
            });
        });
        if bind {
            let path = PathBuf::from(self.bind_path.trim());
            self.begin_binding_validation(cat_id, format, path);
        }
        if cancel || !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.bind_cat = None; }
    }

    fn start_library_sync(&mut self) {
        self.library_sync_after = None;
        let categories = std::mem::take(&mut self.pending_library_sync);
        let mut req = ExportRequest { ad: true, kicad: true, bound_only: true, source_json: true, out_dir: PathBuf::from(self.out_dir.trim()), ..Default::default() };
        self.apply_export_opts(&mut req);
        req.sync_bindings = true;
        req.ad_library_dir = None;
        req.ad_library_files.clear();
        req.kicad_library_files.clear();
        let mut ids = BTreeSet::new();
        for (category, format) in &categories {
            if let Some(path) = self.lib.binding_for(category, *format) {
                for part in self.lib.favs_in(category) {
                    if self.lib.native(&part.lcsc).is_some(){continue;}
                    let id = part.lcsc.to_ascii_uppercase();
                    match format {
                        LibraryFormat::Ad => { req.ad_library_files.insert(id.clone(), path.to_path_buf()); }
                        LibraryFormat::KiCad => { req.kicad_library_files.insert(id.clone(), path.to_path_buf()); }
                    }
                    ids.insert(id);
                }
            }
        }
        self.launch_library_sync(categories, req, ids);
    }

    fn queue_library_sync(&mut self, category: &str, format: Option<LibraryFormat>) {
        for candidate in LibraryFormat::ALL {
            if format.is_none_or(|selected| selected == candidate) && self.lib.binding_for(category, candidate).is_some() {
                self.pending_library_sync.insert((category.to_string(), candidate));
            }
        }
    }

    fn handle_library_action(&mut self, action: LibraryAction) {
        match action {
            LibraryAction::ShowProgress => { self.show_library_sync = true; }
            LibraryAction::Bind(id, format) => {
                self.bind_path = self.lib.binding_for(&id, format).map(|p| p.display().to_string()).unwrap_or_else(|| {
                    let name = if id == UNCAT { i18n::t(self.lang, "uncategorized") }
                        else { self.lib.cat(&id).map(|c| c.name.as_str()).unwrap_or("Library") };
                    PathBuf::from(&self.out_dir).join(format!("{}.{}", lceda_core::util::sanitize_filename(name), format.extension())).display().to_string()
                });
                self.bind_format = format;
                self.bind_cat = Some(id);
            }
            LibraryAction::Unbind(id, format) => {
                let mut next = self.lib.clone();
                match next.clear_binding(&id, format) {
                    Ok(()) => match next.try_save() {
                        Ok(()) => { self.lib = next; self.pending_library_sync.remove(&(id, format)); }
                        Err(error) => self.alert(format!("保存库绑定失败: {error}")),
                    },
                    Err(error) => self.alert(error),
                }
            }
            LibraryAction::Sync(id, format) => { self.queue_library_sync(&id, format); self.library_sync_after = None; self.show_library_sync = true; }
            LibraryAction::SyncAll => {
                let ids: Vec<_> = self.lib.bound_categories().iter().map(|(c, _)| c.id.clone()).collect();
                for id in ids { self.queue_library_sync(&id, None); }
                self.library_sync_after = None;
                self.show_library_sync = true;
            }
        }
    }

    fn page_bom(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let busy = self.busy();
        let mut import = false;
        let mut resolve = false;
        let mut export = false;
        let mut preview = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::heading(i18n::t(lang, "bom_import_review")).size(20.0));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                import = theme::pill_button(ui, i18n::t(lang, "import_bom"), !busy, self.bom.is_none()).clicked();
            });
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t(lang, "bom_hint")).color(theme::secondary()).size(12.0));
        ui.add_space(8.0);
        if let Some(doc) = &mut self.bom {
            let confirmed = doc.rows.iter().filter(|r| r.included && r.confirmed).count();
            let included = doc.rows.iter().filter(|r| r.included).count();
            theme::show_card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Label::new(theme::heading(&doc.source)).truncate()).on_hover_text(&doc.source);
                    theme::badge(ui, &format!("{confirmed} / {included} {}", i18n::t(lang, "bom_confirmed")), confirmed == included && included > 0);
                    if self.bom_job.is_some() {
                        let (done, total) = *self.bom_progress.lock().unwrap();
                        ui.spinner();
                        ui.label(format!("{done} / {total}"));
                        if total > 0 && ui.button(i18n::t(lang, "bom_cancel")).clicked() { self.bom_cancel.store(true, Ordering::SeqCst); }
                        if self.bom_cancel.load(Ordering::SeqCst) { ui.label(i18n::t(lang, "bom_stopping")); }
                    } else {
                        resolve = theme::pill_button(ui, i18n::t(lang, if doc.rows.iter().any(|r| !r.candidates.is_empty()) { "bom_retry" } else { "match_bom" }), !busy, confirmed == 0).clicked();
                        if theme::pill_button(ui, i18n::t(lang, "bom_confirm_exact"), !busy, false)
                            .on_hover_text(i18n::t(lang, "bom_confirm_hint")).clicked() {
                            doc.confirm_exact_ids();
                        }
                    }
                    export = theme::pill_button(ui, i18n::t(lang, "bom_add_library"), !busy && confirmed > 0 && confirmed == included, true)
                        .on_disabled_hover_text(i18n::t(lang, "bom_confirm_hint")).clicked();
                });
            });
            for warning in doc.warnings.iter().take(3) {
                ui.label(egui::RichText::new(warning).color(theme::secondary()).small());
            }
            ui.add_space(12.0);
            egui::ScrollArea::horizontal().id_salt("bom_horizontal").show(ui, |ui| {
                ui.set_min_width(730.0);
                let height = ui.available_height().max(80.0);
                egui_extras::TableBuilder::new(ui)
                    .id_salt("bom_table")
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::top_down(egui::Align::Min))
                    .column(egui_extras::Column::exact(55.0))
                    .column(egui_extras::Column::initial(180.0).at_least(140.0))
                    .column(egui_extras::Column::exact(48.0))
                    .column(egui_extras::Column::remainder().at_least(270.0))
                    .column(egui_extras::Column::exact(90.0))
                    .min_scrolled_height(80.0)
                    .max_scroll_height((height - 36.0).max(80.0))
                    .header(30.0, |mut header| {
                        for key in ["bom_exclude", "bom_input", "bom_qty", "bom_candidate", "bom_state"] {
                            header.col(|ui| { ui.label(theme::heading(i18n::t(lang, key))); });
                        }
                    })
                    .body(|body| {
                        body.rows(94.0, doc.rows.len(), |mut table_row| {
                            let index = table_row.index();
                            let row = &mut doc.rows[index];
                            table_row.col(|ui| {
                                ui.add_enabled(!busy, egui::Checkbox::without_text(&mut row.included));
                                ui.label(egui::RichText::new(format!("#{}", row.line)).small().color(theme::secondary()));
                            });
                            table_row.col(|ui| {
                                ui.add(egui::Label::new(theme::heading(if row.model.is_empty() { &row.lcsc } else { &row.model })).truncate()).on_hover_text(&row.model);
                                ui.add(egui::Label::new(egui::RichText::new(format!("{}  {}  {}", row.lcsc, row.manufacturer, row.package)).small().color(theme::secondary())).truncate());
                                ui.add(egui::Label::new(egui::RichText::new(&row.designators).small()).truncate()).on_hover_text(&row.designators);
                            });
                            table_row.col(|ui| { ui.label(row.quantity.to_string()); });
                            table_row.col(|ui| {
                                ui.add_enabled_ui(!busy && row.included && !row.validation_error, |ui| {
                                    let caption = row.chosen().map(|i| format!("{}  ·  {}", i.lcsc_id().unwrap_or_default(), i.name()))
                                        .unwrap_or_else(|| i18n::t(lang, "bom_choose").into());
                                    egui::ComboBox::from_id_salt(("bom_candidate", index)).width(ui.available_width().min(390.0))
                                        .selected_text(caption).show_ui(ui, |ui| {
                                            ui.set_max_width(570.0);
                                            let candidates = row.candidates.clone();
                                            for (candidate_index, item) in candidates.iter().enumerate() {
                                                let label = format!("{}  {}\n{}  ·  {}", item.lcsc_id().unwrap_or_default(), item.name(), item.manufacturer_label(), crate::library::package_from_item(item));
                                                if ui.selectable_label(row.selected == Some(candidate_index), label).clicked() { row.choose(candidate_index); }
                                            }
                                        });
                                    if let Some(chosen) = row.chosen().cloned() {
                                        ui.label(egui::RichText::new(format!("{}  ·  {}", chosen.manufacturer_label(), crate::library::package_from_item(&chosen))).small().color(theme::secondary()));
                                        ui.horizontal(|ui| {
                                            if ui.small_button(i18n::t(lang, "details")).clicked() { preview = Some(chosen.clone()); }
                                            if !row.confirmed && ui.small_button(i18n::t(lang, "bom_confirm_choice")).clicked() {
                                                if let Some(index) = row.selected { row.choose(index); }
                                            }
                                        });
                                    }
                                });
                            });
                            table_row.col(|ui| {
                                if let Some(error) = &row.error { ui.label(egui::RichText::new(i18n::t(lang, "bom_error_state")).size(11.5).color(Color32::from_rgb(194, 87, 49))).on_hover_text(error); }
                                else { theme::badge(ui, i18n::t(lang, if !row.included { "bom_excluded" } else if row.confirmed { "bom_confirmed" } else if row.candidates.is_empty() { "bom_unmatched" } else { "bom_unconfirmed" }), row.included && row.confirmed); }
                            });
                        });
                    });
            });
        } else {
            ui.add_space(50.0);
            theme::show_card(ui, |ui| {
                ui.add_space(25.0);
                ui.vertical_centered(|ui| {
                    ui.label(theme::heading(i18n::t(lang, "bom_empty")).size(22.0));
                    ui.add_space(10.0);
                    ui.label(i18n::t(lang, "bom_empty_hint"));
                    ui.add_space(18.0);
                    import |= theme::action_button(ui, i18n::t(lang, "import_bom"), !busy, true, egui::vec2(180.0, 40.0)).clicked();
                });
                ui.add_space(25.0);
            });
        }
        if import { self.import_bom(); }
        if resolve { self.match_bom(); }
        if let Some(item) = preview { self.detail = Some(item); self.show_detail = true; }
        if export {
            if let Some(doc) = &self.bom {
                match doc.confirmed_parts() {
                    Ok(parts) => {
                        let mut items = Vec::new();
                        for row in doc.rows.iter().filter(|r| r.included && r.confirmed) {
                            if let Some(item) = row.chosen() {
                                if !items.iter().any(|i: &SearchItem| i.lcsc_id() == item.lcsc_id()) { items.push(item.clone()); }
                            }
                        }
                        let mut next = self.lib.clone();
                        for item in &items { next.enrich_from_item(item); }
                        for mut part in parts { part.cat_id = self.fav_cat.clone(); next.add_fav(part); }
                        let mut totals = std::collections::BTreeMap::<String, (u64, Vec<String>)>::new();
                        for row in doc.rows.iter().filter(|r| r.included && r.confirmed) {
                            if let Some(id) = row.chosen().and_then(SearchItem::lcsc_id) {
                                let entry = totals.entry(id).or_default();
                                entry.0 += u64::from(row.quantity);
                                if !row.designators.is_empty() { entry.1.push(row.designators.clone()); }
                            }
                        }
                        for (id, (quantity, references)) in totals {
                            let record = next.records.entry(id).or_default();
                            record.parameters.insert("BOM数量".into(), quantity.to_string());
                            record.parameters.insert("BOM位号".into(), references.join(","));
                            record.parameters.insert("BOM源文件".into(), doc.source.clone());
                        }
                        match next.try_save() {
                            Ok(()) => {
                                self.lib = next;
                                self.queue_library_sync(&self.fav_cat.clone(), None);
                                self.show_bom_import = false;
                                self.fav_inside = true;
                                self.goto(NavPage::Favorites);
                            }
                            Err(error) => self.alert(format!("导入器件库失败: {error}")),
                        }
                    }
                    Err(error) => self.alert(error),
                }
            }
        }
    }

    fn keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog_open() { return; }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.goto(NavPage::Search);
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("mid_search")));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) { self.import_bom(); }
        if !self.busy() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) { self.ask_export_selected(); }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let mut folder = false;
        egui::TopBottomPanel::bottom("workstation_status").exact_height(36.0)
            .frame(egui::Frame::new().fill(theme::fill()).inner_margin(egui::Margin::symmetric(16, 4)))
            .show(ctx, |ui| {
                ui.spacing_mut().interact_size.y = 24.0;
                ui.spacing_mut().button_padding = egui::vec2(9.0, 3.0);
                // Reserve the action first so long paths cannot push it beyond the window edge.
                ui.allocate_ui_with_layout(ui.available_size(), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new(i18n::t(self.lang, "activity")).size(11.5))
                        .wrap_mode(egui::TextWrapMode::Extend).min_size(egui::vec2(0.0, 24.0)).corner_radius(5)).clicked() {
                        self.show_activity = true;
                    }
                    if let Some(task) = &self.library_sync_task {
                        let (done, total) = task.status();
                        let text = if task.running() { format!("{} {done}/{total}", i18n::t(self.lang, "sync_progress")) }
                            else { i18n::t(self.lang, "sync_result").to_string() };
                        if ui.add(egui::Button::new(egui::RichText::new(text).size(11.5))
                            .wrap_mode(egui::TextWrapMode::Extend).min_size(egui::vec2(0.0, 24.0))).clicked() { self.show_library_sync = true; }
                    }
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if self.busy() { ui.add(egui::Spinner::new().size(14.0)); }
                        let state = if self.bind_job.is_some() { "sync_validating" } else if self.library_sync_job { "sync_running" } else if self.job.is_some() { "working" } else if self.search.is_some() || self.resolve.is_some() { "searching" } else if self.bom_job.is_some() { "match_bom" } else { "ready" };
                        ui.add(egui::Label::new(egui::RichText::new(i18n::t(self.lang, state)).size(11.5).color(theme::secondary())).extend());
                        ui.separator();
                        folder = ui.add(egui::Label::new(egui::RichText::new(&self.out_dir).size(11.5).color(theme::secondary())).truncate().sense(egui::Sense::click()))
                            .on_hover_text(format!("{}\n{}", self.out_dir, i18n::t(self.lang, "open_folder"))).clicked();
                    });
                });
            });
        if folder { self.open_out_dir(); }
    }

    fn activity_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_activity { return; }
        let mut open = true;
        fit_window(i18n::t(self.lang, "activity"), "activity_window", 650.0).open(&mut open).resizable(true).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(420.0).stick_to_bottom(true).show(ui, |ui| {
                for entry in &self.logs { ui.add(egui::Label::new(entry).selectable(true)); ui.separator(); }
            });
            if ui.button(i18n::t(self.lang, "ok")).clicked() { self.show_activity = false; }
        });
        if !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.show_activity = false; }
    }

    fn clear_queue_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_clear_queue { return; }
        let mut open = true;
        let mut clear = false;
        let mut cancel = false;
        fit_window(i18n::t(self.lang, "clear_queue_title"), "clear_queue", 380.0).open(&mut open).show(ctx, |ui| {
            ui.label(i18n::t(self.lang, "clear_queue_hint"));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                cancel = theme::pill_button(ui, i18n::t(self.lang, "cancel"), true, false).clicked();
                clear = theme::pill_button(ui, i18n::t(self.lang, "clear_confirm"), true, true).clicked();
            });
        });
        if clear { self.lib.clear_queue(); self.lib.save(); }
        if clear || cancel || !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.confirm_clear_queue = false; }
    }

    fn export_exact_items(&mut self, items: Vec<SearchItem>) {
        if self.busy() || items.is_empty() { return; }
        if !self.validate_export_target() { return; }
        let mut req = self.batch_opts.request(PathBuf::from(self.out_dir.trim()));
        self.apply_export_opts(&mut req);
        req.merge = self.batch_merge;
        self.job = Some(Promise::spawn_thread("export-confirmed", move || {
            Ok(export_report(&LcedaClient::new(), &items, &req))
        }));
    }
}

enum LibraryAction {
    ShowProgress,
    Bind(String, LibraryFormat), Unbind(String, LibraryFormat),
    Sync(String, Option<LibraryFormat>), SyncAll,
}

fn dropdown_menu<R>(ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<Option<R>> {
    // Paint the disclosure icon so it works with every Windows/CJK font fallback.
    let menu = ui.menu_button(format!("{label}    "), contents);
    let center = menu.response.rect.right_center() - egui::vec2(11.0, 0.0);
    let color = ui.style().interact(&menu.response).fg_stroke.color;
    ui.painter().add(egui::Shape::convex_polygon(vec![center + egui::vec2(-3.5, -2.0), center + egui::vec2(3.5, -2.0), center + egui::vec2(0.0, 2.5)], color, egui::Stroke::NONE));
    menu
}

fn library_menu(ui: &mut egui::Ui, library: &Library, category: &str, lang: Lang, busy: bool, has_progress: bool) -> Option<LibraryAction> {
    theme::prepare_menu(ui);
    let mut action = None;
    if has_progress {
        if ui.button(i18n::t(lang, "sync_view_progress")).clicked() { action = Some(LibraryAction::ShowProgress); ui.close(); }
        ui.separator();
    }
    let bound = LibraryFormat::ALL.into_iter().any(|f| library.binding_for(category, f).is_some());
    if ui.add_enabled(!busy && bound, egui::Button::new(i18n::t(lang, "sync_current_library"))).clicked() {
        action = Some(LibraryAction::Sync(category.into(), None)); ui.close();
    }
    for format in LibraryFormat::ALL {
        if library.binding_for(category, format).is_some() {
            let label = format!("{} {}", i18n::t(lang, "sync_format_prefix"), format.name());
            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                action = Some(LibraryAction::Sync(category.into(), Some(format))); ui.close();
            }
        }
    }
    ui.separator();
    ui.menu_button(i18n::t(lang, "bind_library"), |ui| {
        for format in LibraryFormat::ALL {
            if ui.add_enabled(!busy, egui::Button::new(format!("{} (.{})", format.name(), format.extension()))).clicked() {
                action = Some(LibraryAction::Bind(category.into(), format)); ui.close();
            }
        }
    });
    if bound {
        ui.menu_button(i18n::t(lang, "unbind_library"), |ui| {
            for format in LibraryFormat::ALL {
                if library.binding_for(category, format).is_some() && ui.add_enabled(!busy, egui::Button::new(format.name())).clicked() {
                    action = Some(LibraryAction::Unbind(category.into(), format)); ui.close();
                }
            }
        });
    }
    ui.separator();
    if ui.add_enabled(!busy && !library.bound_categories().is_empty(), egui::Button::new(i18n::t(lang, "sync_all_libraries"))).clicked() {
        action = Some(LibraryAction::SyncAll); ui.close();
    }
    action
}

fn can_merge_library_parts(ad: bool, kicad: bool, parts: impl IntoIterator<Item = (String, bool, bool, bool)>) -> bool {
    let mut ad_parts = std::collections::HashSet::new();
    let mut kicad_parts = std::collections::HashSet::new();
    for (id, has_cad, bound_ad, bound_kicad) in parts {
        if !has_cad { continue; }
        if kicad && !bound_kicad { kicad_parts.insert(id.clone()); }
        if ad && !bound_ad { ad_parts.insert(id); }
        if ad_parts.len() >= 2 || kicad_parts.len() >= 2 { return true; }
    }
    false
}

fn export_report(client: &LcedaClient, items: &[SearchItem], req: &ExportRequest) -> JobReport {
    // Recheck resolved parts: an old preference or a failed lookup must never
    // turn a single-part export into a merged library.
    let mut req = req.clone();
    req.merge &= can_merge_library_parts(req.ad, req.kicad, items.iter().map(|item| {
        let id = item.lcsc_id().unwrap_or_else(|| item.name().to_string()).to_ascii_uppercase();
        let bound = req.ad_library_dir.is_some() || (req.sync_bindings && req.ad_library_files.contains_key(&id));
        let kicad_bound = req.sync_bindings && req.kicad_library_files.contains_key(&id);
        (id, item.has_symbol_or_footprint(), bound, kicad_bound)
    }));
    let mut report = JobReport { message: String::new(), exported: Vec::new(), notify: true, failed: 0 };
    let mut fail = 0;
    for item in items {
        if item.model_uuid.is_none() && (req.step || req.obj) {
            report.message.push_str(&format!("SKIP {}: no 3D resource; other requested assets continue\n", item.name()));
        }
    }
    for (item, result) in lceda_core::export::export_items(client, items, &req) {
        match result {
            Ok(paths) => {
                report.message.push_str(&format!("OK {}\n{}\n", item.name(), format_paths(&paths, &req.out_dir)));
                report.exported.push((item, paths));
            }
            Err(e) => { fail += 1; report.message.push_str(&format!("FAIL {}: {e}\n", item.name())); }
        }
    }
    report.message.push_str(&format!("done, failed={fail}"));
    report.failed = fail;
    report
}

fn donate_para(ui: &mut egui::Ui, mark: Color32, text: &str) {
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().rect_filled(dot, 2.0, mark);
        ui.add(
            egui::Label::new(egui::RichText::new(text).color(theme::secondary()))
                .wrap(),
        );
    });
}

fn paint_part_row(ui: &mut egui::Ui, title: &str, sub: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 44.0),
        egui::Sense::click(),
    );
    let bg = if selected {
        Color32::from_rgba_unmultiplied(0, 122, 255, 36)
    } else if resp.hovered() {
        theme::well()
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 8.0, bg);
    }
    for (text, y, size, color) in [
        (title, rect.top() + 6.0, 14.0, theme::label()),
        (sub, rect.top() + 24.0, 12.0, theme::secondary()),
    ] {
        if text.is_empty() { continue; }
        let mut job = egui::text::LayoutJob::simple_singleline(text.into(), egui::FontId::proportional(size), color);
        job.wrap.max_width = (rect.width() - 20.0).max(1.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts(|fonts| fonts.layout_job(job));
        ui.painter_at(rect.shrink(2.0)).galley(egui::pos2(rect.left() + 10.0, y), galley, color);
    }
    resp.on_hover_text(format!("{title}\n{sub}"))
}

fn fit_window<'a>(
    title: impl Into<egui::WidgetText>,
    id: &'static str,
    width: f32,
) -> egui::Window<'a> {
    egui::Window::new(title)
        .id(egui::Id::new(id))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .default_width(width)
        .default_height(1.0)
        .min_width(width)
        .max_width(width)
        .min_height(0.0)
        .frame(
            egui::Frame::new()
                .fill(theme::fill())
                .stroke(egui::Stroke::new(1.0_f32, theme::hairline()))
                .corner_radius(12)
                .inner_margin(egui::Margin::same(12))
                .shadow(egui::Shadow::NONE),
        )
}

fn show_ad_field_map(ui: &mut egui::Ui, lang: Lang) {
    ui.label(
        egui::RichText::new(i18n::t(lang, "ad_map_title"))
            .strong()
            .color(theme::label()),
    );
    ui.add_space(6.0);
    let pairs = [
        ("Designator", "ad_map_designator"),
        ("Comment", "ad_map_comment"),
        ("Description", "ad_map_description"),
    ];
    egui::Frame::new()
        .fill(theme::fill())
        .stroke(egui::Stroke::new(1.0_f32, theme::hairline()))
        .corner_radius(8)
        .inner_margin(egui::Margin::ZERO)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (name, key)) in pairs.iter().enumerate() {
                paint_ad_prop_row(
                    ui,
                    &PropRow {
                        group: PropGroup::General,
                        name: (*name).into(),
                        value: i18n::t(lang, key).into(),
                    },
                    i,
                );
            }
        });
}

fn show_ad_props(
    ui: &mut egui::Ui,
    lang: Lang,
    rows: &[PropRow],
    filter: &str,
    max_h: Option<f32>,
    id: &'static str,
    show_hints: bool,
) {
    let q = filter.trim().to_lowercase();
    let shown: Vec<&PropRow> = rows
        .iter()
        .filter(|r| {
            q.is_empty()
                || r.name.to_lowercase().contains(&q)
                || r.value.to_lowercase().contains(&q)
        })
        .collect();
    let body = |ui: &mut egui::Ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        if shown.is_empty() {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(i18n::t(lang, "details_empty")).color(theme::secondary()),
            );
            ui.add_space(12.0);
            return;
        }
        let mut first = true;
        for group in [
            PropGroup::General,
            PropGroup::Identity,
            PropGroup::Specs,
            PropGroup::Extra,
        ] {
            let chunk: Vec<&PropRow> = shown
                .iter()
                .copied()
                .filter(|r| r.group == group)
                .collect();
            if chunk.is_empty() {
                continue;
            }
            if !first {
                ui.add_space(16.0);
            }
            first = false;
            egui::Frame::new()
                .fill(theme::fill())
                .stroke(egui::Stroke::new(1.0_f32, theme::hairline()))
                .corner_radius(8)
                .inner_margin(egui::Margin::ZERO)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let (header, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 32.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(header, 0.0, theme::fill_strong());
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(header.min, egui::vec2(3.0, header.height())),
                        0.0,
                        ACCENT,
                    );
                    ui.painter().text(
                        header.left_center() + egui::vec2(14.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        i18n::t(lang, group.i18n_key()),
                        egui::FontId::proportional(13.0),
                        theme::label(),
                    );
                    if show_hints {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(i18n::t(lang, group.hint_key()))
                                        .color(theme::secondary())
                                        .size(11.5),
                                )
                                .wrap(),
                            );
                        });
                        ui.add_space(8.0);
                    }
                    for (i, row) in chunk.iter().enumerate() {
                        paint_ad_prop_row(ui, row, i);
                    }
                });
        }
    };
    if let Some(max_h) = max_h {
        egui::ScrollArea::vertical()
            .id_salt(id)
            .min_scrolled_height(max_h)
            .max_height(max_h)
            .auto_shrink([false, false])
            .show(ui, body);
    } else {
        body(ui);
    }
}

fn paint_ad_prop_row(ui: &mut egui::Ui, row: &PropRow, i: usize) {
    let w = ui.available_width();
    let name_w = (w * 0.34).clamp(118.0, 168.0);
    let val_w = (w - name_w).max(80.0);
    let galley = ui.fonts(|f| {
        f.layout(
            row.value.clone(),
            egui::FontId::proportional(12.5),
            theme::label(),
            (val_w - 20.0).max(40.0),
        )
    });
    let h = (galley.size().y + 16.0).max(34.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let bg = if i % 2 == 0 {
        theme::well()
    } else {
        theme::fill()
    };
    ui.painter().rect_filled(rect, 0.0, bg);
    ui.painter().vline(
        rect.min.x + name_w,
        rect.y_range(),
        egui::Stroke::new(1.0_f32, theme::hairline()),
    );
    let name_y = if h > 40.0 { 17.0 } else { h * 0.5 };
    ui.painter().text(
        rect.min + egui::vec2(12.0, name_y),
        egui::Align2::LEFT_CENTER,
        &row.name,
        egui::FontId::proportional(12.0),
        theme::secondary(),
    );
    ui.painter().galley(
        egui::pos2(
            rect.min.x + name_w + 12.0,
            rect.min.y + (h - galley.size().y) * 0.5,
        ),
        galley,
        theme::label(),
    );
}

fn consume_dialog_confirm(ui: &mut egui::Ui, allow_space: bool) -> bool {
    ui.input_mut(|i| {
        let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
        let space = allow_space && i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
        enter || space
    })
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

fn bgr_color(c: i32) -> Color32 {
    let [r, g, b] = SchColors::to_rgb(c);
    Color32::from_rgb(r, g, b)
}

fn sch_color_row(ui: &mut egui::Ui, label: &str, color: &mut i32) -> bool {
    let mut rgb = SchColors::to_rgb(*color);
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *color = SchColors::from_rgb(rgb);
            changed = true;
        }
        ui.label(egui::RichText::new(label).color(theme::label()).size(12.0));
        ui.label(
            egui::RichText::new(SchColors::as_hex(*color))
                .color(theme::secondary())
                .size(11.0)
                .monospace(),
        );
    });
    changed
}

fn sch_color_preview(ui: &mut egui::Ui, colors: SchColors) {
    let w = ui.available_width().max(200.0);
    let pin_pt = colors.clamped_pin_font_size() as f32;
    let h = (148.0 + (pin_pt - 7.0) * 3.0).clamp(148.0, 188.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 8.0, bgr_color(colors.background));
    p.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0_f32, theme::hairline()),
        egui::StrokeKind::Inside,
    );
    let body = egui::Rect::from_center_size(rect.center() + egui::vec2(0.0, 6.0), egui::vec2(96.0, 78.0));
    let body_c = bgr_color(colors.body);
    p.rect_stroke(body, 0.0, egui::Stroke::new(1.6_f32, body_c), egui::StrokeKind::Inside);
    p.circle_stroke(
        egui::pos2(body.left() + 9.0, body.top() + 9.0),
        2.4,
        egui::Stroke::new(1.2_f32, body_c),
    );
    let left = ["V+", "S", "D+", "D-", "GND"];
    let right = ["#OE", "HSD2+", "HSD2-", "HSD1+", "HSD1-"];
    let font = egui::FontId::proportional(colors.clamped_pin_font_size() as f32);
    for i in 0..5 {
        let y = body.top() + 12.0 + i as f32 * 13.0;
        let ls = colors.pin_style("", left[i]);
        let rs = colors.pin_style("", right[i]);
        p.line_segment(
            [egui::pos2(body.left() - 18.0, y), egui::pos2(body.left(), y)],
            egui::Stroke::new(1.3_f32, bgr_color(ls.line)),
        );
        p.circle_stroke(
            egui::pos2(body.left() - 18.0, y),
            2.0,
            egui::Stroke::new(1.0_f32, bgr_color(ls.line)),
        );
        p.text(
            egui::pos2(body.left() - 24.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{}", i + 1),
            font.clone(),
            bgr_color(ls.number),
        );
        p.text(
            egui::pos2(body.left() + 5.0, y),
            egui::Align2::LEFT_CENTER,
            left[i],
            font.clone(),
            bgr_color(ls.name),
        );
        p.line_segment(
            [egui::pos2(body.right(), y), egui::pos2(body.right() + 18.0, y)],
            egui::Stroke::new(1.3_f32, bgr_color(rs.line)),
        );
        p.circle_stroke(
            egui::pos2(body.right() + 18.0, y),
            2.0,
            egui::Stroke::new(1.0_f32, bgr_color(rs.line)),
        );
        p.text(
            egui::pos2(body.right() + 24.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{}", 10 - i),
            font.clone(),
            bgr_color(rs.number),
        );
        p.text(
            egui::pos2(body.right() - 5.0, y),
            egui::Align2::RIGHT_CENTER,
            right[i],
            font.clone(),
            bgr_color(rs.name),
        );
    }
    p.text(
        egui::pos2(body.left(), body.top() - 8.0),
        egui::Align2::LEFT_BOTTOM,
        "U?",
        egui::FontId::proportional(12.0),
        bgr_color(colors.designator),
    );
    p.text(
        egui::pos2(body.left(), body.bottom() + 6.0),
        egui::Align2::LEFT_TOP,
        "PART",
        egui::FontId::proportional(12.0),
        bgr_color(colors.comment),
    );
}

fn title_bar_radius(maximized: bool) -> egui::CornerRadius {
    if maximized {
        egui::CornerRadius::ZERO
    } else {
        egui::CornerRadius {
            nw: WIN_CORNER,
            ne: WIN_CORNER,
            sw: 0,
            se: 0,
        }
    }
}

fn shell_panel_fill(maximized: bool) -> Color32 {
    if maximized {
        theme::window_bg()
    } else {
        Color32::TRANSPARENT
    }
}

fn resize_dir_at(
    rect: egui::Rect,
    pos: egui::Pos2,
    edge: f32,
    corner: f32,
) -> Option<egui::viewport::ResizeDirection> {
    use egui::viewport::ResizeDirection;
    let left = pos.x - rect.left();
    let right = rect.right() - pos.x;
    let top = pos.y - rect.top();
    let bottom = rect.bottom() - pos.y;
    if left <= corner && top <= corner {
        return Some(ResizeDirection::NorthWest);
    }
    if right <= corner && top <= corner {
        return Some(ResizeDirection::NorthEast);
    }
    if left <= corner && bottom <= corner {
        return Some(ResizeDirection::SouthWest);
    }
    if right <= corner && bottom <= corner {
        return Some(ResizeDirection::SouthEast);
    }
    if top <= edge {
        return Some(ResizeDirection::North);
    }
    if bottom <= edge {
        return Some(ResizeDirection::South);
    }
    if left <= edge {
        return Some(ResizeDirection::West);
    }
    if right <= edge {
        return Some(ResizeDirection::East);
    }
    None
}

fn resize_cursor(dir: egui::viewport::ResizeDirection) -> egui::CursorIcon {
    use egui::viewport::ResizeDirection;
    match dir {
        ResizeDirection::North | ResizeDirection::South => egui::CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
    }
}

fn handle_window_resize(ctx: &egui::Context, maximized: bool) {
    if maximized {
        return;
    }
    let Some(pos) = ctx.pointer_hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos()))
    else {
        return;
    };
    let Some(dir) = resize_dir_at(ctx.screen_rect(), pos, RESIZE_EDGE, RESIZE_CORNER) else {
        return;
    };
    ctx.set_cursor_icon(resize_cursor(dir));
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
    }
}

fn paint_window_shell(ctx: &egui::Context, maximized: bool) {
    let rect = ctx.screen_rect();
    let radius = if maximized {
        egui::CornerRadius::ZERO
    } else {
        egui::CornerRadius::same(WIN_CORNER)
    };
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.rect_filled(rect, radius, theme::window_bg());
    if !maximized {
        painter.rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(1.0_f32, theme::hairline()),
            egui::StrokeKind::Inside,
        );
    }
}

fn paint_well(ui: &egui::Ui, rect: egui::Rect) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 10.0, theme::well());
    p.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0_f32, theme::hairline()),
        egui::StrokeKind::Inside,
    );
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

fn env_f32(key: &str) -> Option<f32> {
    env::var(key).ok()?.parse().ok()
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

fn load_named_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(name, color, TextureOptions::LINEAR))
}

fn load_nav_tex(ctx: &egui::Context) -> Option<NavTex> {
    Some(NavTex {
        sidebar: load_named_texture(ctx, "nav_sidebar", SIDEBAR_ICON_PNG)?,
        search: load_named_texture(ctx, "nav_search", NAV_SEARCH_PNG)?,
        fav: load_named_texture(ctx, "nav_fav", NAV_FAV_PNG)?,
        queue: load_named_texture(ctx, "nav_queue", NAV_QUEUE_PNG)?,
        sponsor: load_named_texture(ctx, "nav_sponsor", NAV_SPONSOR_PNG)?,
        settings: load_named_texture(ctx, "nav_settings", NAV_SETTINGS_PNG)?,
        about: load_named_texture(ctx, "nav_about", NAV_ABOUT_PNG)?,
    })
}

fn clamp_parts_max(window_w: f32, nav_w: f32) -> f32 {
    (window_w - nav_w - RIGHT_MIN).clamp(PARTS_MIN, PARTS_MAX)
}

fn split_grip_rect(gap: egui::Rect, vertical: bool) -> egui::Rect {
    if vertical {
        egui::Rect::from_center_size(
            gap.center(),
            egui::vec2(3.0, (gap.height() * 0.32).clamp(24.0, 72.0)),
        )
    } else {
        egui::Rect::from_center_size(
            gap.center(),
            egui::vec2((gap.width() * 0.32).clamp(24.0, 120.0), 3.0),
        )
    }
}

fn paint_split_grip(painter: &egui::Painter, gap: egui::Rect, vertical: bool, hot: bool) {
    painter.rect_filled(
        split_grip_rect(gap, vertical),
        2.0,
        if hot { ACCENT } else { theme::hairline() },
    );
}

fn parts_split_drag(ctx: &egui::Context, panel: egui::Rect) -> (f32, bool) {
    let gap = egui::Rect::from_min_max(
        egui::pos2(panel.right() - 5.0, panel.top() + 16.0),
        egui::pos2(panel.right() + 5.0, panel.bottom() - 16.0),
    );
    if gap.width() < 1.0 || gap.height() < 1.0 {
        return (0.0, false);
    }
    let mut delta = 0.0;
    let mut done = false;
    egui::Area::new(egui::Id::new("parts_split_area"))
        .order(egui::Order::Foreground)
        .fixed_pos(gap.min)
        .interactable(true)
        .show(ctx, |ui| {
            let (_rect, resp) = ui.allocate_exact_size(gap.size(), egui::Sense::drag());
            let hot = resp.hovered() || resp.dragged();
            if hot {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                paint_split_grip(ui.painter(), gap, true, true);
            }
            delta = resp.drag_delta().x;
            done = resp.drag_stopped();
        });
    (delta, done)
}

fn load_pay_tex(ctx: &egui::Context) -> Option<PayTex> {
    Some(PayTex {
        wechat: load_named_texture(ctx, "pay_wechat", PAY_WECHAT)?,
        alipay: load_named_texture(ctx, "pay_alipay", PAY_ALIPAY)?,
        afdian: load_named_texture(ctx, "pay_afdian", PAY_AFDIAN)?,
        cn: load_named_texture(ctx, "flag_cn", FLAG_CN)?,
        world: load_named_texture(ctx, "flag_world", FLAG_WORLD)?,
    })
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
    let bg = theme::well();
    for px in pixels.chunks_exact_mut(4) {
        px[0] = bg.r();
        px[1] = bg.g();
        px[2] = bg.b();
        px[3] = bg.a();
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn merge_is_only_available_for_multiple_distinct_parts_in_the_same_export() {
        let part = |id: &str, cad, bound| (id.to_string(), cad, bound, false);
        assert!(!can_merge_library_parts(true, true, [part("C1", true, false)]));
        assert!(!can_merge_library_parts(true, true, [part("C1", true, false), part("C1", true, false)]));
        assert!(!can_merge_library_parts(true, true, [part("C1", true, false), part("C2", false, false)]));
        assert!(!can_merge_library_parts(false, false, [part("C1", true, false), part("C2", true, false)]));
        assert!(can_merge_library_parts(true, false, [part("C1", true, false), part("C2", true, false)]));
        assert!(!can_merge_library_parts(true, false, [part("C1", true, true), part("C2", true, true)]));
        assert!(!can_merge_library_parts(true, false, [part("C1", true, false), part("C2", true, true)]));
        assert!(can_merge_library_parts(true, true, [part("C1", true, true), part("C2", true, true)]));
        assert!(!can_merge_library_parts(true, true, [("C1".into(), true, true, true), ("C2".into(), true, true, true)]));
        assert!(!can_merge_library_parts(false, true, [("C1".into(), true, false, true), ("C2".into(), true, false, true)]));
    }

    #[test]
    fn parts_min_keeps_lcsc_prefix() {
        assert!(PARTS_MIN >= 200.0);
        let max = clamp_parts_max(1180.0, 208.0);
        assert!(max >= PARTS_MIN);
        assert!(max <= PARTS_MAX);
        assert!(1180.0 - 208.0 - max >= RIGHT_MIN - 1.0);
        let tight = clamp_parts_max(960.0, 208.0);
        assert!(tight >= PARTS_MIN);
        assert!(960.0 - 208.0 - tight >= RIGHT_MIN - 1.0);
    }

    #[test]
    fn window_edge_picks_resize_direction() {
        use egui::viewport::ResizeDirection;
        let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0));
        assert_eq!(
            resize_dir_at(r, egui::pos2(2.0, 150.0), 6.0, 12.0),
            Some(ResizeDirection::West)
        );
        assert_eq!(
            resize_dir_at(r, egui::pos2(398.0, 150.0), 6.0, 12.0),
            Some(ResizeDirection::East)
        );
        assert_eq!(
            resize_dir_at(r, egui::pos2(8.0, 8.0), 6.0, 12.0),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            resize_dir_at(r, egui::pos2(200.0, 150.0), 6.0, 12.0),
            None
        );
    }
}
