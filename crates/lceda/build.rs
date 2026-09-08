fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let icon = std::path::Path::new(&manifest).join("assets/icon.ico");
    if !icon.exists() {
        println!("cargo:warning=missing icon {}", icon.display());
        return;
    }

    if std::env::var_os("RC_PATH").is_none() {
        if let Ok(cache) = std::env::var("XWIN_CACHE") {
            let llvm_rc = std::path::Path::new(&cache).join("llvm-rc");
            if llvm_rc.is_file() {
                std::env::set_var("RC_PATH", llvm_rc);
            }
        }
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon.to_str().expect("icon path utf-8"));
    res.set("FileDescription", "嘉立创导出工作站 / JLC Export Workstation");
    res.set("ProductName", "JLC Export Workstation");
    res.set("OriginalFilename", "lceda.exe");
    res.set("LegalCopyright", "Copyright (c) 2026 LZJ-I");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Windows icon embed skipped: {e}");
    }
}
