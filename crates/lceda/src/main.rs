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
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn AttachConsole(dwProcessId: u32) -> i32;
}

/// Screenshot runs can isolate their state from the user's working library.
fn profile_dir() -> Option<std::path::PathBuf> {
    if std::env::var_os("LCEDA_SHOT").is_some() {
        if let Some(path) = std::env::var_os("LCEDA_SHOT_PROFILE") { return Some(path.into()); }
    }
    directories::ProjectDirs::from("", "LZJ-I", "lceda-assistant").map(|d| d.config_dir().to_path_buf())
}
