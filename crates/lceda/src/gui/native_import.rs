use super::*;
use lceda_core::native_library::NativePart;
use std::{collections::{BTreeMap,BTreeSet},fs};
use lceda_core::supplement::ResourceKinds;

pub(super) struct NativeImportState {
    info: NativeLibrary,
    name: String,
    filter: String,
    selected: BTreeSet<String>,
    focused: Option<String>,
    scene: Option<SchPreview>,
    note: Option<String>,
    view: SymbolView,
    resources: ResourceKinds,
    resource_current: bool,
}
pub(super) struct PreparedImport {info:NativeLibrary,keys:Vec<String>,name:String,library:PathBuf,directory:PathBuf,resources:ResourceKinds,resource_keys:Vec<String>}

impl App {
    pub(super) fn import_native_library(&mut self,format:LibraryFormat){
        if self.busy(){return;}
        if let Some(path)=rfd::FileDialog::new().add_filter(format.name(),&[format.extension()]).pick_file(){self.load_native_library(path,format);}
    }
    pub(super) fn load_native_library(&mut self,path:PathBuf,format:LibraryFormat){
        self.goto(NavPage::Favorites);self.native_import=None;
        self.native_load=Some(Promise::spawn_thread("read-native-library",move||native_library::inspect(&path,format)));
    }
    pub(super) fn poll_native_jobs(&mut self,ctx:&egui::Context){
        if self.native_load.as_ref().is_some_and(|p|p.ready().is_some()){
            match self.native_load.take().unwrap().block_and_take(){
                Ok(info)=>{
                    let name=info.source.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let selected=info.parts.iter().map(|p|p.key.clone()).collect();
                    let focused=info.parts.first().map(|p|p.key.clone());
                    let scene=focused.as_ref().and_then(|key|native_library::preview(&info.source,info.format,key,1).ok());
                    self.native_import=Some(NativeImportState{info,name,filter:String::new(),selected,focused,scene,note:None,view:SymbolView::default(),resources:ResourceKinds::default(),resource_current:false});
                }
                Err(e)=>self.alert(e.to_string()),
            }
        }
        if self.native_save.as_ref().is_some_and(|p|p.ready().is_some()){
            match self.native_save.take().unwrap().block_and_take(){
                Ok(prepared)=>{
                    let mut next=self.lib.clone();
                    let result=next.add_native_parts(&prepared.info,&prepared.keys,&prepared.name,&prepared.library)
                        .and_then(|result|next.try_save().map(|_|result).map_err(|e|e.to_string()));
                    match result{
                        Ok((category,count))=>{
                            self.lib=next;self.fav_cat=category;self.fav_inside=true;self.library_filter.clear();self.native_import=None;
                            self.log(format!("{} · {} {count} {}",prepared.info.format.name(),i18n::t(self.lang,"native_imported"),i18n::t(self.lang,"components")));
                            let id=self.lib.favs_in(&self.fav_cat).first().map(|p|p.lcsc.clone());if let Some(id)=id{self.select_saved(&id);}
                            if prepared.resources.any(){
                                let ids=self.lib.favs_in(&self.fav_cat).into_iter().filter(|p|self.lib.native(&p.lcsc).is_some_and(|r|r.library==prepared.library&&prepared.resource_keys.contains(&r.part.key))).map(|p|p.lcsc.clone()).collect();
                                self.open_resource_assist(ids,prepared.resources);
                            }
                        }
                        Err(e)=>{let _=fs::remove_dir_all(prepared.directory);self.alert(e);}
                    }
                }
                Err(e)=>self.alert(e.to_string()),
            }
        }
        if self.native_load.is_some()||self.native_save.is_some(){ctx.request_repaint();}
    }
    pub(super) fn native_import_dialog(&mut self,ctx:&egui::Context){
        let lang=self.lang;
        if self.native_load.is_some()||self.native_save.is_some(){
            fit_window(i18n::t(lang,"native_import_title"),"native_loading",420.0).show(ctx,|ui|{
                ui.horizontal(|ui|{ui.spinner();ui.label(i18n::t(lang,if self.native_save.is_some(){"native_copying"}else{"native_reading"}));});
            });return;
        }
        let Some(state)=&mut self.native_import else{return;};
        let mut open=true;let mut cancel=false;let mut start=false;let mut focused=None;
        let width=(ctx.screen_rect().width()-72.0).clamp(640.0,900.0);
        fit_window(i18n::t(lang,"native_import_title"),"native_import_review",width).open(&mut open)
            .max_height((ctx.screen_rect().height()-76.0).max(360.0)).show(ctx,|ui|{
                ui.set_width(width-24.0);
                ui.add(egui::Label::new(theme::heading(format!("{} · {} · {} {}",state.info.source.file_name().unwrap_or_default().to_string_lossy(),state.info.format.name(),state.info.parts.len(),i18n::t(lang,"components")))).truncate()).on_hover_text(state.info.source.display().to_string());
                ui.add(egui::Label::new(egui::RichText::new(i18n::t(lang,"native_import_hint")).small().color(theme::secondary())).wrap());
                ui.add_space(10.0);
                ui.horizontal(|ui|{ui.label(i18n::t(lang,"native_library_name"));ui.add(egui::TextEdit::singleline(&mut state.name).desired_width((ui.available_width()-10.0).max(100.0)));});
                ui.add_space(8.0);
                let list_width=(ui.available_width()*0.48).max(230.0);
                let body_height=(ctx.screen_rect().height()-358.0).clamp(160.0,400.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),body_height),egui::Layout::left_to_right(egui::Align::Min),|ui|{
                    ui.set_max_height(body_height);
                    ui.allocate_ui_with_layout(egui::vec2(list_width,body_height),egui::Layout::top_down(egui::Align::Min),|ui|{
                        ui.set_width(list_width);
                        theme::search_field(ui,&mut state.filter,i18n::t(lang,"native_filter"),"native_import_filter",egui::vec2(list_width,30.0));
                        let query=state.filter.trim().to_lowercase();
                        let visible:Vec<_>=state.info.parts.iter().filter(|p|format!("{} {} {}",p.name,p.description,p.footprints.join(" ")).to_lowercase().contains(&query)).collect();
                        ui.horizontal_wrapped(|ui|{
                            if ui.small_button(i18n::t(lang,"native_select_visible")).clicked(){for p in &visible{state.selected.insert(p.key.clone());}}
                            if ui.small_button(i18n::t(lang,"native_clear_visible")).clicked(){for p in &visible{state.selected.remove(&p.key);}}
                            ui.label(format!("{} / {}",state.selected.len(),state.info.parts.len()));
                        });
                        egui::ScrollArea::vertical().id_salt("native_import_list").max_height(body_height-68.0).show(ui,|ui|{
                            ui.set_width(list_width);
                            for part in visible{
                                ui.horizontal(|ui|{
                                    let mut checked=state.selected.contains(&part.key);
                                    if ui.checkbox(&mut checked,"").changed(){if checked{state.selected.insert(part.key.clone());}else{state.selected.remove(&part.key);}}
                                    let subtitle=format!("{} {} · {} {}",part.pins,i18n::t(lang,"native_pins"),part.units,i18n::t(lang,"native_unit"));
                                    let response=paint_part_row(ui,&part.name,&subtitle,state.focused.as_deref()==Some(&part.key));
                                    if response.clicked(){focused=Some(part.key.clone());}
                                    response.on_hover_text(format!("{}\n{} · {} {} · {} {}",part.description,part.footprints.join("; "),part.pins,i18n::t(lang,"native_pins"),part.units,i18n::t(lang,"native_unit")));
                                });
                            }
                        });
                    });
                    let line=ui.allocate_exact_size(egui::vec2(1.0,body_height),egui::Sense::hover()).0;
                    ui.painter().line_segment([line.center_top(),line.center_bottom()],egui::Stroke::new(1.0_f32,theme::hairline()));
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),body_height),egui::Layout::top_down(egui::Align::Min),|ui|{
                        ui.label(theme::heading(i18n::t(lang,"schematic_preview")));
                        let rect=ui.allocate_exact_size(egui::vec2(ui.available_width(),(body_height-36.0).max(100.0)),egui::Sense::hover()).0;
                        if let Some(scene)=&state.scene{state.view.show_native(ui,rect,scene,self.alert.is_some());}
                        else{paint_well(ui,rect);ui.painter_at(rect).text(rect.center(),egui::Align2::CENTER_CENTER,state.note.as_deref().unwrap_or(i18n::t(lang,"no_symbol_preview")),egui::FontId::proportional(12.0),theme::secondary());}
                    });
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui|{
                    ui.label(egui::RichText::new(i18n::t(lang,"resource_after_import")).small().color(theme::secondary()));
                    ui.checkbox(&mut state.resources.footprint,i18n::t(lang,"resource_footprint"));ui.checkbox(&mut state.resources.model3d,i18n::t(lang,"resource_3d"));
                    if state.resources.any(){
                        if state.resource_current&&!state.focused.as_ref().is_some_and(|key|state.selected.contains(key)){state.resource_current=false;}
                        egui::ComboBox::from_id_salt("import_resource_scope").selected_text(i18n::t(lang,if state.resource_current{"resource_scope_current"}else{"resource_scope_all"})).show_ui(ui,|ui|{
                            ui.selectable_value(&mut state.resource_current,false,i18n::t(lang,"resource_scope_all"));
                            ui.add_enabled_ui(state.focused.as_ref().is_some_and(|key|state.selected.contains(key)),|ui|{ui.selectable_value(&mut state.resource_current,true,i18n::t(lang,"resource_scope_current"));});
                        });
                    }
                });
                ui.add_space(6.0);ui.separator();
                ui.horizontal(|ui|{
                    ui.label(egui::RichText::new(format!("{} / {}",state.selected.len(),state.info.parts.len())).small().color(theme::secondary()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                        start=theme::pill_button(ui,&format!("{} ({})",i18n::t(lang,"native_import_confirm"),state.selected.len()),!state.selected.is_empty()&&!state.name.trim().is_empty(),true).clicked();
                        cancel=theme::pill_button(ui,i18n::t(lang,"cancel"),true,false).clicked();
                    });
                });
            });
        if let Some(key)=focused{
            match native_library::preview(&state.info.source,state.info.format,&key,1){Ok(scene)=>{state.scene=Some(scene);state.note=None;},Err(e)=>{state.scene=None;state.note=Some(e.to_string());}}
            state.focused=Some(key);state.view.reset();
        }
        if cancel||!open||ctx.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Escape)){self.native_import=None;return;}
        if start{
            let info=state.info.clone();let keys:Vec<_>=state.selected.iter().cloned().collect();let name=state.name.clone();
            let resources=state.resources;let resource_keys=if state.resource_current{state.focused.iter().cloned().collect()}else{keys.clone()};
            let Some(profile)=crate::profile_dir()else{self.alert("无法确定器件库目录");return;};
            self.native_save=Some(Promise::spawn_thread("copy-native-library",move||{
                let parent=profile.join("library_sources");fs::create_dir_all(&parent)?;
                let directory=parent.join(lceda_core::util::unique_id());fs::create_dir(&directory)?;
                let result=(||->lceda_core::Result<PathBuf>{
                    let library=directory.join(info.source.file_name().ok_or_else(||lceda_core::Error::msg("库文件名称无效"))?);
                    native_library::copy_file(&info.source,&library)?;
                    if native_library::content_hash(&fs::read(&library)?)!=info.content_hash{return Err(lceda_core::Error::msg("源库在预览后发生变化，请重新选择库文件"));}
                    native_library::copy_companions(&info.source,&library,info.format)?;
                    native_library::inspect(&library,info.format)?;Ok(library)
                })();
                match result{Ok(library)=>Ok(PreparedImport{info,keys,name,library,directory,resources,resource_keys}),Err(e)=>{let _=fs::remove_dir_all(directory);Err(e)}}
            }));
        }
    }
    pub(super) fn queue_native_preview(&mut self){
        let Some(id)=self.native_selected.clone()else{return;};let Some(record)=self.lib.native(&id).cloned()else{return;};
        let supplements=self.lib.record(&id).map(|r|r.supplements.clone()).unwrap_or_default();let lang=self.lang;
        self.image_tex=None;self.mesh=None;self.mesh_tex=None;self.mesh_note=None;self.symbol=None;self.native_scene=None;self.symbol_note=None;
        self.symbol_view.reset();self.footprint=None;self.footprint_note=Some(i18n::t(self.lang,"native_no_footprint").into());self.footprint_view.reset();
        self.yaw=0.7;self.pitch=0.55;self.zoom=1.0;
        let unit=self.native_unit;
        self.preview=Some(Promise::spawn_thread("native-preview",move||{
            let mut data=PreviewData{native_scene:None,lcsc:Some(id),files:Vec::new(),image:None,mesh:None,mesh_note:Some(i18n::t(lang,"native_no_model").into()),symbol:None,symbol_note:None,footprint:None,footprint_note:Some(i18n::t(lang,"native_no_footprint").into())};
            match native_library::preview(&record.library,record.format,&record.part.key,unit){Ok(scene)=>data.native_scene=Some(scene),Err(e)=>data.symbol_note=Some(e.to_string())}
            if let Some(bundle)=supplements.iter().rev().find(|b|b.kinds.footprint){
                match bundle.path("footprint_ir").and_then(|p|fs::read(p).ok()).and_then(|b|serde_json::from_slice(&b).ok()){
                    Some(fp)=>{data.footprint=Some(fp);data.footprint_note=None;},None=>data.footprint_note=Some(i18n::t(lang,"resource_missing").into()),
                }
            }
            if let Some(bundle)=supplements.iter().rev().find(|b|b.kinds.model3d){
                data.mesh=bundle.path("obj").and_then(|p|fs::read(p).ok()).and_then(|b|load_preview_mesh(&b).ok());
                if data.mesh.is_none(){data.mesh_note=Some(i18n::t(lang,if bundle.path("step").is_some_and(|p|p.is_file()){"resource_step_only"}else{"resource_missing"}).into());}
            }
            data
        }));
    }
    pub(super) fn native_header(&mut self,ui:&mut egui::Ui){
        let Some(id)=self.native_selected.clone()else{return;};let Some(record)=self.lib.native(&id).cloned()else{return;};let lang=self.lang;
        let mut export=false;let mut details=false;let mut folder=false;let mut resources=false;let page=native_lcsc(&record.part);
        theme::card_frame().show(ui,|ui|{
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(theme::heading(&record.part.name).size(18.0)).truncate()).on_hover_text(&record.part.name);
            ui.add(egui::Label::new(egui::RichText::new(format!("{} · {} · {} {} · {} {}",record.format.name(),record.source.file_name().unwrap_or_default().to_string_lossy(),record.part.pins,i18n::t(lang,"native_pins"),record.part.units,i18n::t(lang,"native_unit"))).small().color(theme::secondary())).truncate());
            if !record.part.description.is_empty(){ui.add(egui::Label::new(&record.part.description).truncate()).on_hover_text(&record.part.description);}
            if let Some(saved)=self.lib.record(&id).filter(|r|!r.supplements.is_empty()){
                let footprint=saved.supplements.iter().rev().find(|b|b.kinds.footprint).map(|b|b.supplier_id.as_str()).unwrap_or("—");
                let model=saved.supplements.iter().rev().find(|b|b.kinds.model3d).map(|b|b.supplier_id.as_str()).unwrap_or("—");
                let text=format!("{} · {}: {footprint} · 3D: {model}",i18n::t(lang,"resource_preview_source"),i18n::t(lang,"resource_footprint"));
                ui.add(egui::Label::new(egui::RichText::new(&text).small().color(theme::secondary())).truncate()).on_hover_text(text);
            }
            ui.add_space(6.0);ui.horizontal_wrapped(|ui|{
                export=theme::pill_button(ui,i18n::t(lang,"native_export"),!self.busy(),true).clicked();
                details=theme::pill_button(ui,i18n::t(lang,"details"),true,false).clicked();
                resources=theme::pill_button(ui,i18n::t(lang,"resource_title"),!self.busy(),false).clicked();
                folder=theme::pill_button(ui,i18n::t(lang,"native_open_source"),true,false).clicked();
                if let Some(code)=&page{if theme::pill_button(ui,i18n::t(lang,"open_page"),true,false).clicked(){let _=webbrowser::open(&lceda_core::models::szlcsc_search_url(code));}}
            });
        });
        if export{self.ask_export_ids(vec![id.clone()]);}if details{self.detail_query.clear();self.show_detail=true;}
        if resources{self.open_resource_assist(vec![id],ResourceKinds::default());}
        if folder{if let Some(parent)=record.source.parent(){if !open_folder(parent){self.alert(format!("{}: {}",i18n::t(lang,"error"),parent.display()));}}}
    }
    pub(super) fn native_details(&mut self,ctx:&egui::Context){
        if !self.show_detail{return;}
        let Some(record)=self.native_selected.as_ref().and_then(|id|self.lib.native(id)).cloned()else{return;};let lang=self.lang;
        let mut open=true;let mut close=false;let query=&mut self.detail_query;
        fit_window(format!("{} · {}",i18n::t(lang,"details"),record.part.name),"native_details",600.0).open(&mut open).min_width(540.0).max_width(680.0).resizable(true).show(ctx,|ui|{
            ui.set_min_width(560.0);
            ui.add(egui::Label::new(record.source.display().to_string()).truncate()).on_hover_text(record.source.display().to_string());
            theme::search_field(ui,query,i18n::t(lang,"details_search"),"native_details_query",egui::vec2(ui.available_width(),30.0));
            let mut rows=record.part.properties.clone();rows.insert(i18n::t(lang,"native_footprints").into(),record.part.footprints.join("; "));
            rows.insert(i18n::t(lang,"native_description").into(),record.part.description);
            let rows:Vec<_>=rows.into_iter().map(|(name,value)|PropRow{group:PropGroup::Extra,name,value}).collect();
            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height()-240.0).clamp(180.0,420.0)).show(ui,|ui|{
                show_ad_props(ui,lang,&rows,query,None,"native_properties",false);
            });
            ui.add_space(10.0);ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{close=theme::pill_button(ui,i18n::t(lang,"ok"),true,true).clicked();});
        });
        if close||!open||ctx.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Escape)){self.show_detail=false;}
    }
    pub(super) fn native_export_dialog(&mut self,ctx:&egui::Context){
        let Some(ids)=self.native_export.clone()else{return;};let lang=self.lang;let mut open=true;let mut start=false;let mut cancel=false;
        fit_window(i18n::t(lang,"native_export"),"native_export",520.0).open(&mut open).show(ctx,|ui|{
            ui.label(theme::heading(format!("{} {}",ids.len(),i18n::t(lang,"components"))));
            ui.add(egui::Label::new(i18n::t(lang,"native_export_hint")).wrap());
            if ids.iter().any(|id|self.lib.record(id).is_some_and(|r|!r.supplements.is_empty())){ui.checkbox(&mut self.native_export_resources,i18n::t(lang,"resource_export"));}
            ui.add_space(10.0);ui.horizontal(|ui|{
                ui.add(egui::TextEdit::singleline(&mut self.out_dir).desired_width((ui.available_width()-82.0).max(120.0)));
                if ui.button(i18n::t(lang,"browse")).clicked(){if let Some(path)=rfd::FileDialog::new().pick_folder(){self.out_dir=path.display().to_string();self.persist_prefs();}}
            });
            ui.add_space(12.0);ui.horizontal(|ui|{
                cancel=theme::pill_button(ui,i18n::t(lang,"cancel"),true,false).clicked();
                start=theme::pill_button(ui,i18n::t(lang,"export_default"),!self.busy()&&!self.out_dir.trim().is_empty(),true).clicked();
            });
        });
        if cancel||!open||ctx.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Escape)){self.native_export=None;return;}
        if start{self.native_export=None;self.export_native_ids(ids);}
    }
    fn export_native_ids(&mut self,ids:Vec<String>){
        if self.busy()||!self.validate_export_target(){return;}
        let library=self.lib.clone();let output=PathBuf::from(self.out_dir.trim());let mut req=self.batch_opts.request(output.clone());self.apply_export_opts(&mut req);
        req.merge=self.batch_merge;
        let include_resources=self.native_export_resources;
        self.job=Some(Promise::spawn_thread("export-native",move||{
            let root=output.join(format!("Library-{}",lceda_core::util::unique_id()));fs::create_dir_all(&output)?;fs::create_dir(&root)?;
            let mut groups:BTreeMap<(LibraryFormat,PathBuf),Vec<String>>=BTreeMap::new();let mut online=Vec::new();
            for id in ids{if let Some(record)=library.native(&id){groups.entry((record.format,record.library.clone())).or_default().push(record.part.key.clone());}else{online.push(id);}}
            let mut messages=Vec::new();let mut failed=0;let mut exported=Vec::new();
            for (i,((format,source),keys)) in groups.into_iter().enumerate(){
                let sub=root.join(format!("{}-{}",format.name(),i+1));fs::create_dir(&sub)?;
                let destination=sub.join(source.file_name().ok_or_else(||lceda_core::Error::msg("库文件名称无效"))?);
                match native_library::export_subset(&source,format,&keys,&destination).and_then(|_|native_library::copy_companions(&source,&destination,format)){
                    Ok(())=>{
                        messages.push(format!("OK {} · {} · {}",format.name(),keys.len(),destination.display()));
                        if include_resources {
                            for(id,record)in &library.records{
                                if !record.native.as_ref().is_some_and(|r|r.library==source&&keys.contains(&r.part.key)){continue;}
                                for(i,bundle)in record.supplements.iter().enumerate(){
                                    let parent=sub.join("Resources");fs::create_dir_all(&parent)?;
                                    let target=parent.join(format!("{id}-{}",i+1));
                                    match lceda_core::supplement::export_bundle(bundle,&target){Ok(())=>messages.push(format!("OK Resources {} · {}",bundle.native_name,target.display())),Err(e)=>{failed+=1;messages.push(format!("FAIL Resources {}: {e}",bundle.native_name));}}
                                }
                            }
                        }
                    },
                    Err(e)=>{failed+=keys.len();messages.push(format!("FAIL {}: {e}",source.display()));let _=fs::remove_dir_all(sub);}
                }
            }
            if !online.is_empty(){
                let client=LcedaClient::new();let mut items=Vec::new();
                for id in online{if let Some(item)=library.cached_item(&id){items.push(item);}else{match client.search(&id){Ok(items_found)=>{if let Some(item)=items_found.into_iter().find(|i|i.lcsc_id().as_deref()==Some(&id)){items.push(item);}else{failed+=1;messages.push(format!("FAIL {id}: 未找到对应器件"));}},Err(e)=>{failed+=1;messages.push(format!("FAIL {id}: {e}"));}}}}
                req.out_dir=root.join("LCSC");let report=export_report(&client,&items,&req);failed+=report.failed;exported=report.exported;messages.push(report.message);
            }
            Ok(JobReport{message:messages.join("\n"),exported,notify:true,failed})
        }));
    }
}
pub(super) fn native_lcsc(part:&NativePart)->Option<String>{
    part.properties.iter().filter(|(k,_)|["LCSC","SUPPLIERPART","LCSC PART #","LCSC PART"].contains(&k.to_ascii_uppercase().as_str()))
        .map(|(_,v)|v.trim().to_ascii_uppercase()).find(|s|lceda_core::models::looks_like_lcsc_token(s))
}
