use super::transaction::{self, Failure, Installed, Request};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const PROTOCOL: u32 = 1;
const BOOT_ENV: &str = "LCEDA_UPDATE_BOOT";

#[derive(Debug, Serialize, Deserialize)]
struct Identity {
    protocol: u32,
    version: String,
    executable: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
enum Work {
    Install(Request),
    Rollback(Installed),
}

#[derive(Debug, Serialize, Deserialize)]
enum Done {
    Installed(Installed),
    RolledBack,
}

#[derive(Debug, Serialize, Deserialize)]
struct Boot {
    nonce: String,
    version: String,
    installed: Installed,
}

#[derive(Debug, Serialize, Deserialize)]
struct Started {
    nonce: String,
    identity: Identity,
}

fn identity() -> Result<Identity, String> {
    Ok(Identity {
        protocol: PROTOCOL,
        version: super::current_version().into(),
        executable: std::env::current_exe()
            .and_then(|p| p.canonicalize())
            .map_err(|e| e.to_string())?,
    })
}

fn new_session() -> Result<tempfile::TempDir, String> {
    // 临时目录不可写时退到本用户配置目录；两者都失败时保留当前进程并报错。
    if let Ok(dir) = tempfile::Builder::new().prefix("lceda-update-").tempdir() {
        return Ok(dir);
    }
    let root = crate::profile_dir()
        .ok_or("无法找到可写的更新目录")?
        .join("updates");
    fs::create_dir_all(&root).map_err(|e| format!("无法创建更新目录：{e}"))?;
    tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())
}

fn wait_child(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("程序退出码：{status}"))
            };
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("启动新程序超时，可能被系统或安全软件拦截".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn probe(candidate: &Path, version: &str, session: &Path) -> Result<(), String> {
    let output = session.join("probe.json");
    let mut child = Command::new(candidate)
        .arg("--update-probe")
        .arg(&output)
        .env_remove(BOOT_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("下载的新程序无法在本机运行：{e}"))?;
    wait_child(&mut child, Duration::from_secs(20))?;
    let found: Identity = transaction::read_json(&output)?;
    if found.protocol != PROTOCOL
        || found.version != version
        || found.executable != candidate.canonicalize().map_err(|e| e.to_string())?
    {
        return Err(format!(
            "更新包版本或启动路径不符：期望 {version}，实际 {}。已保留旧版。",
            found.version
        ));
    }
    Ok(())
}

fn run_work(work: &Work) -> Result<Done, Failure> {
    match work {
        Work::Install(request) => transaction::install(request).map(Done::Installed),
        Work::Rollback(installed) => transaction::rollback(installed).map(|()| Done::RolledBack),
    }
}

fn perform(work: Work, candidate: &Path, session: &Path) -> Result<Done, String> {
    match run_work(&work) {
        Ok(done) => Ok(done),
        Err(error) => {
            #[cfg(windows)]
            if error.needs_elevation() {
                let path = session.join("work.json");
                let result_path = session.join("result.json");
                let _ = fs::remove_file(&result_path);
                transaction::write_json(&path, &work).map_err(|e| e.to_string())?;
                // 仅提权安装操作。主界面和重启的新程序仍由当前用户进程启动。
                super::windows::elevated_worker(candidate, &path)?;
                let result: Result<Done, Failure> = transaction::read_json(&result_path)?;
                return result.map_err(|e| e.to_string());
            }
            let _ = (candidate, session);
            Err(error.to_string())
        }
    }
}

fn start_confirmed(
    installed: &Installed,
    version: &str,
    args: &[OsString],
    session: &Path,
) -> Result<(), String> {
    let boot = Boot {
        nonce: lceda_core::util::unique_id(),
        version: version.into(),
        installed: installed.clone(),
    };
    let boot_path = session.join("boot.json");
    transaction::write_json(&boot_path, &boot).map_err(|e| e.to_string())?;
    let ack = session.join("started.json");
    let _ = fs::remove_file(&ack);
    let mut child = Command::new(&installed.dest)
        .args(args)
        .env(BOOT_ENV, &boot_path)
        .spawn()
        .map_err(|e| format!("新版无法启动：{e}"))?;
    let start = Instant::now();
    let error = loop {
        if let Ok(started) = transaction::read_json::<Started>(&ack) {
            if started.nonce == boot.nonce
                && started.identity.protocol == PROTOCOL
                && started.identity.version == version
                && started.identity.executable == installed.dest
            {
                // 只有协调进程确认新版就绪，才授权新版清理旧备份。
                if let Err(error) =
                    transaction::write_json(&session.join("committed.json"), &boot.nonce)
                {
                    break format!("无法确认更新完成：{error}");
                }
                return Ok(());
            }
            break "新版启动确认与目标版本或程序路径不一致".to_string();
        }
        match child.try_wait() {
            Ok(Some(status)) => break format!("新版在窗口就绪前退出（{status}）"),
            Err(e) => break format!("无法确认新版运行状态：{e}"),
            _ => {}
        }
        if start.elapsed() >= Duration::from_secs(45) {
            break "新版窗口启动超时".to_string();
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // 仅终止本次启动且未确认就绪的子进程，不按进程名结束其他程序。
    let _ = child.kill();
    let _ = child.wait();
    Err(error)
}

fn apply_candidate(
    candidate: &Path,
    dest: &Path,
    version: &str,
    args: &[OsString],
    session: &Path,
) -> Result<(), String> {
    probe(candidate, version, session)?;
    let request = Request {
        dest: dest
            .canonicalize()
            .map_err(|e| format!("无法找到正在使用的程序：{e}"))?,
        candidate: candidate.canonicalize().map_err(|e| e.to_string())?,
        old_hash: transaction::file_digest(dest).map_err(|e| e.to_string())?,
        new_hash: transaction::file_digest(candidate).map_err(|e| e.to_string())?,
    };
    let Done::Installed(installed) = perform(Work::Install(request), candidate, session)? else {
        return Err("更新进程没有返回安装结果".into());
    };
    if let Err(error) = start_confirmed(&installed, version, args, session) {
        match perform(Work::Rollback(installed.clone()), candidate, session) {
            Ok(_) => return Err(format!("{error}。已恢复旧版，请重试或手动下载更新。")),
            Err(restore) => {
                return Err(format!(
                    "{error}\n恢复失败：{restore}\n完整旧版保留在：{}",
                    installed.backup.display()
                ))
            }
        }
    }
    Ok(())
}

pub fn apply(bytes: &[u8], version: &str) -> Result<(), String> {
    let session = new_session()?;
    let candidate = session.path().join(if cfg!(windows) {
        "candidate.exe"
    } else {
        "candidate"
    });
    fs::write(&candidate, bytes).map_err(|e| format!("无法写入完整的新程序：{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let dest = std::env::current_exe().map_err(|e| e.to_string())?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Err(error) = apply_candidate(&candidate, &dest, version, &args, session.path()) {
        return Err(report_error(&error));
    }
    // 新版窗口已确认；此时才关闭旧进程。新版负责在旧进程退出后清理备份。
    let _ = session.keep();
    std::process::exit(0);
}

pub fn maybe_handle() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    let flag = args.next()?;
    if flag == "--update-probe" {
        let result = (|| {
            let path = PathBuf::from(args.next().ok_or("缺少探测输出路径")?);
            transaction::write_json(&path, &identity()?).map_err(|e| e.to_string())
        })();
        return Some(if result.is_ok() { 0 } else { 2 });
    }
    if flag == "--update-worker" {
        let result = (|| {
            let path = PathBuf::from(args.next().ok_or("缺少更新任务路径")?);
            let work: Work = transaction::read_json(&path)?;
            // 安装助手只安装自己，不能用请求文件指定另一个待提权执行的程序。
            if let Work::Install(request) = &work {
                if request
                    .candidate
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    != identity()?.executable
                {
                    return Err("更新助手路径不一致".into());
                }
            }
            let result = run_work(&work);
            transaction::write_json(&path.with_file_name("result.json"), &result)
                .map_err(|e| e.to_string())
        })();
        return Some(if result.is_ok() { 0 } else { 2 });
    }
    // 兼容 0.9.1 及更早版本启动新版 EXE 时使用的入口。
    if flag == "--apply-update" {
        let dest = args.next().map(PathBuf::from);
        let rest: Vec<_> = args.collect();
        let result = (|| {
            let dest = dest.as_ref().ok_or("缺少安装目标")?;
            let session = new_session()?;
            let candidate = std::env::current_exe().map_err(|e| e.to_string())?;
            apply_candidate(
                &candidate,
                dest,
                super::current_version(),
                &rest,
                session.path(),
            )?;
            let _ = session.keep();
            Ok::<_, String>(())
        })();
        return Some(match result {
            Ok(()) => 0,
            Err(error) => {
                let message = report_error(&error);
                // 旧版父进程已经退出，失败后恢复可使用的窗口并明确显示原因。
                if let Some(dest) = dest {
                    let _ = Command::new(dest).args(&rest).env_remove(BOOT_ENV).spawn();
                }
                show_error(&message);
                2
            }
        });
    }
    None
}

pub fn confirm_startup() {
    static CONFIRMED: AtomicBool = AtomicBool::new(false);
    if CONFIRMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(path) = std::env::var_os(BOOT_ENV).map(PathBuf::from) else {
        return;
    };
    let result = (|| {
        let boot: Boot = transaction::read_json(&path)?;
        let identity = identity()?;
        if boot.version != identity.version
            || boot
                .installed
                .dest
                .canonicalize()
                .map_err(|e| e.to_string())?
                != identity.executable
        {
            return Err("启动的程序与更新目标不一致".to_string());
        }
        transaction::write_json(
            &path.with_file_name("started.json"),
            &Started {
                nonce: boot.nonce.clone(),
                identity,
            },
        )
        .map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let session = path.parent().unwrap();
            let mut committed = false;
            for _ in 0..60 {
                std::thread::sleep(Duration::from_secs(1));
                if !committed {
                    committed = transaction::read_json::<String>(&session.join("committed.json"))
                        .is_ok_and(|nonce| nonce == boot.nonce);
                    if !committed {
                        continue;
                    }
                }
                if transaction::cleanup(&boot.installed).is_ok() {
                    let _ = fs::remove_dir_all(session);
                    break;
                }
            }
        });
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        eprintln!("{}", report_error(&error));
        std::process::exit(2);
    }
}

fn report_error(error: &str) -> String {
    let Some(root) = crate::profile_dir() else {
        return error.into();
    };
    let _ = fs::create_dir_all(&root);
    let path = root.join("update-last-error.txt");
    let text = format!(
        "版本：{}\n程序：{:?}\n{error}\n",
        super::current_version(),
        std::env::current_exe()
    );
    if fs::write(&path, text).is_ok() {
        format!("{error}\n更新记录：{}", path.display())
    } else {
        error.into()
    }
}

fn show_error(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    rfd::MessageDialog::new()
        .set_title("更新未完成 / Update not completed")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::process::CommandExt;

    #[test]
    #[ignore = "only spawned by windows_update_roundtrip"]
    fn hold_executable() {
        let ready = std::env::var_os("LCEDA_UPDATE_HOLD_READY").expect("isolated process only");
        fs::write(ready, b"ready").unwrap();
        std::thread::sleep(Duration::from_secs(90));
    }

    struct Held(Child);
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    #[ignore = "requires LCEDA_UPDATE_TEST_BINARY pointing to the built Windows GUI"]
    fn windows_update_roundtrip() {
        let candidate =
            PathBuf::from(std::env::var_os("LCEDA_UPDATE_TEST_BINARY").expect("built Windows exe"));
        let evidence = PathBuf::from(
            std::env::var_os("LCEDA_UPDATE_EVIDENCE").expect("isolated evidence directory"),
        );
        fs::create_dir_all(&evidence).unwrap();
        let new_hash = transaction::file_digest(&candidate).unwrap();
        let mut results = Vec::new();
        for case in [
            "running",
            "readonly",
            "long-path",
            "relaunch-failed",
            "wrong-version",
            "locked-file",
            "elevated-worker",
            "legacy-entry",
        ] {
            let root = tempfile::Builder::new()
                .prefix("更新 中文 & 空格 ")
                .tempdir()
                .unwrap();
            let mut folder = root.path().join(case);
            if case == "long-path" {
                for _ in 0..8 {
                    folder = folder.join("long-path-中文-012345678901234567890");
                }
            }
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("user-library.json"),
                b"keep user libraries unchanged",
            )
            .unwrap();
            let dest = folder.join("嘉立创 工作站.exe");
            fs::copy(std::env::current_exe().unwrap(), &dest).unwrap();
            let old_hash = transaction::file_digest(&dest).unwrap();
            let session = tempfile::tempdir().unwrap();
            let ready = root.path().join("holder-ready");
            let mut held = Held(
                Command::new(dest.canonicalize().unwrap())
                    .current_dir(root.path())
                    .args([
                        "--exact",
                        "update::runtime::tests::hold_executable",
                        "--ignored",
                    ])
                    .env("LCEDA_UPDATE_HOLD_READY", &ready)
                    .creation_flags(0x08000000)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let start = Instant::now();
            while !ready.exists() {
                assert!(start.elapsed() < Duration::from_secs(15));
                std::thread::sleep(Duration::from_millis(50));
            }
            if case == "readonly" {
                let mut p = fs::metadata(&dest).unwrap().permissions();
                p.set_readonly(true);
                fs::set_permissions(&dest, p).unwrap();
            }
            let file_lock = if case == "locked-file" {
                Some(
                    fs::OpenOptions::new()
                        .read(true)
                        .share_mode(1)
                        .open(&dest)
                        .unwrap(),
                )
            } else {
                None
            };
            // 仅此串行、显式启用的测试设置进程环境；普通回归不改变 GUI 环境。
            std::env::set_var("LCEDA_SHOT", evidence.join(format!("{case}.png")));
            std::env::set_var("LCEDA_SHOT_PROFILE", root.path().join("isolated-profile"));
            std::env::set_var("LCEDA_LANG", "zh");
            let args = if case == "relaunch-failed" {
                vec![OsString::from("--not-a-valid-command")]
            } else {
                vec![OsString::from("gui")]
            };
            let expected = if case == "wrong-version" {
                "999.0.0"
            } else {
                super::super::current_version()
            };
            let result = if case == "elevated-worker" {
                let helper_dir = root.path().join("安装器 & 中文 (test)");
                fs::create_dir_all(&helper_dir).unwrap();
                let helper = helper_dir.join("candidate.exe");
                fs::copy(&candidate, &helper).unwrap();
                let path = helper_dir.join("安装任务.json");
                let work = Work::Install(Request {
                    dest: dest.canonicalize().unwrap(),
                    candidate: helper.canonicalize().unwrap(),
                    old_hash: old_hash.clone(),
                    new_hash: new_hash.clone(),
                });
                transaction::write_json(&path, &work).unwrap();
                super::super::windows::elevated_worker(&helper, &path).unwrap();
                let done: Result<Done, Failure> =
                    transaction::read_json(&path.with_file_name("result.json")).unwrap();
                let Done::Installed(installed) = done.unwrap() else {
                    panic!("no installation result");
                };
                start_confirmed(&installed, expected, &args, session.path())
            } else if case == "legacy-entry" {
                let mut helper = Command::new(&candidate)
                    .arg("--apply-update")
                    .arg(dest.canonicalize().unwrap())
                    .args(&args)
                    .spawn()
                    .unwrap();
                wait_child(&mut helper, Duration::from_secs(60))
            } else {
                apply_candidate(&candidate, &dest, expected, &args, session.path())
            };
            assert!(
                held.0.try_wait().unwrap().is_none(),
                "old process was terminated: {case}"
            );
            let should_succeed = matches!(
                case,
                "running" | "readonly" | "long-path" | "elevated-worker" | "legacy-entry"
            );
            assert_eq!(result.is_ok(), should_succeed, "{case}: {result:?}");
            assert_eq!(
                transaction::file_digest(&dest).unwrap(),
                if should_succeed {
                    new_hash.clone()
                } else {
                    old_hash
                }
            );
            if should_succeed {
                let start = Instant::now();
                while !evidence.join(format!("{case}.png")).exists() {
                    assert!(
                        start.elapsed() < Duration::from_secs(15),
                        "new GUI screenshot missing: {case}"
                    );
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            results.push(serde_json::json!({"case":case,"success":result.is_ok(),"old_process_alive":true,"error":result.err()}));
            assert_eq!(
                fs::read(folder.join("user-library.json")).unwrap(),
                b"keep user libraries unchanged"
            );
            drop(file_lock);
            drop(held);
        }
        std::env::remove_var("LCEDA_SHOT");
        std::env::remove_var("LCEDA_SHOT_PROFILE");
        std::env::remove_var("LCEDA_LANG");
        let mut report = fs::File::create(evidence.join("windows-update-results.json")).unwrap();
        report
            .write_all(serde_json::to_string_pretty(&results).unwrap().as_bytes())
            .unwrap();
    }
}
