#![cfg_attr(windows, windows_subsystem = "windows")]

mod cli;
mod gui;
mod i18n;
mod prefs;
mod update;

fn main() {
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
