use std::env;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        if code.to_ascii_lowercase().starts_with("zh") {
            Self::Zh
        } else {
            Self::En
        }
    }

    pub fn detect() -> Self {
        for key in ["LCEDA_LANG", "LANG", "LC_ALL"] {
            if let Ok(v) = env::var(key) {
                if !v.is_empty() {
                    return Self::from_code(&v);
                }
            }
        }
        Self::Zh
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Zh => Self::En,
            Self::En => Self::Zh,
        }
    }
}

pub fn t(lang: Lang, key: &str) -> &'static str {
    match (lang, key) {
        (Lang::Zh, "app_title") => "立创封装助手",
        (Lang::En, "app_title") => "LCSC Parts",
        (Lang::Zh, "search") => "搜索",
        (Lang::En, "search") => "Search",
        (Lang::Zh, "keyword") => "型号 / 立创编号",
        (Lang::En, "keyword") => "MPN or LCSC ID",
        (Lang::Zh, "keyword_hint") => "例如 C8755 或 TYPE-C",
        (Lang::En, "keyword_hint") => "e.g. C8755 or TYPE-C",
        (Lang::Zh, "components") => "器件",
        (Lang::En, "components") => "Parts",
        (Lang::Zh, "preview") => "预览",
        (Lang::En, "preview") => "Preview",
        (Lang::Zh, "model3d") => "三维外形",
        (Lang::En, "model3d") => "3D outline",
        (Lang::Zh, "download_step") => "下载 STEP",
        (Lang::En, "download_step") => "Download STEP",
        (Lang::Zh, "download_obj") => "下载 OBJ",
        (Lang::En, "download_obj") => "Download OBJ",
        (Lang::Zh, "export_ad") => "导出 AD 库",
        (Lang::En, "export_ad") => "Export Altium",
        (Lang::Zh, "export_kicad") => "导出 KiCad",
        (Lang::En, "export_kicad") => "Export KiCad",
        (Lang::Zh, "export_pads") => "导出 PADS",
        (Lang::En, "export_pads") => "Export PADS",
        (Lang::Zh, "datasheet") => "数据手册",
        (Lang::En, "datasheet") => "Datasheet",
        (Lang::Zh, "open_page") => "打开立创页",
        (Lang::En, "open_page") => "Open LCSC",
        (Lang::Zh, "batch") => "批量文件…",
        (Lang::En, "batch") => "Batch file…",
        (Lang::Zh, "batch_title") => "批量导出",
        (Lang::En, "batch_title") => "Batch export",
        (Lang::Zh, "batch_hint") => "勾选要写出的类型，再选文本列表。",
        (Lang::En, "batch_hint") => "Tick the formats to write, then pick the list file.",
        (Lang::Zh, "batch_start") => "选择文件…",
        (Lang::En, "batch_start") => "Choose file…",
        (Lang::Zh, "batch_cancel") => "取消",
        (Lang::En, "batch_cancel") => "Cancel",
        (Lang::Zh, "batch_none") => "请至少勾选一种导出。",
        (Lang::En, "batch_none") => "Tick at least one export type.",
        (Lang::Zh, "export_source") => "仅源文件",
        (Lang::En, "export_source") => "JSON only",
        (Lang::Zh, "output") => "保存到",
        (Lang::En, "output") => "Save to",
        (Lang::Zh, "output_hint") => "下载和导出的文件夹",
        (Lang::En, "output_hint") => "Download / export folder",
        (Lang::Zh, "browse") => "选择…",
        (Lang::En, "browse") => "Browse…",
        (Lang::Zh, "open_folder") => "打开目录",
        (Lang::En, "open_folder") => "Open folder",
        (Lang::Zh, "opened_folder") => "已打开文件夹",
        (Lang::En, "opened_folder") => "Opened folder",
        (Lang::Zh, "saving_to") => "正在保存到",
        (Lang::En, "saving_to") => "Saving to",
        (Lang::Zh, "searching") => "正在搜索…",
        (Lang::En, "searching") => "Searching…",
        (Lang::Zh, "working") => "处理中…",
        (Lang::En, "working") => "Working…",
        (Lang::Zh, "log") => "日志",
        (Lang::En, "log") => "Log",
        (Lang::Zh, "manufacturer") => "厂牌",
        (Lang::En, "manufacturer") => "Mfr",
        (Lang::Zh, "has3d") => "3D",
        (Lang::En, "has3d") => "3D",
        (Lang::Zh, "no_results") => "没有找到器件",
        (Lang::En, "no_results") => "No parts found",
        (Lang::Zh, "empty_keyword") => "请输入关键字",
        (Lang::En, "empty_keyword") => "Please enter a keyword",
        (Lang::Zh, "select_first") => "请先选择器件",
        (Lang::En, "select_first") => "Select a part first",
        (Lang::Zh, "done") => "完成",
        (Lang::En, "done") => "Done",
        (Lang::Zh, "error") => "出错",
        (Lang::En, "error") => "Error",
        (Lang::Zh, "no_dotnet") => "搜索器件，导出 Altium / KiCad / PADS 库与 3D 模型。",
        (Lang::En, "no_dotnet") => "Search parts and export Altium / KiCad / PADS libraries and 3D models.",
        (Lang::Zh, "loading") => "加载中…",
        (Lang::En, "loading") => "Loading…",
        (Lang::Zh, "no_3d") => "该器件没有 3D 模型",
        (Lang::En, "no_3d") => "No 3D model",
        (Lang::Zh, "mesh_fail") => "3D 预览解析失败",
        (Lang::En, "mesh_fail") => "Could not parse 3D preview",
        (Lang::Zh, "drag_orbit") => "拖动旋转 · 滚轮缩放",
        (Lang::En, "drag_orbit") => "Drag to orbit · scroll to zoom",
        (Lang::Zh, "preview_none") => "无预览图",
        (Lang::En, "preview_none") => "No photo",
        (Lang::Zh, "batch_done") => "批量完成",
        (Lang::En, "batch_done") => "Batch done",
        (Lang::Zh, "opened") => "已在浏览器打开",
        (Lang::En, "opened") => "Opened in browser",
        (Lang::Zh, "notice") => "提示",
        (Lang::En, "notice") => "Notice",
        (Lang::Zh, "ok") => "确定",
        (Lang::En, "ok") => "OK",
        (Lang::Zh, "no_3d_dl") => "该器件没有 3D 模型，无法下载。",
        (Lang::En, "no_3d_dl") => "This part has no 3D model.",
        (Lang::Zh, "no_cad") => "该器件没有原理图或封装，无法导出。",
        (Lang::En, "no_cad") => "This part has no symbol or footprint.",
        (Lang::Zh, "no_datasheet") => "没有可下载的数据手册。",
        (Lang::En, "no_datasheet") => "No datasheet is available.",
        (Lang::Zh, "json_kept") => "已保留 EasyEDA 源 JSON，可对照检查。",
        (Lang::En, "json_kept") => "EasyEDA source JSON was kept for inspection.",
        (Lang::Zh, "about") => "关于",
        (Lang::En, "about") => "About",
        (Lang::Zh, "about_author") => "作者",
        (Lang::En, "about_author") => "Author",
        (Lang::Zh, "about_repo") => "仓库",
        (Lang::En, "about_repo") => "Repository",
        (Lang::Zh, "about_license") => "许可",
        (Lang::En, "about_license") => "License",
        (Lang::Zh, "about_ver") => "版本",
        (Lang::En, "about_ver") => "Version",
        (Lang::Zh, "open_repo") => "打开Github仓库",
        (Lang::En, "open_repo") => "Open GitHub repo",
        (Lang::Zh, "check_update") => "检查更新",
        (Lang::En, "check_update") => "Check updates",
        (Lang::Zh, "checking_update") => "正在检查…",
        (Lang::En, "checking_update") => "Checking…",
        (Lang::Zh, "already_latest") => "当前已是最新版本 {ver}",
        (Lang::En, "already_latest") => "You already have the latest version {ver}",
        (Lang::Zh, "update_check_fail") => "检查更新失败，请稍后重试。",
        (Lang::En, "update_check_fail") => "Could not check for updates. Try again later.",
        (Lang::Zh, "update_title") => "发现新版本",
        (Lang::En, "update_title") => "Update available",
        (Lang::Zh, "update_body") => "当前 {cur}，可更新到 {new}。下载并替换后会自动重启。",
        (Lang::En, "update_body") => "You have {cur}. Version {new} is available. The app will restart after updating.",
        (Lang::Zh, "update_notes") => "更新说明",
        (Lang::En, "update_notes") => "Release notes",
        (Lang::Zh, "update_now") => "立即更新",
        (Lang::En, "update_now") => "Update now",
        (Lang::Zh, "update_later") => "以后再说",
        (Lang::En, "update_later") => "Later",
        (Lang::Zh, "update_downloading") => "正在下载更新…",
        (Lang::En, "update_downloading") => "Downloading update…",
        (Lang::Zh, "update_extracting") => "正在解压…",
        (Lang::En, "update_extracting") => "Extracting…",
        (Lang::Zh, "update_installing") => "正在安装…",
        (Lang::En, "update_installing") => "Installing…",
        (Lang::Zh, "update_open") => "打开下载页",
        (Lang::En, "update_open") => "Open download page",
        _ => "???",
    }
}
