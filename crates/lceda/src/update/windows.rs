use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

// ShellExecuteEx 接收参数字符串。采用 CommandLineToArgvW 的反向规则，不经过 cmd/PowerShell。
pub fn quote(text: &OsStr) -> Vec<u16> {
    let mut out = vec![b'"' as u16];
    let mut slashes = 0;
    for ch in text.encode_wide() {
        if ch == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            b'\\' as u16,
            if ch == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        out.push(ch);
    }
    out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    out.push(b'"' as u16);
    out
}

pub fn elevated_worker(exe: &Path, request: &Path) -> Result<(), String> {
    let file = wide(exe.as_os_str());
    let verb = wide(OsStr::new("runas"));
    let mut params: Vec<u16> = "--update-worker ".encode_utf16().collect();
    params.extend(quote(request.as_os_str()));
    params.push(0);
    unsafe {
        let initialized = CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        ) >= 0;
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        let launched = ShellExecuteExW(&mut info);
        let error = io::Error::last_os_error();
        if initialized {
            CoUninitialize();
        }
        if launched == 0 {
            return Err(if error.raw_os_error() == Some(1223) {
                "已取消管理员授权，更新未完成，旧版仍可继续使用。".into()
            } else {
                format!("无法启动管理员更新程序：{error}。旧版仍可继续使用。")
            });
        }
        if info.hProcess.is_null() {
            return Err("Windows 没有返回更新进程句柄，已保留旧版。".into());
        }
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0;
        let queried = GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 || queried == 0 || code != 0 {
            return Err(format!(
                "管理员更新进程未成功完成（退出码 {code}），请查看更新记录。"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

    #[test]
    fn arguments_roundtrip_through_windows_parser() {
        for value in [
            "",
            "C:\\中文目录 with spaces\\update.json",
            "C:\\ends in slash\\",
            "quote\"and\\\"slash",
            "a & b %PATH% ! ^ (x)",
        ] {
            let mut command: Vec<u16> = "worker.exe ".encode_utf16().collect();
            command.extend(quote(OsStr::new(value)));
            command.push(0);
            unsafe {
                let mut argc = 0;
                let argv = CommandLineToArgvW(command.as_ptr(), &mut argc);
                assert!(!argv.is_null());
                assert_eq!(argc, 2);
                let arg = *argv.add(1);
                let mut length = 0;
                while *arg.add(length) != 0 {
                    length += 1;
                }
                assert_eq!(
                    String::from_utf16_lossy(std::slice::from_raw_parts(arg, length)),
                    value
                );
                LocalFree(argv as _);
            }
        }
    }
}
