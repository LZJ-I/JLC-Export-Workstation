//! 同版本只开一份界面：已在运行则唤醒现有窗口。

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct InstanceGuard {
    pub wakeup: Arc<AtomicBool>,
    lock_path: PathBuf,
    port_path: PathBuf,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.port_path);
        let _ = fs::remove_file(&self.lock_path);
    }
}

pub fn acquire() -> Option<InstanceGuard> {
    let ver = crate::update::current_version();
    let dir = dirs()?;
    let _ = fs::create_dir_all(&dir);
    let lock_path = dir.join(format!("instance-{ver}.lock"));
    let port_path = dir.join(format!("instance-{ver}.port"));

    if !take_lock(&lock_path) {
        signal(&port_path);
        return None;
    }

    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(_) => {
            let _ = fs::remove_file(&lock_path);
            return Some(InstanceGuard {
                wakeup: Arc::new(AtomicBool::new(false)),
                lock_path,
                port_path,
            });
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(_) => {
            let _ = fs::remove_file(&lock_path);
            return Some(InstanceGuard {
                wakeup: Arc::new(AtomicBool::new(false)),
                lock_path,
                port_path,
            });
        }
    };
    let _ = fs::write(&port_path, port.to_string());
    let wakeup = Arc::new(AtomicBool::new(false));
    let flag = wakeup.clone();
    thread::spawn(move || {
        let _ = listener.set_nonblocking(false);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 1];
                    let _ = stream.read(&mut buf);
                    flag.store(true, Ordering::SeqCst);
                }
                Err(_) => break,
            }
        }
    });
    Some(InstanceGuard {
        wakeup,
        lock_path,
        port_path,
    })
}

fn dirs() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant")
        .map(|d| d.config_dir().to_path_buf())
}

fn take_lock(path: &PathBuf) -> bool {
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                return true;
            }
            Err(_) => {
                if let Some(pid) = read_pid(path) {
                    if pid_alive(pid) {
                        return false;
                    }
                }
                let _ = fs::remove_file(path);
            }
        }
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|mut f| {
            let _ = writeln!(f, "{}", std::process::id());
            true
        })
        .unwrap_or(false)
}

fn read_pid(path: &PathBuf) -> Option<u32> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

fn signal(port_path: &PathBuf) {
    let Ok(text) = fs::read_to_string(port_path) else {
        activate_existing_window();
        return;
    };
    let Ok(port) = text.trim().parse::<u16>() else {
        activate_existing_window();
        return;
    };
    if let Ok(mut stream) = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(400),
    ) {
        let _ = stream.write_all(&[1]);
        let _ = stream.flush();
    }
    activate_existing_window();
}

#[cfg(windows)]
fn activate_existing_window() {
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(cls: *const u16, title: *const u16) -> *mut std::ffi::c_void;
        fn SetForegroundWindow(hwnd: *mut std::ffi::c_void) -> i32;
        fn ShowWindow(hwnd: *mut std::ffi::c_void, cmd: i32) -> i32;
        fn IsIconic(hwnd: *mut std::ffi::c_void) -> i32;
    }
    const SW_RESTORE: i32 = 9;
    unsafe {
        let Some(hwnd) = find_app_window() else {
            return;
        };
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(not(windows))]
fn activate_existing_window() {}

#[cfg(windows)]
fn find_app_window() -> Option<*mut std::ffi::c_void> {
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(cls: *const u16, title: *const u16) -> *mut std::ffi::c_void;
    }
    for title in [
        "嘉立创导出工作站",
        "JLC Export Workstation",
        "JLC-Export",
    ] {
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = unsafe { FindWindowW(std::ptr::null(), wide.as_ptr()) };
        if !hwnd.is_null() {
            return Some(hwnd);
        }
    }
    None
}

/// Win11 用 DWM 圆角；失败时用窗口区域裁成 12px 圆角。最大化时去掉圆角。
pub fn apply_window_rounding(maximized: bool, logical_w: f32) -> bool {
    #[cfg(windows)]
    {
        apply_window_rounding_win(maximized, logical_w)
    }
    #[cfg(not(windows))]
    {
        let _ = (maximized, logical_w);
        true
    }
}

#[cfg(windows)]
fn apply_window_rounding_win(maximized: bool, logical_w: f32) -> bool {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(cls: *const u16, title: *const u16) -> *mut std::ffi::c_void;
        fn GetWindowRect(hwnd: *mut std::ffi::c_void, rc: *mut Rect) -> i32;
        fn SetWindowRgn(hwnd: *mut std::ffi::c_void, rgn: *mut std::ffi::c_void, redraw: i32) -> i32;
        fn GetDpiForWindow(hwnd: *mut std::ffi::c_void) -> u32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateRoundRectRgn(x1: i32, y1: i32, x2: i32, y2: i32, w: i32, h: i32) -> *mut std::ffi::c_void;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut std::ffi::c_void,
            attr: u32,
            ptr: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;
    const DWMWCP_DONOTROUND: u32 = 1;
    const CORNER: i32 = 12;

    unsafe {
        let Some(hwnd) = find_app_window() else {
            return false;
        };
        let pref = if maximized {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
        let dwm_ok = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const u32 as *const _,
            4,
        ) == 0;
        if maximized {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
            return true;
        }
        if dwm_ok {
            return true;
        }
        let mut rc = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rc) == 0 {
            return true;
        }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if w <= 0 || h <= 0 {
            return true;
        }
        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi >= 96 {
            dpi as f32 / 96.0
        } else if logical_w > 1.0 {
            w as f32 / logical_w
        } else {
            1.0
        };
        let dia = (CORNER as f32 * scale * 2.0).round().max(2.0) as i32;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, dia, dia);
        if !rgn.is_null() {
            SetWindowRgn(hwnd, rgn, 1);
        }
        true
    }
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
        fn GetExitCodeProcess(h: *mut std::ffi::c_void, code: *mut u32) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE
    }
}

#[cfg(not(windows))]
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let path = format!("/proc/{pid}");
    std::path::Path::new(&path).exists()
}
