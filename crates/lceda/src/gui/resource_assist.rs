use super::*;
use crate::library::NativeRecord;
use lceda_core::{parallel, supplement::{self, ResourceBundle, ResourceKinds}};
use std::collections::BTreeMap;

struct ResourceRow {
    id: String, native: NativeRecord, query: String, included: bool,
    candidates: Vec<SearchItem>, selected: Option<usize>, confirmed: bool, note: Option<String>,
}
impl ResourceRow {
    fn chosen(&self) -> Option<&SearchItem> { self.selected.and_then(|i| self.candidates.get(i)) }
    fn exact(&self, kinds: ResourceKinds) -> Option<usize> {
        let manufacturer=self.native.part.properties.iter().find(|(k,v)|k.eq_ignore_ascii_case("Manufacturer")&&!v.trim().is_empty()).map(|(_,v)|v.trim());
        let matches: Vec<_> = self.candidates.iter().enumerate().filter(|(_,i)| kinds.available(i) &&
            (native_import::native_lcsc(&self.native.part).as_ref().is_some_and(|id|i.lcsc_id().as_ref()==Some(id)) ||
             i.name().eq_ignore_ascii_case(self.query.trim())&&manufacturer.is_none_or(|m|i.manufacturer_label().eq_ignore_ascii_case(m))))
            .map(|(i,_)|i).collect();
        (matches.len()==1).then(||matches[0])
    }
}
type SearchResults = Vec<(String, Vec<String>, Result<Vec<SearchItem>, String>)>;
struct DownloadResults { saved: Vec<(String,ResourceBundle)>, failed: Vec<(String,String)> }
struct CandidatePreview { footprint: Option<FootprintIr>, mesh: Option<Mesh>, note: String }
pub(super) struct ResourceAssist {
    rows: Vec<ResourceRow>, focused: usize, kinds: ResourceKinds, filter: String,
    search: Option<Promise<SearchResults>>, download: Option<Promise<DownloadResults>>,
    cancel: Arc<AtomicBool>, progress: Arc<Mutex<(usize,usize)>>, note: String, close_when_idle: bool,
    preview: Option<Promise<CandidatePreview>>, preview_key: Option<String>, footprint: Option<FootprintIr>,
    preview_pending: bool,
    mesh: Option<Arc<Mesh>>, texture: Option<TextureHandle>, footprint_view: FootprintView,
    show_3d: bool, yaw: f32, pitch: f32, zoom: f32,
}
impl ResourceAssist {
    pub(super) fn is_busy(&self) -> bool { self.search.is_some() || self.download.is_some() }
    fn reset_preview(&mut self) {
        self.preview_key=None; self.preview_pending=false; self.footprint=None; self.mesh=None; self.texture=None; self.footprint_view.reset();
        self.yaw=0.7; self.pitch=0.55; self.zoom=1.0;
    }
    fn start_search(&mut self, only_current: bool) {
        let mut queries=BTreeMap::<String,(String,Vec<String>)>::new();let focused=self.focused;
        for (_,row) in self.rows.iter_mut().enumerate().filter(|(i,r)|r.included && (!only_current || *i==focused) && (only_current || !r.confirmed)) {
            if row.query.trim().is_empty(){continue;}
            row.note=None; row.confirmed=false; row.candidates.clear(); row.selected=None;
            queries.entry(row.query.trim().to_lowercase()).or_insert_with(||(row.query.trim().to_string(),Vec::new())).1.push(row.id.clone());
        }
        let queries:Vec<_>=queries.into_values().collect(); if queries.is_empty(){return;}
        self.cancel.store(false,Ordering::Relaxed); *self.progress.lock().unwrap()=(0,queries.len());
        let cancel=self.cancel.clone(); let progress=self.progress.clone(); self.reset_preview();
        self.search=Some(Promise::spawn_thread("resource-search",move||{
            let client=LcedaClient::new(); let mut done=0;
            parallel::map_bounded(&queries,parallel::NETWORK_WORKERS,&cancel,
                |(q,_)|client.search(q).map_err(|e|e.to_string()), |_,_|{done+=1;*progress.lock().unwrap()=(done,queries.len());})
                .into_iter().map(|(i,result)|(queries[i].0.to_lowercase(),queries[i].1.clone(),result)).collect()
        }));
    }
    fn start_preview(&mut self) {
        if self.preview.is_some(){return;}
        self.preview_pending=false;
        let Some(item)=self.rows.get(self.focused).and_then(ResourceRow::chosen).cloned()else{return;};
        let key=preview_key(&item,self.kinds); if self.preview_key.as_deref()==Some(&key){return;}
        self.preview_key=Some(key); let kinds=self.kinds;
        self.preview=Some(Promise::spawn_thread("resource-preview",move||{
            let client=LcedaClient::new(); let mut note=Vec::new();
            let footprint=if kinds.footprint {match supplement::fetch_footprint(&client,&item){Ok((_,fp))=>Some(fp),Err(e)=>{note.push(e.to_string());None}}}else{None};
            let mesh=if kinds.model3d {match client.download_obj_bytes(&item).map_err(|e|e.to_string()).and_then(|b|load_preview_mesh(&b)) {Ok(m)=>Some(m),Err(e)=>{note.push(e);None}}}else{None};
            CandidatePreview{footprint,mesh,note:note.join("\n")}
        }));
    }
}
fn preview_key(item:&SearchItem,kinds:ResourceKinds)->String{format!("{}:{}:{}",item.lcsc_id().unwrap_or_default(),kinds.footprint,kinds.model3d)}
fn initial_query(native:&NativeRecord)->String {
    if let Some(id)=native_import::native_lcsc(&native.part){return id;}
    for wanted in ["MPN","Manufacturer Part Number","型号","Comment","Value"] {
        if let Some((_,v))=native.part.properties.iter().find(|(k,v)|k.eq_ignore_ascii_case(wanted)&&!v.trim().is_empty()&&!v.starts_with('=')){
            if !matches!(v.as_str(),"*"|"~"|"U?"|"R?"|"C?"){return v.clone();}
        }
    }
    native.part.name.clone()
}

impl App {
    pub(super) fn open_resource_assist(&mut self,ids:Vec<String>,kinds:ResourceKinds) {
        if self.busy(){return;}
        let rows:Vec<_>=ids.into_iter().filter_map(|id|{let native=self.lib.native(&id)?.clone();Some(ResourceRow{id,query:initial_query(&native),native,included:true,candidates:Vec::new(),selected:None,confirmed:false,note:None})}).collect();
        if rows.is_empty(){return;}
        self.resource_assist=Some(ResourceAssist{rows,focused:0,kinds,filter:String::new(),search:None,download:None,
            cancel:Arc::new(AtomicBool::new(false)),progress:Arc::new(Mutex::new((0,0))),note:String::new(),close_when_idle:false,
            preview:None,preview_key:None,preview_pending:false,footprint:None,mesh:None,texture:None,footprint_view:FootprintView::default(),show_3d:!kinds.footprint,
            yaw:0.7,pitch:0.55,zoom:1.0});
    }
    pub(super) fn poll_resource_assist(&mut self,ctx:&egui::Context) {
        let Some(mut state)=self.resource_assist.take()else{return;};
        if state.search.as_ref().is_some_and(|p|p.ready().is_some()){
            for(query,ids,result)in state.search.take().unwrap().block_and_take(){
                for row in state.rows.iter_mut().filter(|r|ids.contains(&r.id)&&r.query.trim().eq_ignore_ascii_case(&query)&&r.included){
                    match &result {Ok(items)=>{row.candidates=items.clone();row.selected=None;row.confirmed=false;},Err(e)=>row.note=Some(e.clone())}
                }
            }
        }
        if state.preview.as_ref().is_some_and(|p|p.ready().is_some()){
            let result=state.preview.take().unwrap().block_and_take();
            let current=state.rows.get(state.focused).and_then(ResourceRow::chosen).map(|i|preview_key(i,state.kinds));
            if current==state.preview_key {state.footprint=result.footprint;state.mesh=result.mesh.map(Arc::new);if !result.note.is_empty(){state.note=result.note;}}
        }
        if state.download.as_ref().is_some_and(|p|p.ready().is_some()){
            let result=state.download.take().unwrap().block_and_take(); let mut next=self.lib.clone();
            let mut save_error=None;
            for(id,bundle)in &result.saved {if let Err(e)=next.add_supplement(id,bundle.clone()){save_error=Some(e);break;}}
            if save_error.is_none()&&!result.saved.is_empty(){if let Err(e)=next.try_save(){save_error=Some(e.to_string());}}
            if let Some(e)=save_error {
                for(_,bundle)in &result.saved{let _=std::fs::remove_dir_all(&bundle.directory);} self.alert(e);
            }else{
                self.lib=next;
                for(id,_)in &result.saved {if let Some(row)=state.rows.iter_mut().find(|r|&r.id==id){row.included=false;row.confirmed=false;row.note=Some(i18n::t(self.lang,"resource_saved").into());}}
                for(id,error)in &result.failed{if let Some(row)=state.rows.iter_mut().find(|r|&r.id==id){row.note=Some(error.clone());}}
                state.note=format!("{} {} · {} {}",result.saved.len(),i18n::t(self.lang,"resource_saved"),result.failed.len(),i18n::t(self.lang,"resource_failed"));
                self.log(state.note.clone());
                if self.native_selected.as_ref().is_some_and(|id|result.saved.iter().any(|(saved,_)|saved==id)){self.queue_native_preview();}
            }
        }
        if state.preview.is_none()&&state.preview_pending{state.start_preview();}
        if state.is_busy()||state.preview.is_some(){ctx.request_repaint();}
        if !(state.close_when_idle&&!state.is_busy()){self.resource_assist=Some(state);}
    }
    pub(super) fn resource_assist_dialog(&mut self,ctx:&egui::Context) {
        let Some(state)=&mut self.resource_assist else{return;}; let lang=self.lang; let running=state.is_busy();
        let mut open=true;let mut close=false;let mut search=false;let mut search_one=false;let mut download=false;
        let mut confirm_exact=false;let mut preview=false;let mut reset_preview=false;let mut focus=None;
        let width=(ctx.screen_rect().width()-72.0).clamp(640.0,1000.0);
        let body_height=(ctx.screen_rect().height()-300.0).clamp(200.0,460.0);
        fit_window(i18n::t(lang,"resource_title"),"resource_assist",width).open(&mut open).max_height(ctx.screen_rect().height()-64.0).show(ctx,|ui|{
            ui.set_width(width-24.0);
            ui.add(egui::Label::new(i18n::t(lang,"resource_hint")).wrap());
            ui.add_space(6.0);
            ui.add_enabled_ui(!running,|ui|{ui.horizontal_wrapped(|ui|{
                let before=state.kinds;
                ui.checkbox(&mut state.kinds.footprint,i18n::t(lang,"resource_footprint"));ui.checkbox(&mut state.kinds.model3d,i18n::t(lang,"resource_3d"));
                if before!=state.kinds{for row in &mut state.rows{row.confirmed=false;}state.reset_preview();state.show_3d=!state.kinds.footprint;}
                ui.add_space(8.0);
                search=theme::pill_button(ui,i18n::t(lang,"resource_search_all"),state.kinds.any(),false).clicked();
                confirm_exact=theme::pill_button(ui,i18n::t(lang,"resource_confirm_exact"),state.kinds.any(),false).on_hover_text(i18n::t(lang,"resource_confirm_hint")).clicked();
            });});
            ui.add_space(8.0);
            let left=(ui.available_width()*0.40).clamp(260.0,380.0);
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),body_height),egui::Layout::left_to_right(egui::Align::Min),|ui|{
                ui.set_max_height(body_height);
                ui.allocate_ui_with_layout(egui::vec2(left,body_height),egui::Layout::top_down(egui::Align::Min),|ui|{
                    ui.set_width(left);
                    theme::search_field(ui,&mut state.filter,i18n::t(lang,"native_filter"),"resource_filter",egui::vec2(left,30.0));
                    ui.add_enabled_ui(!running,|ui|{ui.horizontal_wrapped(|ui|{
                        if ui.small_button(i18n::t(lang,"resource_select_all")).clicked(){for row in &mut state.rows{row.included=true;}}
                        if ui.small_button(i18n::t(lang,"resource_select_one")).clicked(){for(i,row)in state.rows.iter_mut().enumerate(){row.included=i==state.focused;}}
                        if ui.small_button(i18n::t(lang,"native_clear_visible")).clicked(){for row in &mut state.rows{row.included=false;}}
                    });});
                    let filter=state.filter.trim().to_lowercase();
                    egui::ScrollArea::vertical().id_salt("resource_rows").max_height(body_height-68.0).show(ui,|ui|{
                        ui.set_width(left);
                        for(index,row)in state.rows.iter_mut().enumerate(){
                            if !filter.is_empty()&&!format!("{} {} {}",row.native.part.name,row.query,row.native.part.footprints.join(" ")).to_lowercase().contains(&filter){continue;}
                            ui.horizontal(|ui|{
                                ui.add_enabled(!running,egui::Checkbox::without_text(&mut row.included));
                                let status=row.note.clone().unwrap_or_else(||if row.confirmed{i18n::t(lang,"resource_confirmed").into()}else if row.candidates.is_empty(){i18n::t(lang,"resource_not_searched").into()}else{format!("{} {}",row.candidates.len(),i18n::t(lang,"resource_candidates"))});
                                if paint_part_row(ui,&row.native.part.name,&status,index==state.focused).clicked(){focus=Some(index);}
                            });
                        }
                    });
                });
                let line=ui.allocate_exact_size(egui::vec2(1.0,body_height),egui::Sense::hover()).0;
                ui.painter().line_segment([line.center_top(),line.center_bottom()],egui::Stroke::new(1.0_f32,theme::hairline()));
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),body_height),egui::Layout::top_down(egui::Align::Min),|ui|{
                    let row=&mut state.rows[state.focused];
                    ui.add(egui::Label::new(theme::heading(&row.native.part.name)).truncate());
                    let links=if row.native.part.footprints.is_empty(){i18n::t(lang,"resource_no_link").to_string()}else{row.native.part.footprints.join("; ")};
                    ui.add(egui::Label::new(egui::RichText::new(format!("{}: {links}",i18n::t(lang,"native_footprints"))).small().color(theme::secondary())).truncate()).on_hover_text(links);
                    ui.add_enabled_ui(!running,|ui|{
                        ui.horizontal(|ui|{
                            let changed=ui.add(egui::TextEdit::singleline(&mut row.query).hint_text(i18n::t(lang,"resource_query")).desired_width((ui.available_width()-74.0).max(100.0))).changed();
                            if changed{row.candidates.clear();row.selected=None;row.confirmed=false;reset_preview=true;}
                            search_one=theme::pill_button(ui,i18n::t(lang,"search"),state.kinds.any()&&row.included&&!row.query.trim().is_empty(),false).clicked();
                        });
                        let selected=row.chosen().map(candidate_label).unwrap_or_else(||i18n::t(lang,"resource_choose").to_string());
                        let old=row.selected;
                        let candidate_width=ui.available_width()-10.0;
                        egui::ComboBox::from_id_salt("resource_candidate").width(candidate_width).truncate().selected_text(selected.clone()).show_ui(ui,|ui|{
                            ui.set_max_width(candidate_width);ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Truncate);
                            for(i,item)in row.candidates.iter().enumerate(){let label=candidate_label(item);ui.selectable_value(&mut row.selected,Some(i),&label).on_hover_text(label);}
                        }).response.on_hover_text(selected);
                        if old!=row.selected{row.confirmed=false;preview=true;}
                        ui.horizontal_wrapped(|ui|{
                            preview|=theme::pill_button(ui,i18n::t(lang,"resource_preview"),row.chosen().is_some(),false).clicked();
                            if theme::pill_button(ui,i18n::t(lang,if row.confirmed{"resource_confirmed"}else{"resource_confirm"}),row.included&&row.chosen().is_some_and(|i|state.kinds.available(i)),false).clicked(){row.confirmed=true;}
                            if state.kinds.footprint&&state.kinds.model3d{egui::ComboBox::from_id_salt("resource_preview_kind").selected_text(i18n::t(lang,if state.show_3d{"model_preview"}else{"footprint_preview"})).show_ui(ui,|ui|{ui.selectable_value(&mut state.show_3d,false,i18n::t(lang,"footprint_preview"));ui.selectable_value(&mut state.show_3d,true,i18n::t(lang,"model_preview"));});}
                        });
                    });
                    let rect=ui.allocate_exact_size(egui::vec2(ui.available_width(),(body_height-174.0).max(80.0)),egui::Sense::hover()).0;
                    paint_resource_preview(ui,rect,state,self.gpu.clone(),lang);
                });
            });
            ui.add_space(8.0);ui.separator();
            if !state.note.is_empty(){ui.add(egui::Label::new(egui::RichText::new(&state.note).small()).truncate()).on_hover_text(&state.note);}
            let included=state.rows.iter().filter(|r|r.included).count();
            let confirmed=state.rows.iter().filter(|r|r.included&&r.confirmed&&r.chosen().is_some_and(|i|state.kinds.available(i))).count();
            ui.horizontal_wrapped(|ui|{
                if running{let(done,total)=*state.progress.lock().unwrap();ui.spinner();ui.label(format!("{done} / {total}"));if ui.add_enabled(!state.cancel.load(Ordering::Relaxed),egui::Button::new(i18n::t(lang,"bom_cancel"))).clicked(){state.cancel.store(true,Ordering::Relaxed);}if state.cancel.load(Ordering::Relaxed){ui.label(i18n::t(lang,"bom_stopping"));}}
                else{ui.label(format!("{confirmed} / {included} {}",i18n::t(lang,"resource_confirmed")));}
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    download=theme::pill_button(ui,&format!("{} ({confirmed})",i18n::t(lang,"resource_download")),!running&&state.kinds.any()&&confirmed>0,true).clicked();
                    close=theme::pill_button(ui,i18n::t(lang,"win_close"),!running,false).clicked();
                });
            });
        });
        if let Some(index)=focus{state.focused=index;state.reset_preview();}
        if confirm_exact {for row in state.rows.iter_mut().filter(|r|r.included&&!r.confirmed){if let Some(index)=row.exact(state.kinds){row.selected=Some(index);row.confirmed=true;}}}
        if search||search_one{state.start_search(search_one);}
        if reset_preview{state.reset_preview();}
        if preview{state.reset_preview();state.preview_pending=true;state.start_preview();}
        if download {
            if let Some(profile)=crate::profile_dir(){
                let jobs:Vec<_>=state.rows.iter().filter(|r|r.included&&r.confirmed).filter_map(|r|Some((r.id.clone(),r.native.clone(),r.chosen()?.clone()))).collect();
                state.cancel.store(false,Ordering::Relaxed);*state.progress.lock().unwrap()=(0,jobs.len());
                let cancel=state.cancel.clone();let progress=state.progress.clone();let kinds=state.kinds;let parent=profile.join("library_resources");
                state.download=Some(Promise::spawn_thread("resource-download",move||{
                    let client=LcedaClient::new();let mut done=0;
                    let results=parallel::map_bounded(&jobs,parallel::NETWORK_WORKERS,&cancel,|(_,native,item)|supplement::download(&client,&native.part,native.format,item,kinds,&parent).map_err(|e|e.to_string()),|_,_|{done+=1;*progress.lock().unwrap()=(done,jobs.len());});
                    let mut saved=Vec::new();let mut failed=Vec::new();for(i,result)in results{match result{Ok(bundle)=>saved.push((jobs[i].0.clone(),bundle)),Err(e)=>failed.push((jobs[i].0.clone(),e))}}
                    DownloadResults{saved,failed}
                }));
            }
        }
        if close||!open||ctx.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Escape)){
            if state.is_busy(){state.cancel.store(true,Ordering::Relaxed);state.close_when_idle=true;}else{self.resource_assist=None;}
        }
    }
}
fn candidate_label(item:&SearchItem)->String{format!("{} · {} · {} · {}{}{}",item.name(),item.lcsc_id().unwrap_or_default(),item.manufacturer_label(),crate::library::package_from_item(item),if item.footprint_uuid().is_some(){" · FP"}else{""},if item.model_uuid.is_some(){" · 3D"}else{""})}
fn paint_resource_preview(ui:&mut egui::Ui,rect:egui::Rect,state:&mut ResourceAssist,gpu:Option<Arc<egui::mutex::Mutex<GpuPreview>>>,lang:Lang){
    paint_well(ui,rect);
    if state.show_3d {
        if let Some(mesh)=state.mesh.clone(){
            let response=ui.interact(rect,ui.id().with("resource_mesh"),egui::Sense::click_and_drag());
            if response.dragged(){let d=response.drag_delta();state.yaw+=d.x*0.01;state.pitch+=d.y*0.01;}
            if response.hovered(){let scroll=ui.input(|i|i.raw_scroll_delta.y);if scroll!=0.0{state.zoom=scroll_zoom(state.zoom,scroll,0.6,5.0);}}
            if response.double_clicked(){state.yaw=0.7;state.pitch=0.55;state.zoom=1.0;}
            if let Some(gpu)=gpu{ui.painter().add(preview3d::paint_callback(rect.shrink(6.0),gpu,mesh,state.yaw,state.pitch,state.zoom,theme::well()));}
            else{let image=rasterize_mesh(&mesh,rect,state.yaw,state.pitch,state.zoom,ui.ctx().pixels_per_point());let texture=ui.ctx().load_texture("resource_mesh",image,TextureOptions::LINEAR);ui.painter_at(rect).image(texture.id(),rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);state.texture=Some(texture);}
            return;
        }
    }else if let Some(fp)=&state.footprint{state.footprint_view.show(ui,rect,fp,false);return;}
    ui.painter_at(rect).text(rect.center(),egui::Align2::CENTER_CENTER,i18n::t(lang,if state.preview.is_some(){"loading"}else{"resource_preview_hint"}),egui::FontId::proportional(12.0),theme::secondary());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_names_are_queries_and_never_supplier_bindings_by_themselves(){
        let native=NativeRecord{format:LibraryFormat::Ad,source:PathBuf::from("A.SchLib"),library:PathBuf::from("copy.SchLib"),part:lceda_core::native_library::NativePart{key:"C2040".into(),name:"C2040".into(),description:String::new(),properties:BTreeMap::new(),footprints:vec!["Keep".into()],pins:2,units:1}};
        assert_eq!(initial_query(&native),"C2040");assert!(native_import::native_lcsc(&native.part).is_none());
        let row=ResourceRow{id:"LOCAL-test".into(),query:"C2040".into(),native,included:true,candidates:vec![],selected:None,confirmed:false,note:None};
        assert!(row.exact(ResourceKinds{footprint:true,model3d:true}).is_none());assert!(!row.confirmed);
    }
}
