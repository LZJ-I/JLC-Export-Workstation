//! Read-only library inspection, also useful for validating user-provided fixtures.
use lceda_core::native_library::{self,LibraryFormat};
fn main() -> lceda_core::Result<()> {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from).expect("library path");
    let format = if path.extension().is_some_and(|s|s.eq_ignore_ascii_case("SchLib")) {LibraryFormat::Ad}else{LibraryFormat::KiCad};
    let lib = native_library::inspect(&path,format)?;
    let mut previews = 0usize; let mut units = 0usize;
    for part in &lib.parts {
        for unit in 1..=part.units {
            let scene = native_library::preview(&path,format,&part.key,unit)
                .map_err(|e|lceda_core::Error::msg(format!("{} unit {unit}: {e}",part.name)))?;
            if !scene.strokes.is_empty()||!scene.texts.is_empty()||!scene.ellipses.is_empty(){previews+=1;}
            units+=1;
        }
    }
    println!("{}: {} parts, {units} units, {previews} nonempty previews",path.display(),lib.parts.len());
    println!("{}",serde_json::to_string_pretty(&lib.parts.iter().take(3).collect::<Vec<_>>())?);
    Ok(())
}
