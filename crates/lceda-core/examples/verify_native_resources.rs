//! Opt-in live verification. Reads the source and writes only a new test directory.
use lceda_core::{native_library::{self,LibraryFormat},parallel,supplement::{self,ResourceKinds},LcedaClient};
use std::{fs,path::PathBuf,sync::atomic::AtomicBool};
fn main()->lceda_core::Result<()> {
    let source=PathBuf::from(std::env::args_os().nth(1).expect("source SchLib"));
    let parent=PathBuf::from(std::env::args_os().nth(2).expect("fresh output directory"));
    if parent.exists(){return Err(lceda_core::Error::msg("test output already exists"));}
    let before=fs::read(&source)?;let info=native_library::inspect(&source,LibraryFormat::Ad)?;
    let part=info.parts.iter().find(|p|p.name=="6N137").ok_or_else(||lceda_core::Error::msg("fixture needs 6N137"))?;
    let client=LcedaClient::new();
    let query=std::env::args().nth(3).unwrap_or_else(||"C142297".into());
    let item=client.search(&query)?.into_iter().find(|i|i.footprint_uuid().is_some()&&i.model_uuid.is_some()).ok_or_else(||lceda_core::Error::msg("no complete candidate"))?;
    let jobs=[(LibraryFormat::Ad,ResourceKinds{footprint:true,model3d:false}),(LibraryFormat::Ad,ResourceKinds{footprint:false,model3d:true}),
        (LibraryFormat::Ad,ResourceKinds{footprint:true,model3d:true}),(LibraryFormat::KiCad,ResourceKinds{footprint:true,model3d:true})];
    let results=parallel::map_bounded(&jobs,4,&AtomicBool::new(false),|(format,kinds)|supplement::download(&client,part,*format,&item,*kinds,&parent),|_,_|{});
    for(i,result)in results {
        let bundle=result?;let target=parent.join(format!("export-{i}"));supplement::export_bundle(&bundle,&target)?;
        assert_eq!(bundle.original_footprints,part.footprints);assert_eq!(bundle.kinds,jobs[i].1);
        assert!(!bundle.files.contains_key("schlib"));assert!(!bundle.files.contains_key("kicad_sym"));
        println!("PASS {} footprint={} model3d={} files={} source={} at {}",jobs[i].0.name(),jobs[i].1.footprint,jobs[i].1.model3d,bundle.files.len(),bundle.supplier_id,target.display());
    }
    assert_eq!(fs::read(&source)?,before);println!("PASS source library byte-identical; original bindings retained");Ok(())
}
