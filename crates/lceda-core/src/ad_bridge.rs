//! Local, opt-in IPC with a native plugin or compatibility script inside AD.
//!
//! The workstation writes only disposable source libraries. AD owns every edit
//! to the bound libraries. A reply is accepted only for the current session and
//! request, after AD has saved, reopened and read the components back.
use crate::{altium::{self, SchColors}, managed_ad::Entry, Error, Result};
use std::{collections::BTreeMap, fmt, fs, io::Write, path::{Path, PathBuf}, sync::Arc,
    thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

const PROTOCOL: &str = "1";
const HEARTBEAT_MAX_AGE: Duration = Duration::from_secs(4);
const TIMEOUT: Duration = Duration::from_secs(120);
const BUSY_MAX_AGE: Duration = Duration::from_secs(130);
const SCRIPT: &str = include_str!("../../../integrations/altium/LcedaBridge.pas");
const FORM: &str = include_str!("../../../integrations/altium/LcedaBridge.dfm");
const PROJECT: &str = include_str!("../../../integrations/altium/LcedaBridge.PrjScr");
const GUIDE: &str = include_str!("../../../integrations/altium/README_ZH.md");

#[derive(Clone)]
pub struct Bridge {
    pub directory: PathBuf,
    progress: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}
impl fmt::Debug for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_struct("Bridge").field("directory", &self.directory).finish() }
}
impl Bridge {
    pub fn new(directory: PathBuf) -> Self { Self { directory, progress: None } }
    pub fn with_progress(mut self, progress: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(progress)); self
    }
    fn report(&self, phase: &str) { if let Some(progress) = &self.progress { progress(phase); } }
}

#[derive(Debug, Clone)]
pub struct Connection { pub session: String, pub native: bool }

#[derive(Debug)]
pub struct Synced {
    pub schlib: PathBuf, pub pcblib: PathBuf,
    pub component: String, pub existing: bool,
    pub footprint_existing: bool,
}

pub fn enabled(directory: &Path) -> bool { directory.join("enabled").is_file() }

/// File synchronization is an explicit choice. A missing plugin or heartbeat
/// must never silently select out-of-process writes on Windows.
pub fn file_mode(directory: &Path) -> bool { directory.join("file-mode").is_file() }

pub fn enable(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    fs::write(directory.join("enabled"), PROTOCOL)?;
    let stop = directory.join("stop");
    if stop.exists() { fs::remove_file(stop)?; }
    let offline = directory.join("file-mode");
    if offline.exists() { fs::remove_file(offline)?; }
    Ok(())
}

pub fn disable(directory: &Path) -> Result<()> {
    require_editor_closed()?;
    fs::create_dir_all(directory)?;
    fs::write(directory.join("stop"), b"stop")?;
    let path = directory.join("enabled");
    if path.exists() { fs::remove_file(path)?; }
    fs::write(directory.join("file-mode"), PROTOCOL)?;
    Ok(())
}

/// Install a self-contained script project in this Windows user's local profile.
/// Do not replace scripts underneath a running AD session.
pub fn install(directory: &Path) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let project = directory.join("LcedaBridge.PrjScr");
    let stop = directory.join("stop");
    if connection(directory).is_ok() {
        enable(directory)?;
        return Ok(project);
    }
    if stop.exists() { fs::remove_file(stop)?; }
    let root = absolute(directory)?;
    let script = SCRIPT.replace("__IPC_ROOT_HEX__", &hex(&editor_path(&root)?));
    for (name, text) in [("LcedaBridge.pas", script.as_str()), ("LcedaBridge.dfm", FORM),
        ("LcedaBridge.PrjScr", PROJECT), ("使用说明.md", GUIDE)] {
        fs::write(directory.join(name), text.replace('\n', "\r\n"))?;
    }
    enable(directory)?;
    Ok(project)
}

pub fn connection(directory: &Path) -> Result<Connection> {
    let path = directory.join("heartbeat.ini");
    let fields = read_fields(&path).map_err(|_| disconnected())?;
    let state = fields.get("state").map(String::as_str);
    if fields.get("protocol").map(String::as_str) != Some(PROTOCOL)
        || !matches!(state, Some("ready" | "busy")) {
        return Err(disconnected());
    }
    let age = fs::metadata(path)?.modified()?.elapsed().map_err(|_| disconnected())?;
    let session = fields.get("session").filter(|s| valid_token(s)).ok_or_else(disconnected)?;
    let max_age = if state == Some("busy") { BUSY_MAX_AGE } else { HEARTBEAT_MAX_AGE };
    if age > max_age { return Err(disconnected()); }
    Ok(Connection { session: session.clone(), native: fields.get("backend").map(String::as_str) == Some("native") })
}

fn disconnected() -> Error {
    Error::msg("AD 尚未连接。请在“同步库 → 连接 AD…”安装原生插件并启动 AD，或运行兼容脚本的 LCEDA_StartBridge；不会改写绑定库文件。")
}

/// Names are searchable by both model and supplier ID; footprint IDs remain
/// stable and collision-free. Existing library objects are never renamed.
pub fn component_name(entry: &Entry) -> String {
    let model = entry.symbol.as_ref().map(|s| if s.meta.mpn.trim().is_empty() { &s.name } else { &s.meta.mpn })
        .map(|s| crate::util::sanitize_filename(s.trim())).unwrap_or_default();
    let id = entry.id.to_ascii_uppercase();
    let model: String = model.chars().take(160).collect();
    if model.is_empty() || model.eq_ignore_ascii_case(&id) { id }
    else if model.to_ascii_uppercase().ends_with(&format!("_{id}")) { model }
    else { format!("{model}_{id}") }
}

impl Bridge {
    pub fn update(&self, target: &Path, mut entry: Entry, colors: SchColors) -> Result<Synced> {
        self.update_with_timeout(target, &mut entry, colors, TIMEOUT)
    }

    fn update_with_timeout(&self, target: &Path, entry: &mut Entry, colors: SchColors, timeout: Duration) -> Result<Synced> {
        if !crate::models::looks_like_lcsc_token(&entry.id) || (entry.symbol.is_none() && entry.footprint.is_none()) {
            return Err(Error::msg("AD 同步需要明确的 C 编号及符号或封装"));
        }
        entry.id.make_ascii_uppercase();
        let target = absolute(target)?;
        let pcb_target = crate::bound_ad::paired(&target)?;
        let root = absolute(&self.directory)?;
        fs::create_dir_all(&root)?;
        let _lock = crate::library_lock::acquire(&root.join("client.lock"))?;
        let connection = connection(&root)?;
        let parent = target.parent().ok_or_else(|| Error::msg("AD 库路径缺少目录"))?;
        fs::create_dir_all(parent)?;
        let filename = target.file_name().ok_or_else(|| Error::msg("AD 库路径缺少文件名"))?.to_string_lossy();
        let _library_lock = crate::library_lock::acquire(&parent.join(format!(".{filename}.jlc-lock")))?;
        if parent.join(format!(".{filename}.jlc-transaction")).exists() {
            return Err(Error::msg("上次文件同步的事务尚未恢复；请关闭 AD 后使用文件同步恢复，再连接 AD。原库及备份保留。"));
        }
        let request = format!("{}-{}", std::process::id(), crate::util::unique_id());
        let job = root.join("jobs").join(&request);
        fs::create_dir_all(&job)?;
        let name = component_name(entry);
        let source_sch = job.join("source.SchLib");
        let source_pcb = job.join("source.PcbLib");
        let seed_sch = job.join("empty.SchLib");
        let seed_pcb = job.join("empty.PcbLib");
        if let Some(symbol) = &mut entry.symbol {
            symbol.name = name.clone();
            if entry.footprint.is_some() {
                // The native module adds the implementation only to a NEW
                // symbol. The compatibility script edits its source link.
                symbol.meta.footprint_lib = if connection.native { String::new() } else { entry.id.clone() };
            }
            altium::write_schlib_with_colors(&source_sch, symbol, colors)?;
            altium::schlib::write_empty(&seed_sch, colors)?;
        }
        if let Some(footprint) = &mut entry.footprint {
            footprint.name = entry.id.clone();
            // Models are imported by AD, so no dangling source-library GUIDs
            // are copied into the destination PcbLib.
            altium::write_pcblib(&source_pcb, footprint)?;
            altium::pcblib::write_empty(&seed_pcb)?;
        }
        let step = job.join("model.step");
        if entry.footprint.is_some() {
            if let Some(bytes) = &entry.step {
                if !crate::util::looks_like_step(bytes) { return Err(Error::msg("AD 同步的 STEP 模型无效")); }
                fs::write(&step, bytes)?;
            }
        }
        let expires = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| Error::msg(e.to_string()))?.as_secs() + timeout.as_secs().max(1);
        let mut fields = BTreeMap::from([
            ("protocol".into(), PROTOCOL.into()), ("session".into(), connection.session.clone()),
            ("request".into(), request.clone()), ("expires".into(), expires.to_string()),
            ("ttl_seconds".into(), timeout.as_secs().max(1).to_string()),
            ("symbol".into(), hex(if entry.symbol.is_some() { &name } else { "" })),
            ("legacy_symbol".into(), hex(&entry.id)),
            ("footprint".into(), hex(if entry.footprint.is_some() { &entry.id } else { "" })),
        ]);
        if connection.native && entry.symbol.is_some() && target.is_file() {
            let parts = crate::bound_ad::inspect_library(&target)?;
            let suffix = format!("_{}", entry.id);
            let matches: Vec<_> = parts.iter().filter(|part| part.name.eq_ignore_ascii_case(&name)
                || part.name.eq_ignore_ascii_case(&entry.id) || part.name.to_ascii_uppercase().ends_with(&suffix)).collect();
            if matches.len() > 1 { return Err(Error::msg("AD 库中有多个器件使用相同 C 编号，请先确认，未提交同步")); }
            fields.insert("existing_symbol".into(), hex(matches.first().map(|part| part.name.as_str()).unwrap_or("")));
        }
        for (key, path) in [("source_sch", &source_sch), ("source_pcb", &source_pcb),
            ("seed_sch", &seed_sch), ("seed_pcb", &seed_pcb), ("target_sch", &target),
            ("target_pcb", &pcb_target), ("job", &job), ("step", &step)] {
            fields.insert(key.into(), hex(&editor_path(path)?));
        }
        let placement = entry.footprint.as_ref().map(|f| f.model).unwrap_or_default();
        for (key, value) in [("rx", placement.rot_x), ("ry", placement.rot_y), ("rz", placement.rot_z)] {
            fields.insert(key.into(), ((value * 1000.0).round() as i32).to_string());
        }
        for (key, value) in [("x", placement.off_x), ("y", placement.off_y), ("z", placement.off_z)] {
            fields.insert(key.into(), altium::binary::from_mm(value).to_string());
        }
        write_fields(&job.join("request.ini"), &fields)?;
        // Only one pointer can be outstanding, protected across app/CLI processes.
        // An expired pointer is never replayed in a new AD session.
        if root.join("pending.ini").exists() {
            let pending = read_fields(&root.join("pending.ini"))?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            if pending.get("expires").and_then(|s| s.parse::<u64>().ok()).is_some_and(|s| s >= now) {
                return Err(Error::msg("AD 上一次同步尚未结束，请查看连接窗口及任务记录"));
            }
            fs::remove_file(root.join("pending.ini"))?;
        }
        let started = Instant::now();
        self.report("waiting");
        write_fields(&root.join("pending.ini"), &fields)?;
        let result = self.wait(&root, &job, &fields, started, timeout);
        // A timeout leaves the job and backup for inspection. Do not fall back to
        // a direct write or enqueue an automatic retry of an uncertain AD edit.
        if result.is_err() {
            if let Ok(pending) = read_fields(&root.join("pending.ini")) {
                if pending.get("request") == Some(&request) { let _ = fs::remove_file(root.join("pending.ini")); }
            }
        }
        let reply = result?;
        let component = unhex(reply.get("component").map(String::as_str).unwrap_or(""))?;
        let footprint = unhex(reply.get("footprint").map(String::as_str).unwrap_or(""))?;
        if entry.symbol.is_some() && component != name && component != entry.id
            && !(reply.get("existing").map(String::as_str) == Some("1") && component.to_ascii_uppercase().ends_with(&format!("_{}", entry.id))) {
            return Err(Error::msg("AD 返回了其它器件的名称，已拒绝标记同步完成"));
        }
        if entry.footprint.is_some() && footprint != entry.id {
            return Err(Error::msg("AD 返回了其它封装的名称，已拒绝标记同步完成"));
        }
        verify_disk(&target, &pcb_target, entry, &component, &reply)?;
        self.report("complete");
        Ok(Synced { schlib: target, pcblib: pcb_target, component,
            existing: reply.get("existing").map(String::as_str) == Some("1"),
            footprint_existing: reply.get("footprint_existing").map(String::as_str) == Some("1") })
    }

    fn wait(&self, root: &Path, job: &Path, request: &BTreeMap<String, String>, started: Instant, timeout: Duration) -> Result<BTreeMap<String, String>> {
        let mut last_phase = String::new();
        loop {
            if let Ok(status) = read_fields(&job.join("status.ini")) {
                if same_exchange(&status, request) {
                    let phase = status.get("phase").cloned().unwrap_or_default();
                    if phase != last_phase { self.report(&phase); last_phase = phase; }
                }
            }
            if job.join("reply.ini").exists() {
                let reply = read_fields(&job.join("reply.ini"))?;
                if !same_exchange(&reply, request) { return Err(Error::msg("AD 响应的会话或任务编号不匹配，未标记完成")); }
                if reply.get("ok").map(String::as_str) != Some("1") {
                    let error = unhex(reply.get("error").map(String::as_str).unwrap_or(""))?;
                    return Err(Error::msg(format!("AD 同步失败：{error}。记录与原库备份：{}", job.display())));
                }
                if reply.get("verified").map(String::as_str) != Some("1") {
                    return Err(Error::msg("AD 未确认保存并重新读取器件，未标记完成"));
                }
                return Ok(reply);
            }
            if started.elapsed() >= timeout {
                return Err(Error::msg(format!("等待 AD 超时，结果尚未确认；请查看 AD 连接窗口。不会自动重写或重复提交。记录：{}", job.display())));
            }
            // Before AD claims the request, fail quickly on a stopped/restarted
            // editor. Once claimed, give saving large libraries the full timeout.
            if !job.join("status.ini").exists() {
                match connection(root) {
                    Ok(c) if request.get("session") == Some(&c.session) => {},
                    _ => return Err(disconnected()),
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn verify_disk(sch: &Path, pcb: &Path, entry: &Entry, name: &str, reply: &BTreeMap<String, String>) -> Result<()> {
    if entry.symbol.is_some() {
        let parts = crate::bound_ad::inspect_library(sch)?;
        let part = parts.iter().find(|p| p.name == name).ok_or_else(|| Error::msg("AD 回复成功，但保存后的 SchLib 找不到该器件"))?;
        if reply.get("existing").map(String::as_str) != Some("1") {
            let symbol = entry.symbol.as_ref().unwrap();
            if part.pins != symbol.pins.len() { return Err(Error::msg("AD 保存后的符号管脚数量与源器件不一致")); }
            if entry.footprint.is_some() && !part.footprints.iter().any(|name| name == &entry.id) {
                return Err(Error::msg("AD 保存后的新增符号缺少本次封装关联"));
            }
        }
    }
    if entry.footprint.is_some() {
        let existing = reply.get("footprint_existing").map(String::as_str) == Some("1");
        if entry.step.is_some() && !existing {
            if reply.get("model_verified").map(String::as_str) != Some("1") {
                return Err(Error::msg("AD 未确认新增封装的内嵌 3D 模型"));
            }
        }
        crate::bound_ad::verify_editor_footprint(pcb, &entry.id, entry.step.as_deref().filter(|_| !existing))?;
    }
    Ok(())
}

fn same_exchange(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> bool {
    ["protocol", "session", "request"].iter().all(|key| a.get(*key).is_some() && a.get(*key) == b.get(*key))
}
fn valid_token(s: &str) -> bool { !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') }
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) })
}
/// AD's APIs do not consistently accept Win32 extended path prefixes.
fn editor_path(path: &Path) -> Result<String> {
    let s = path.to_str().ok_or_else(|| Error::msg("AD 路径不是有效 Unicode"))?;
    let s = if let Some(unc) = s.strip_prefix("\\\\?\\UNC\\") { format!("\\\\{unc}") }
        else { s.strip_prefix("\\\\?\\").unwrap_or(s).to_string() };
    if s.contains(['\r', '\n', '\0']) { return Err(Error::msg("AD 路径包含无效字符")); }
    Ok(s)
}
fn hex(s: &str) -> String { s.encode_utf16().map(|c| format!("{c:04X}")).collect() }
fn unhex(s: &str) -> Result<String> {
    if !s.is_ascii() || s.len() % 4 != 0 { return Err(Error::msg("AD 响应文本编码无效")); }
    let units: std::result::Result<Vec<_>, _> = (0..s.len()).step_by(4).map(|i| u16::from_str_radix(&s[i..i + 4], 16)).collect();
    String::from_utf16(&units.map_err(|_| Error::msg("AD 响应文本编码无效"))?).map_err(|_| Error::msg("AD 响应 Unicode 无效"))
}
fn read_fields(path: &Path) -> Result<BTreeMap<String, String>> {
    if fs::metadata(path)?.len() > 128 * 1024 { return Err(Error::msg("AD 响应过大")); }
    let bytes = fs::read(path)?;
    let text = if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 { return Err(Error::msg("AD 响应文本编码无效")); }
        let units: Vec<_> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).map_err(|_| Error::msg("AD 响应文本编码无效"))?
    } else { String::from_utf8(bytes).map_err(|_| Error::msg("AD 响应文本编码无效"))? };
    let mut fields = BTreeMap::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        if line.is_empty() { continue; }
        let (k, v) = line.split_once('=').ok_or_else(|| Error::msg("AD 响应格式无效"))?;
        if fields.insert(k.into(), v.into()).is_some() { return Err(Error::msg("AD 响应含重复字段")); }
    }
    Ok(fields)
}
fn write_fields(path: &Path, fields: &BTreeMap<String, String>) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    for (key, value) in fields { writeln!(file, "{key}={value}\r")?; }
    file.sync_all()?; drop(file);
    fs::rename(temporary, path)?; Ok(())
}

/// A running editor must never receive out-of-process replacements behind its
/// cached documents. This also protects CLI callers without a bridge configured.
pub fn require_editor_closed() -> Result<()> {
    if altium_running()? { return Err(Error::msg("文件同步需要先关闭 AD。若要在 AD 打开时同步，请切换到 AD 编辑器同步并先连接 AD；原库文件保持不变。")); }
    Ok(())
}

#[cfg(not(windows))]
fn altium_running() -> Result<bool> { Ok(false) }

#[cfg(windows)]
fn altium_running() -> Result<bool> {
    use std::{ffi::c_void, mem::{size_of, zeroed}};
    #[repr(C)]
    struct ProcessEntry { size: u32, usage: u32, pid: u32, heap: usize, module: u32,
        threads: u32, parent: u32, priority: i32, flags: u32, exe: [u16; 260] }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    unsafe {
        let handle = CreateToolhelp32Snapshot(2, 0);
        if handle as isize == -1 { return Err(std::io::Error::last_os_error().into()); }
        let mut entry: ProcessEntry = zeroed(); entry.size = size_of::<ProcessEntry>() as u32;
        let mut found = false;
        let mut valid = Process32FirstW(handle, &mut entry);
        while valid != 0 {
            let len = entry.exe.iter().position(|c| *c == 0).unwrap_or(entry.exe.len());
            let name = String::from_utf16_lossy(&entry.exe[..len]).to_ascii_lowercase();
            if matches!(name.as_str(), "x2.exe" | "dxp.exe" | "altiumdesigner.exe") { found = true; break; }
            valid = Process32NextW(handle, &mut entry);
        }
        CloseHandle(handle); Ok(found)
    }
}

#[cfg(test)]
#[path = "ad_bridge_tests.rs"]
mod tests;
