use super::*;
use crate::ir::{FootprintIr, PartMeta, SymbolIr};
use std::sync::Mutex;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("lceda-ad-bridge-test-{}-{}", std::process::id(), crate::util::unique_id()));
        fs::create_dir_all(&path).unwrap(); Self(path)
    }
}
impl Drop for Directory { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
#[test]
fn file_mode_is_explicit_persistent_and_cleared_by_editor_setup() {
    let dir = Directory::new();
    assert!(!file_mode(&dir.0)); assert!(!enabled(&dir.0));
    disable(&dir.0).unwrap();
    assert!(file_mode(&dir.0)); assert!(!enabled(&dir.0)); assert!(dir.0.join("stop").exists());
    enable(&dir.0).unwrap();
    assert!(!file_mode(&dir.0)); assert!(enabled(&dir.0)); assert!(!dir.0.join("stop").exists());
    disable(&dir.0).unwrap();
    assert!(install(&dir.0).unwrap().is_file());
    assert!(!file_mode(&dir.0)); assert!(enabled(&dir.0)); assert!(!dir.0.join("stop").exists());
}
fn entry() -> Entry {
    Entry { id: "C52717".into(), symbol: Some(SymbolIr {
        name: "generic".into(), description: "STM8 controller".into(), designator: "U?".into(),
        meta: PartMeta { lcsc: "C52717".into(), mpn: "STM8S003F3P6TR".into(), ..Default::default() },
        pins: vec![], rects: vec![], polys: vec![], ellipses: vec![],
    }), footprint: None, step: None }
}
fn heartbeat(root: &Path) {
    write_fields(&root.join("heartbeat.ini"), &BTreeMap::from([
        ("protocol".into(), "1".into()), ("session".into(), "test-session".into()), ("state".into(), "ready".into()),
    ])).unwrap();
}
// A protocol peer, not an AD emulator. It exercises the real workstation client
// and disk validation; it does not validate the DelphiScript or AD APIs.
fn peer(root: PathBuf, serve: impl FnOnce(BTreeMap<String, String>, PathBuf) + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let started = Instant::now();
        loop {
            if let Ok(request) = read_fields(&root.join("pending.ini")) {
                let job = PathBuf::from(unhex(&request["job"]).unwrap());
                serve(request, job); return;
            }
            assert!(started.elapsed() < Duration::from_secs(5), "no request arrived");
            thread::sleep(Duration::from_millis(10));
        }
    })
}
fn reply(request: &BTreeMap<String, String>, job: &Path, fields: &[(&str, String)]) {
    let mut response = BTreeMap::new();
    for key in ["protocol", "session", "request"] { response.insert(key.into(), request[key].clone()); }
    for (key, value) in fields { response.insert((*key).into(), value.clone()); }
    write_fields(&job.join("reply.ini"), &response).unwrap();
}

#[test]
fn unicode_protocol_and_native_windows_paths_round_trip() {
    let name = "C:\\Users\\测试\\电阻 库\\MYJ.SchLib";
    assert_eq!(unhex(&hex(name)).unwrap(), name);
    assert!(unhex("000").is_err()); assert!(unhex("D800").is_err()); assert!(unhex("GGGG").is_err());
    assert_eq!(editor_path(Path::new("\\\\?\\C:\\Users\\测试\\MYJ.SchLib")).unwrap(), "C:\\Users\\测试\\MYJ.SchLib");
    assert_eq!(editor_path(Path::new("\\\\?\\UNC\\server\\share\\Parts.SchLib")).unwrap(), "\\\\server\\share\\Parts.SchLib");
}

#[test]
fn missing_connection_does_not_modify_original_or_publish_a_request() {
    let dir = Directory::new(); let target = dir.0.join("MYJ.SchLib");
    fs::write(&target, b"original user library").unwrap();
    let bridge = Bridge::new(dir.0.clone());
    assert!(bridge.update(&target, entry(), SchColors::default()).unwrap_err().to_string().contains("尚未连接"));
    assert_eq!(fs::read(target).unwrap(), b"original user library");
    assert!(!dir.0.join("pending.ini").exists());
}

#[test]
fn late_or_foreign_success_cannot_complete_another_request() {
    let dir = Directory::new(); heartbeat(&dir.0);
    let peer = peer(dir.0.clone(), |request, job| {
        reply(&request, &job, &[("session", "old-session".into()), ("ok", "1".into()), ("verified", "1".into())]);
    });
    let error = Bridge::new(dir.0.clone()).update(&dir.0.join("MYJ.SchLib"), entry(), SchColors::default()).unwrap_err().to_string();
    peer.join().unwrap(); assert!(error.contains("会话或任务")); assert!(!dir.0.join("MYJ.SchLib").exists());
}

#[test]
fn editor_error_keeps_original_bytes_without_fallback() {
    let dir = Directory::new(); heartbeat(&dir.0); let target = dir.0.join("MYJ.SchLib");
    fs::write(&target, b"manual edits stay here").unwrap();
    let peer = peer(dir.0.clone(), |request, job| {
        reply(&request, &job, &[("ok", "0".into()), ("error", hex("Save your unsaved library edits in AD first"))]);
    });
    let error = Bridge::new(dir.0.clone()).update(&target, entry(), SchColors::default()).unwrap_err().to_string();
    peer.join().unwrap(); assert!(error.contains("unsaved"));
    assert_eq!(fs::read(target).unwrap(), b"manual edits stay here");
}

#[test]
fn success_reply_requires_the_saved_component_on_disk() {
    let dir = Directory::new(); heartbeat(&dir.0);
    let peer = peer(dir.0.clone(), |request, job| {
        reply(&request, &job, &[("ok", "1".into()), ("verified", "1".into()), ("component", request["symbol"].clone()), ("footprint", String::new())]);
    });
    let error = Bridge::new(dir.0.clone()).update(&dir.0.join("MYJ.SchLib"), entry(), SchColors::default()).unwrap_err();
    peer.join().unwrap(); assert!(!dir.0.join("MYJ.SchLib").exists()); assert!(!error.to_string().is_empty());
}

#[test]
fn successful_protocol_exchange_has_model_searchable_name_and_reports_progress() {
    let dir = Directory::new(); heartbeat(&dir.0); let target = dir.0.join("MYJ.SchLib");
    let progress = Arc::new(Mutex::new(Vec::new())); let progress_copy = progress.clone();
    let peer = peer(dir.0.clone(), |request, job| {
        let source = PathBuf::from(unhex(&request["source_sch"]).unwrap());
        let target = PathBuf::from(unhex(&request["target_sch"]).unwrap());
        let mut status = request.clone(); status.insert("phase".into(), "saving".into());
        write_fields(&job.join("status.ini"), &status).unwrap();
        fs::copy(source, target).unwrap();
        reply(&request, &job, &[("ok", "1".into()), ("verified", "1".into()), ("existing", "0".into()), ("component", request["symbol"].clone()), ("footprint", String::new())]);
    });
    let synced = Bridge::new(dir.0.clone()).with_progress(move |phase| progress_copy.lock().unwrap().push(phase.to_string()))
        .update(&target, entry(), SchColors::default()).unwrap();
    peer.join().unwrap(); assert_eq!(synced.component, "STM8S003F3P6TR_C52717"); assert!(!synced.existing);
    let saved = crate::native_library::inspect(&target, crate::native_library::LibraryFormat::Ad).unwrap();
    assert_eq!(saved.parts[0].name, synced.component); assert_eq!(saved.parts[0].description, "STM8 controller");
    let progress = progress.lock().unwrap(); assert_eq!(progress.first().unwrap(), "waiting"); assert_eq!(progress.last().unwrap(), "complete");
}

#[test]
fn existing_supplier_id_keeps_its_saved_name_and_bytes() {
    for name in ["C52717", "OLD_MODEL_C52717"] {
        let dir = Directory::new(); heartbeat(&dir.0); let target = dir.0.join("MYJ.SchLib");
        let mut saved = entry().symbol.unwrap(); saved.name = name.into();
        altium::write_schlib_with_colors(&target, &saved, SchColors::default()).unwrap();
        let before = fs::read(&target).unwrap();
        let name = name.to_string(); let returned = name.clone();
        let peer = peer(dir.0.clone(), move |request, job| {
            reply(&request, &job, &[("ok", "1".into()), ("verified", "1".into()), ("existing", "1".into()), ("component", hex(&returned)), ("footprint", String::new())]);
        });
        let synced = Bridge::new(dir.0.clone()).update(&target, entry(), SchColors::default()).unwrap();
        peer.join().unwrap(); assert!(synced.existing); assert_eq!(synced.component, name);
        assert_eq!(fs::read(target).unwrap(), before);
    }
}

#[test]
fn timeout_does_not_fall_back_and_withdraws_unclaimed_request() {
    let dir = Directory::new(); heartbeat(&dir.0); let target = dir.0.join("MYJ.SchLib");
    fs::write(&target, b"unchanged").unwrap();
    let error = Bridge::new(dir.0.clone()).update_with_timeout(&target, &mut entry(), SchColors::default(), Duration::from_millis(150)).unwrap_err().to_string();
    assert!(error.contains("超时")); assert_eq!(fs::read(target).unwrap(), b"unchanged");
    assert!(!dir.0.join("pending.ini").exists()); assert_eq!(fs::read_dir(dir.0.join("jobs")).unwrap().count(), 1);
}

#[test]
fn installation_generates_local_project_and_does_not_replace_live_scripts() {
    let dir = Directory::new(); let project = install(&dir.0).unwrap();
    assert!(project.is_file()); assert!(enabled(&dir.0));
    let script = fs::read_to_string(dir.0.join("LcedaBridge.pas")).unwrap();
    assert!(!script.contains("__IPC_ROOT_HEX__"));
    fs::write(dir.0.join("LcedaBridge.pas"), b"live script must stay").unwrap(); heartbeat(&dir.0);
    install(&dir.0).unwrap(); assert_eq!(fs::read(dir.0.join("LcedaBridge.pas")).unwrap(), b"live script must stay");
    disable(&dir.0).unwrap(); assert!(!enabled(&dir.0)); assert!(dir.0.join("stop").is_file());
}

#[test]
fn native_connection_preserves_existing_model_name_and_supplies_it_to_ad() {
    let dir = Directory::new(); heartbeat(&dir.0);
    let mut fields = read_fields(&dir.0.join("heartbeat.ini")).unwrap(); fields.insert("backend".into(), "native".into());
    write_fields(&dir.0.join("heartbeat.ini"), &fields).unwrap();
    assert!(connection(&dir.0).unwrap().native);
    let target = dir.0.join("User.SchLib");
    let mut old = entry().symbol.unwrap(); old.name = "OLD_MODEL_C52717".into();
    altium::write_schlib_with_colors(&target, &old, SchColors::default()).unwrap();
    let before = fs::read(&target).unwrap();
    let worker = peer(dir.0.clone(), |request, job| {
        assert_eq!(unhex(&request["existing_symbol"]).unwrap(), "OLD_MODEL_C52717");
        reply(&request, &job, &[("ok", "1".into()), ("verified", "1".into()),
            ("component", hex("OLD_MODEL_C52717")), ("existing", "1".into())]);
    });
    let result = Bridge::new(dir.0.clone()).update(&target, entry(), SchColors::default()).unwrap();
    worker.join().unwrap();
    assert_eq!(result.component, "OLD_MODEL_C52717"); assert_eq!(fs::read(target).unwrap(), before);
}

#[test]
fn busy_editor_remains_connected_without_replacing_its_script() {
    let dir = Directory::new(); install(&dir.0).unwrap();
    fs::write(dir.0.join("LcedaBridge.pas"), b"busy script must stay").unwrap();
    let fields = BTreeMap::from([("protocol".into(), "1".into()), ("session".into(), "test-session".into()), ("state".into(), "busy".into())]);
    let path = dir.0.join("heartbeat.ini"); write_fields(&path, &fields).unwrap();
    fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(30)).unwrap();
    assert!(connection(&dir.0).is_ok()); install(&dir.0).unwrap();
    assert_eq!(fs::read(dir.0.join("LcedaBridge.pas")).unwrap(), b"busy script must stay");
    fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(140)).unwrap();
    assert!(connection(&dir.0).is_err());
}

#[test]
fn saved_footprint_must_link_to_the_exact_embedded_step() {
    let dir = Directory::new(); let target = dir.0.join("MYJ.PcbLib");
    let fp: FootprintIr = crate::ir::footprint_ir("C52717", "test", crate::easyeda::parse_footprint(&serde_json::from_str(include_str!("../tests/fixtures/FNR3015S2R2MT_footprint_easyeda.json")).unwrap()).unwrap(), PartMeta::default());
    let step = b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;";
    altium::write_pcblib_with_step(&target, &fp, Some(step)).unwrap();
    crate::bound_ad::verify_editor_footprint(&target, "C52717", Some(step)).unwrap();
    assert!(crate::bound_ad::verify_editor_footprint(&target, "C52717", Some(b"wrong STEP")).is_err());
    assert!(crate::bound_ad::verify_editor_footprint(&target, "C5271", None).is_err());
}

#[cfg(windows)]
#[test]
fn running_editor_disables_out_of_process_library_replacement() {
    use std::process::{Command, Stdio};
    let dir = Directory::new();
    let system = PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32").join("cmd.exe");
    let fake_editor = dir.0.join("X2.exe");
    fs::copy(system, &fake_editor).unwrap();
    // The disposable process waits on its own pipe, never user input. Only its
    // name participates in process enumeration; no AD is simulated here.
    let mut child = Command::new(fake_editor).args(["/d", "/c", "pause"])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    thread::sleep(Duration::from_millis(100));
    let result = require_editor_closed();
    let _ = child.kill(); let _ = child.wait();
    assert!(result.unwrap_err().to_string().contains("关闭 AD"));
}
