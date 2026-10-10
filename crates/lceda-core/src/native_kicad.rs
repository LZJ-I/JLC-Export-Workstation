use super::*;
use crate::{altium::sch_preview::{SchPreview,SchStroke,SchEllipse,SchText,SchFont,TextAnchor}, native_library::NativePart};

fn child<'a>(node:&'a Node,head:&str)->Option<&'a Node>{node.children.iter().find(|n| n.head()==Some(head))}
fn arg(node:&Node,head:&str)->Option<String>{child(node,head)?.arg().map(str::to_string)}
fn number(node:&Node,i:usize,default:f64)->f64{node.children.get(i).and_then(Node::text).and_then(|s|s.parse::<f64>().ok()).filter(|n|n.is_finite()).unwrap_or(default)}
fn point(node:&Node)->(f64,f64){(number(node,1,0.0),number(node,2,0.0))}
fn symbol<'a>(root:&'a Node,key:&str)->Result<&'a Node>{root.children.iter().find(|n|n.head()==Some("symbol")&&n.arg()==Some(key)).ok_or_else(||invalid(format!("没有找到符号 {key}")))}
fn chain<'a>(root:&'a Node,key:&str)->Result<Vec<&'a Node>>{
    let mut nodes=Vec::new();let mut seen=HashSet::new();let mut current=key.to_string();
    loop {
        if !seen.insert(current.clone())||nodes.len()>128{return Err(invalid("符号继承有循环或嵌套过深"));}
        let node=symbol(root,&current)?;nodes.push(node);
        if let Some(parent)=arg(node,"extends"){current=parent;}else{break;}
    }
    nodes.reverse();Ok(nodes)
}
fn unit_id(node:&Node)->(usize,usize){
    let mut parts=node.arg().unwrap_or("").rsplit('_');
    let style=parts.next().and_then(|s|s.parse().ok()).unwrap_or(1);
    let unit=parts.next().and_then(|s|s.parse().ok()).unwrap_or(0);(unit,style)
}
fn properties(nodes:&[&Node])->BTreeMap<String,String>{
    let mut properties=BTreeMap::new();
    for node in nodes {for p in node.children.iter().filter(|n|n.head()==Some("property")){
        if let Some(key)=p.arg(){properties.insert(key.to_string(),p.children.get(2).and_then(Node::text).unwrap_or("").to_string());}
    }}properties
}
pub(crate) fn inspect_library(path:&Path)->Result<Vec<NativePart>>{
    let bytes=read_optional(path)?.ok_or_else(||invalid("库文件不存在"))?;
    let text=std::str::from_utf8(&bytes).map_err(invalid)?;let root=library(text)?;let mut result=Vec::new();
    for node in root.children.iter().filter(|n|n.head()==Some("symbol")){
        let name=node.arg().unwrap();let nodes=chain(&root,name)?;let properties=properties(&nodes);
        let units=nodes.iter().flat_map(|n|n.children.iter().filter(|c|c.head()==Some("symbol"))).map(|n|unit_id(n).0).max().unwrap_or(1).clamp(1,1024);
        let pins=nodes.iter().flat_map(|n|n.children.iter().filter(|c|c.head()==Some("symbol"))).filter(|n|unit_id(n).1<=1)
            .flat_map(|n|n.children.iter().filter(|c|c.head()==Some("pin"))).count();
        let description=properties.get("Description").cloned().or_else(||nodes.iter().rev().find_map(|n|arg(n,"description"))).unwrap_or_default();
        let footprints=properties.get("Footprint").filter(|v|!v.is_empty()).cloned().into_iter().collect();
        result.push(NativePart{key:name.to_string(),name:name.to_string(),description,properties,footprints,pins,units});
    }Ok(result)
}
fn hidden(node:&Node)->bool{node.children.iter().any(|n|n.text()==Some("hide")||n.head()==Some("hide")&&n.arg()!=Some("no"))}
fn label(node:&Node,text:String,role:&str,position:(f64,f64),rotation:f64,anchor:TextAnchor)->Option<SchText>{
    let effects=child(node,"effects");if effects.is_some_and(hidden)||hidden(node){return None;}
    let size=effects.and_then(|e|child(e,"font")).and_then(|f|child(f,"size")).map(|s|number(s,1,1.27)).unwrap_or(1.27);
    Some(SchText{role:role.into(),text,position,rotation,anchor,font:SchFont{name:"Times New Roman".into(),points:size/0.254,color:None},color:0})
}
pub(crate) fn preview_component(path:&Path,key:&str,unit:usize)->Result<SchPreview>{
    let bytes=read_optional(path)?.ok_or_else(||invalid("库文件不存在"))?;let text=std::str::from_utf8(&bytes).map_err(invalid)?;
    let root=library(text)?;let nodes=chain(&root,key)?;
    let mut scene=SchPreview{background:16317695,strokes:Vec::new(),ellipses:Vec::new(),texts:Vec::new()};
    let mut props=BTreeMap::new();for n in &nodes {for p in n.children.iter().filter(|c|c.head()==Some("property")){if let Some(name)=p.arg(){props.insert(name.to_string(),p);}}}
    for (name,p) in props {
        let at=child(p,"at");let pos=at.map(point).unwrap_or((0.0,0.0));let angle=at.map(|a|number(a,3,0.0)).unwrap_or(0.0);
        if let Some(text)=label(p,p.children.get(2).and_then(Node::text).unwrap_or("").into(),&name,pos,angle,TextAnchor::CenterBottom){scene.texts.push(text);}
    }
    let pin_names_hidden=nodes.iter().any(|n|child(n,"pin_names").is_some_and(hidden));
    let pin_numbers_hidden=nodes.iter().any(|n|child(n,"pin_numbers").is_some_and(hidden));
    for n in &nodes {for body in n.children.iter().filter(|c|c.head()==Some("symbol")){
        let (part,style)=unit_id(body);if (part!=0&&part!=unit)||style>1{continue;}
        for shape in &body.children {
            let width=child(shape,"stroke").and_then(|s|child(s,"width")).map(|w|number(w,1,0.254)).unwrap_or(0.254).max(0.0254);
            match shape.head().unwrap_or("") {
                "rectangle"=>{if let(Some(a),Some(b))=(child(shape,"start"),child(shape,"end")){let a=point(a);let b=point(b);scene.strokes.push(SchStroke{points:vec![a,(b.0,a.1),b,(a.0,b.1),a],color:128,width});}}
                "polyline"|"bezier"=>{if let Some(pts)=child(shape,"pts"){let points=pts.children.iter().filter(|n|n.head()==Some("xy")).map(point).collect();scene.strokes.push(SchStroke{points,color:128,width});}}
                "circle"=>{if let(Some(c),Some(r))=(child(shape,"center"),child(shape,"radius")){let r=number(r,1,0.0);scene.ellipses.push(SchEllipse{center:point(c),radii:(r,r),color:128,width});}}
                "arc"=>{if let(Some(a),Some(m),Some(b))=(child(shape,"start"),child(shape,"mid"),child(shape,"end")){scene.strokes.push(SchStroke{points:arc_points(point(a),point(m),point(b)),color:128,width});}}
                "text"=>{if let Some(at)=child(shape,"at"){if let Some(label)=label(shape,shape.arg().unwrap_or("").into(),"Text",point(at),number(at,3,0.0),TextAnchor::CenterBottom){scene.texts.push(label);}}}
                "pin"=>{
                    if hidden(shape){continue;}let Some(at)=child(shape,"at")else{continue;};let pos=point(at);let angle=number(at,3,0.0).to_radians();
                    let len=child(shape,"length").map(|n|number(n,1,2.54)).unwrap_or(2.54);let d=(angle.cos(),angle.sin());let end=(pos.0+d.0*len,pos.1+d.1*len);
                    scene.strokes.push(SchStroke{points:vec![pos,end],color:128,width:0.1524});
                    if let Some(n)=child(shape,"name").filter(|_|!pin_names_hidden){let name=n.arg().unwrap_or("");if name!="~"{if let Some(t)=label(n,name.into(),"PinName",(end.0+d.0*0.508,end.1+d.1*0.508),if d.0.abs()<0.1{90.0}else{0.0},if d.0<0.0||d.1<0.0{TextAnchor::RightCenter}else{TextAnchor::LeftCenter}){scene.texts.push(t);}}}
                    if let Some(n)=child(shape,"number").filter(|_|!pin_numbers_hidden){if let Some(t)=label(n,n.arg().unwrap_or("").into(),"PinDesignator",((pos.0+end.0)/2.0-if d.0.abs()<0.1{0.508}else{0.0},(pos.1+end.1)/2.0+if d.0.abs()>0.1{0.508}else{0.0}),if d.0.abs()<0.1{90.0}else{0.0},TextAnchor::CenterBottom){scene.texts.push(t);}}
                }
                _=>{}
            }
        }
    }}Ok(scene)
}
fn arc_points(a:(f64,f64),m:(f64,f64),b:(f64,f64))->Vec<(f64,f64)>{
    let d=2.0*(a.0*(m.1-b.1)+m.0*(b.1-a.1)+b.0*(a.1-m.1));if d.abs()<1e-9{return vec![a,m,b];}
    let q=|p:(f64,f64)|p.0*p.0+p.1*p.1;
    let c=((q(a)*(m.1-b.1)+q(m)*(b.1-a.1)+q(b)*(a.1-m.1))/d,(q(a)*(b.0-m.0)+q(m)*(a.0-b.0)+q(b)*(m.0-a.0))/d);
    let start=(a.1-c.1).atan2(a.0-c.0);let mid=(m.1-c.1).atan2(m.0-c.0);let end=(b.1-c.1).atan2(b.0-c.0);let tau=std::f64::consts::TAU;
    let mut sweep=(end-start).rem_euclid(tau);if (mid-start).rem_euclid(tau)>sweep{sweep-=tau;}let r=(a.0-c.0).hypot(a.1-c.1);
    (0..=48).map(|i|{let t=start+sweep*i as f64/48.0;(c.0+r*t.cos(),c.1+r*t.sin())}).collect()
}
pub(crate) fn export_subset(source:&Path,keys:&[String],destination:&Path)->Result<()>{
    let bytes=read_optional(source)?.ok_or_else(||invalid("库文件不存在"))?;let text=std::str::from_utf8(&bytes).map_err(invalid)?;let root=library(text)?;
    let mut wanted=HashSet::new();for key in keys{for n in chain(&root,key)?{wanted.insert(n.arg().unwrap().to_string());}}
    let mut output=text.to_string();let mut removed:Vec<_>=root.children.iter().filter(|n|n.head()==Some("symbol")&&!wanted.contains(n.arg().unwrap())).map(|n|n.span.clone()).collect();
    removed.sort_by_key(|s|std::cmp::Reverse(s.start));for range in removed{output.replace_range(range,"");}library(&output)?;
    let mut file=OpenOptions::new().write(true).create_new(true).open(destination)?;file.write_all(output.as_bytes())?;file.sync_all()?;Ok(())
}
