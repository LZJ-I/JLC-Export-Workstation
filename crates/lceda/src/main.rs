#![cfg_attr(windows, windows_subsystem = "windows")]

mod cli;
mod bom;
mod gui;
mod i18n;
mod instance;
mod library;
mod prefs;
mod sponsor;
mod update;

static PROFILE_OVERRIDE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

fn main() {
    if let Some(code) = update::maybe_apply_update() {
        std::process::exit(code);
    }
    #[cfg(windows)]
    attach_console_if_cli();
    match cli::run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("Error: {err:#}");
            std::process::exit(2);
        }
    }
}

/// 双击打开不弹 cmd；命令行子命令仍挂到原来的终端。
#[cfg(windows)]
fn attach_console_if_cli() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let gui_only = args.is_empty()
        || args[0] == "--apply-update"
        || args.iter().all(|a| a.eq_ignore_ascii_case("gui") || a.starts_with("--lang"));
    if gui_only {
        return;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe {
        // Keep subprocess pipes and shell redirection when attaching this GUI
        // executable to a console. JSON must reach the calling agent's stdout.
        let redirected = [(-10_i32) as u32, (-11_i32) as u32, (-12_i32) as u32]
            .map(|kind| {
                let handle = GetStdHandle(kind);
                let file_type = GetFileType(handle);
                (kind, handle, file_type == 1 || file_type == 3) // disk or pipe
            });
        AttachConsole(ATTACH_PARENT_PROCESS);
        for (kind, handle, keep) in redirected {
            if keep { SetStdHandle(kind, handle); }
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn AttachConsole(dwProcessId: u32) -> i32;
    fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
    fn GetFileType(hFile: *mut std::ffi::c_void) -> u32;
    fn SetStdHandle(nStdHandle: u32, hHandle: *mut std::ffi::c_void) -> i32;
}

/// Explicit CLI profiles and screenshot runs can isolate state from the desktop.
fn profile_dir() -> Option<std::path::PathBuf> {
    if let Some(path) = PROFILE_OVERRIDE.get() { return Some(path.clone()); }
    if std::env::var_os("LCEDA_SHOT").is_some() {
        if let Some(path) = std::env::var_os("LCEDA_SHOT_PROFILE") { return Some(path.into()); }
    }
    directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant").map(|d| d.config_dir().to_path_buf())
}
