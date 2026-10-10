//! Exercise a copy of an existing library and compare all original data streams.
use lceda_core::{altium::SchColors,bound_ad,ir::{SymbolIr,IrRect,PartMeta},managed_ad::Entry,native_library::{self,LibraryFormat}};
use std::{collections::BTreeMap,fs,io::Read,path::{Path,PathBuf}};
fn streams(path:&Path)->BTreeMap<PathBuf,Vec<u8>>{
    let mut doc=cfb::open(path).unwrap();let entries:Vec<_>=doc.walk().filter(|e|e.is_stream()).map(|e|e.path().to_path_buf()).collect();
    entries.into_iter().map(|path|{let mut bytes=Vec::new();doc.open_stream(&path).unwrap().read_to_end(&mut bytes).unwrap();(path,bytes)}).collect()
}
fn main()->lceda_core::Result<()>{
    let mut args=std::env::args_os().skip(1);let source=PathBuf::from(args.next().expect("source SchLib"));let directory=PathBuf::from(args.next().expect("fresh test directory"));
    fs::create_dir(&directory)?;let target=directory.join("Bound.SchLib");let original=fs::read(&source)?;
    native_library::copy_file(&source,&target)?;let before=streams(&target);bound_ad::validate_binding(&target)?;
    let old=native_library::inspect(&target,LibraryFormat::Ad)?;
    for id in ["C899999991","C899999992","C899999991"]{
        let symbol=SymbolIr{name:id.into(),description:"binding regression".into(),designator:"R?".into(),meta:PartMeta{lcsc:id.into(),mpn:"10k".into(),..Default::default()},pins:vec![],rects:vec![IrRect{x1:-1.0,y1:-0.5,x2:1.0,y2:0.5}],polys:vec![],ellipses:vec![]};
        bound_ad::update(&target,Entry{id:id.into(),symbol:Some(symbol),footprint:None,step:None},SchColors::default())?;
    }
    let after=streams(&target);let mut preserved=0;
    for(path,data)in before{if path!=Path::new("/FileHeader"){assert_eq!(after.get(&path),Some(&data),"original stream changed: {}",path.display());preserved+=1;}}
    let new=native_library::inspect(&target,LibraryFormat::Ad)?;assert_eq!(new.parts.len(),old.parts.len()+2);assert_eq!(fs::read(&source)?,original);
    let keys:Vec<_>=old.parts.iter().filter(|p|["RES","共模电感","USB-C/S"].contains(&p.name.as_str())).map(|p|p.key.clone()).collect();
    if !keys.is_empty(){let subset=directory.join("Subset.SchLib");native_library::export_subset(&target,LibraryFormat::Ad,&keys,&subset)?;
        let saved=native_library::inspect(&subset,LibraryFormat::Ad)?;assert_eq!(saved.parts.len(),keys.len());let copied=streams(&subset);
        for part in saved.parts{native_library::preview(&subset,LibraryFormat::Ad,&part.key,1)?;let key=PathBuf::from(format!("/{}/Data",part.key));assert_eq!(copied.get(&key),after.get(&key));}
    }
    println!("PASS {} → {} components; {preserved} original streams unchanged; repeat update, Unicode/slash subset export and source hash preserved",old.parts.len(),new.parts.len());
    Ok(())
}
