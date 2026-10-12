//! Install the native AD module using the target machine's licensed SDK.
use crate::{Error, Result};
use std::{fs, path::{Path, PathBuf}};

const ASSETS: &[(&str, &str)] = &[
    ("Host.cs", include_str!("../../../integrations/altium/plugin/Host.cs")),
    ("BridgeRuntime.cs", include_str!("../../../integrations/altium/plugin/BridgeRuntime.cs")),
    ("AdEditor.cs", include_str!("../../../integrations/altium/plugin/AdEditor.cs")),
    ("Install.ps1", include_str!("../../../integrations/altium/plugin/Install.ps1")),
    ("InstallFunctions.ps1", include_str!("../../../integrations/altium/plugin/InstallFunctions.ps1")),
    ("Lceda.Altium.Ins", include_str!("../../../integrations/altium/plugin/Lceda.Altium.Ins")),
    ("Lceda.Altium.rcs", include_str!("../../../integrations/altium/plugin/Lceda.Altium.rcs")),
    ("README_ZH.md", include_str!("../../../integrations/altium/plugin/README_ZH.md")),
];

pub fn installed(root: &Path) -> bool {
    fs::read(root.join("native-install.json")).ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|state| state["installed"].as_bool() == Some(true))
}

/// Each invocation gets an immutable source package and its own build log.
pub fn prepare_package(root: &Path) -> Result<PathBuf> {
    let dir = root.join("native-installer").join(crate::util::unique_id());
    fs::create_dir_all(&dir)?;
    for (name, source) in ASSETS { fs::write(dir.join(name), source.replace('\n', "\r\n"))?; }
    Ok(dir)
}

pub fn install(root: &Path, ad_directory: Option<&Path>, extensions_directory: Option<&Path>) -> Result<String> {
    run_installer(root, "Install", ad_directory, extensions_directory)
}

pub fn uninstall(root: &Path) -> Result<String> { run_installer(root, "Uninstall", None, None) }

fn run_installer(root: &Path, action: &str, ad_directory: Option<&Path>, extensions_directory: Option<&Path>) -> Result<String> {
    #[cfg(not(windows))]
    { let _ = (root, action, ad_directory, extensions_directory); Err(Error::msg("AD 原生插件需要在安装 AD 的 Windows 电脑上安装；Linux 可继续同步本地库文件。")) }
    #[cfg(windows)]
    {
        use std::{os::windows::process::CommandExt, process::Command};
        let package = prepare_package(root)?;
        let result = package.join("install-result.json");
        let windows = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let mut command = Command::new(windows.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        command.creation_flags(0x08000000).args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(package.join("Install.ps1")).arg("-Action").arg(action).arg("-BridgeDirectory").arg(root).arg("-ResultPath").arg(&result);
        if let Some(path) = ad_directory { command.arg("-AdDirectory").arg(path); }
        if let Some(path) = extensions_directory { command.arg("-ExtensionsDirectory").arg(path); }
        let output = command.output()?;
        let state = fs::read(&result).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let message = state.as_ref().and_then(|s| s["message"].as_str()).unwrap_or("安装器未返回结果，请查看安装日志");
        if output.status.success() && state.as_ref().is_some_and(|s| s["ok"].as_bool() == Some(true)) {
            Ok(if action == "Install" { format!("AD 原生插件已安装。启动 AD 后自动连接。\n{message}") }
                else { "AD 原生插件已卸载，绑定库文件保留。".into() })
        } else {
            let log = package.join("installer.log");
            let mut bytes = output.stdout; bytes.extend_from_slice(&output.stderr); let _ = fs::write(&log, bytes);
            Err(Error::msg(format!("{message}\n安装日志：{}", log.display())))
        }
    }
}
