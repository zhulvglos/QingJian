//! DOCX 只在内存读取必要的 XML；不解压到磁盘，不访问关系文件中的外部地址。
use crate::document_formats::{paragraph,text_of,MAX_NODES};
use quick_xml::{events::{BytesStart,Event},Reader};
use serde_json::{json,Value};
use std::{collections::{BTreeSet,HashMap},io::{Cursor,Read},sync::atomic::{AtomicBool,Ordering}};
const MAX_EXPANDED:u64=32*1024*1024;
const MAX_XML:u64=16*1024*1024;
fn local(name:&[u8])->String{String::from_utf8_lossy(name.rsplit(|b|*b==b':').next().unwrap_or(name)).into()}
fn val(e:&BytesStart<'_>,key:&str)->Option<String>{e.attributes().filter_map(Result::ok).find(|a|local(a.key.as_ref())==key).and_then(|a|a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|s|s.into_owned()))}
fn cancelled(c:&AtomicBool)->Result<(),String>{if c.load(Ordering::SeqCst){Err("已取消解析".into())}else{Ok(())}}
fn xml(archive:&mut zip::ZipArchive<Cursor<&[u8]>>,name:&str,c:&AtomicBool)->Result<Option<String>,String>{
 cancelled(c)?;let f=match archive.by_name(name){Ok(f)=>f,Err(zip::result::ZipError::FileNotFound)=>return Ok(None),Err(_)=>return Err("DOCX 已加密或压缩内容损坏，不支持导入".into())};
 if f.size()>MAX_XML{return Err("DOCX XML 超过 16 MiB 上限".into());}
 let mut data=vec![];f.take(MAX_XML+1).read_to_end(&mut data).map_err(|_|"DOCX 解压失败，文件可能损坏")?;cancelled(c)?;
 if data.len() as u64>MAX_XML{return Err("DOCX XML 超过解析上限".into());}
 String::from_utf8(data).map(Some).map_err(|_|"DOCX XML 不是受支持的 UTF-8 编码".into())
}
#[derive(Default)]struct Para{nodes:Vec<Value>,style:Option<String>,number:Option<String>,level:usize,marks:Vec<Value>}
#[derive(Default)]struct Table{rows:Vec<String>,row:Vec<String>,cell:Vec<String>}
fn styles(source:Option<String>)->Result<HashMap<String,u8>,String>{
 let mut result=HashMap::new();for n in 1..=6{result.insert(format!("Heading{n}"),n);result.insert(format!("heading{n}"),n);}result.insert("Title".into(),1);
 if let Some(source)=source{let mut r=Reader::from_str(&source);let mut id=None;
  let mut count=0usize;loop{count+=1;if count>MAX_NODES*4{return Err("DOCX 样式结构超过解析上限".into());}match r.read_event().map_err(|_|"DOCX 样式 XML 损坏")?{
  Event::Start(e) if local(e.name().as_ref())=="style"=>id=val(&e,"styleId"),
  Event::Empty(e) if local(e.name().as_ref())=="outlineLvl"=>if let (Some(id),Some(n))=(&id,val(&e,"val").and_then(|s|s.parse::<u8>().ok())){if n<6{result.insert(id.clone(),n+1);}},
  Event::End(e) if local(e.name().as_ref())=="style"=>id=None,
  Event::DocType(_)=>return Err("DOCX 含不支持的 DTD".into()),Event::Eof=>break,_=>{}
 }}}
 Ok(result)
}
fn numbering(source:Option<String>)->Result<HashMap<String,bool>,String>{
 let mut defs=HashMap::new();let mut nums=HashMap::new();let mut abstract_id=None;let mut num_id=None;
 if let Some(source)=source{let mut r=Reader::from_str(&source);let mut count=0usize;loop{count+=1;if count>MAX_NODES*4{return Err("DOCX 列表结构超过解析上限".into());}match r.read_event().map_err(|_|"DOCX 列表 XML 损坏")?{
  Event::Start(e)=>match local(e.name().as_ref()).as_str(){"abstractNum"=>abstract_id=val(&e,"abstractNumId"),"num"=>num_id=val(&e,"numId"),_=>{}},
  Event::Empty(e)=>match local(e.name().as_ref()).as_str(){"numFmt"=>if let Some(id)=&abstract_id{defs.entry(id.clone()).or_insert(val(&e,"val").as_deref()!=Some("bullet"));},"abstractNumId"=>if let (Some(n),Some(a))=(&num_id,val(&e,"val")){nums.insert(n.clone(),a);},_=>{}},
  Event::End(e)=>match local(e.name().as_ref()).as_str(){"abstractNum"=>abstract_id=None,"num"=>num_id=None,_=>{}},
  Event::DocType(_)=>return Err("DOCX 含不支持的 DTD".into()),Event::Eof=>break,_=>{}
 }}}
 Ok(nums.into_iter().map(|(n,a)|(n,*defs.get(&a).unwrap_or(&false))).collect())
}
fn node(p:Para,styles:&HashMap<String,u8>)->Value{
 if let Some(level)=p.style.and_then(|s|styles.get(&s).copied()){json!({"type":"heading","attrs":{"level":level},"content":p.nodes})}else{json!({"type":"paragraph","content":p.nodes})}
}
fn text(p:&mut Para,s:&str){if !s.is_empty(){let mut n=json!({"type":"text","text":s});if !p.marks.is_empty(){n["marks"]=json!(p.marks);}p.nodes.push(n);}}
pub fn parse(data:&[u8],c:&AtomicBool)->Result<(Value,Vec<String>),String>{
 if data.starts_with(&[0xd0,0xcf,0x11,0xe0]){return Err("不支持加密 Word 或旧版 .doc，请另存为未加密 .docx".into());}
 let mut archive=zip::ZipArchive::new(Cursor::new(data)).map_err(|_|"DOCX 文件损坏或已加密")?;
 if archive.len()>2000{return Err("DOCX 包内文件超过 2000 个上限".into());}
 let mut expanded=0u64;let mut warnings=BTreeSet::new();
 for i in 0..archive.len(){cancelled(c)?;let f=archive.by_index(i).map_err(|_|"DOCX 已加密或包内文件损坏")?;expanded=expanded.checked_add(f.size()).ok_or("DOCX 解压体积超限")?;if expanded>MAX_EXPANDED{return Err("DOCX 解压体积超过 32 MiB 上限".into());}
  let name=f.name().to_lowercase();if name.contains("vbaproject"){return Err("不支持包含宏的 Word 文件".into());}
  if name.starts_with("word/media/"){warnings.insert("图片不会导入，正文中保留图片占位说明。".into());}
  if name.starts_with("word/header")||name.starts_with("word/footer"){warnings.insert("页眉和页脚不在正文范围内，未导入。".into());}
  if name.starts_with("word/footnotes")||name.starts_with("word/endnotes")||name.starts_with("word/comments"){warnings.insert("脚注、尾注和批注未导入。".into());}
 }
 let types=xml(&mut archive,"[Content_Types].xml",c)?.ok_or("不是有效的 Word DOCX 包")?;
 if types.to_lowercase().contains("macroenabled"){return Err("不支持包含宏的 Word 文件".into());}
 let document=xml(&mut archive,"word/document.xml",c)?.ok_or("DOCX 缺少正文文件")?;
 let headings=styles(xml(&mut archive,"word/styles.xml",c)?)?;let lists=numbering(xml(&mut archive,"word/numbering.xml",c)?)?;
 let mut r=Reader::from_str(&document);let mut paras:Vec<Para>=vec![];let mut tables:Vec<Table>=vec![];let mut blocks=vec![];let mut in_body=false;let mut in_text=false;let mut depth=0usize;let mut events=0usize;
 loop{
  events+=1;if events>MAX_NODES*10||depth>64{return Err("DOCX 结构超过解析资源上限".into());}if events%128==0{cancelled(c)?;}
  match r.read_event().map_err(|_|"DOCX 正文 XML 损坏")?{
   Event::Start(e)=>{depth+=1;let name=local(e.name().as_ref());if name=="body"{in_body=true;}
    if in_body{match name.as_str(){"p"=>paras.push(Para::default()),"tbl"=>{tables.push(Table::default());warnings.insert("表格已转换为逐行文字，单元格用 | 分隔。".into());},"r"=>{if let Some(p)=paras.last_mut(){p.marks.clear();}},"t"|"delText"=>{in_text=true;if name=="delText"{warnings.insert("修订中的删除文字按可读内容保留。".into());}},"drawing"|"pict"=>{if let Some(p)=paras.last_mut(){text(p,"[图片或绘图未导入]");}warnings.insert("图片和绘图不导入，仅保留占位说明。".into());},_=>{}}
    }
   }
   Event::Empty(e) if in_body=>{let name=local(e.name().as_ref());if let Some(p)=paras.last_mut(){match name.as_str(){
    "pStyle"=>p.style=val(&e,"val"),"numId"=>p.number=val(&e,"val").filter(|s|s!="0"),"ilvl"=>p.level=val(&e,"val").and_then(|s|s.parse().ok()).unwrap_or(0),
    "b"|"i"|"u"|"strike"=>{if !matches!(val(&e,"val").as_deref(),Some("0"|"false"|"none")){p.marks.push(json!({"type":match name.as_str(){"b"=>"bold","i"=>"italic","u"=>"underline",_=>"strike"}}));}},
    "tab"=>text(p,"\t"),"br"|"cr"=>p.nodes.push(json!({"type":"hardBreak"})),"drawing"|"pict"=>{text(p,"[图片或绘图未导入]");warnings.insert("图片和绘图不导入，仅保留占位说明。".into());},_=>{}
   }}}
   Event::Text(e) if in_text=>{let s=e.decode().map_err(|_|"DOCX 正文编码错误")?;if let Some(p)=paras.last_mut(){text(p,&s);}},
   Event::GeneralRef(e) if in_text=>{let source=format!("&{};",String::from_utf8_lossy(e.as_ref()));let s=quick_xml::escape::unescape(&source).map_err(|_|"DOCX 含无法解析的实体")?;if let Some(p)=paras.last_mut(){text(p,&s);}},
   Event::End(e)=>{depth=depth.saturating_sub(1);let name=local(e.name().as_ref());if in_body{match name.as_str(){
    "t"|"delText"=>in_text=false,"r"=>if let Some(p)=paras.last_mut(){p.marks.clear();},
    "p"=>{let p=paras.pop().ok_or("DOCX 段落结构损坏")?;let number=p.number.clone();let level=p.level;let n=node(p,&headings);
     if let Some(parent)=paras.last_mut(){text(parent,&text_of(&n));warnings.insert("复杂文本框已转换为正文文字。".into());}
     else if let Some(table)=tables.last_mut(){table.cell.push(text_of(&n));}
     else if let Some(id)=number{let kind=if *lists.get(&id).unwrap_or(&false){"orderedList"}else{"bulletList"};let mut para=n;para["type"]=json!("paragraph");para.as_object_mut().unwrap().remove("attrs");
      if level>0{warnings.insert("嵌套列表层级已展开为基本列表，原文顺序保留。".into());}warnings.insert("Word 列表编号与缩进统一为编辑器基本列表样式。".into());
      let item=json!({"type":"listItem","content":[para]});if blocks.last().is_some_and(|v:&Value|v["type"]==kind){blocks.last_mut().unwrap()["content"].as_array_mut().unwrap().push(item);}else{blocks.push(json!({"type":kind,"content":[item]}));}
     }else{blocks.push(n);}
    },
    "tc"=>if let Some(t)=tables.last_mut(){t.row.push(std::mem::take(&mut t.cell).join(" / "));},
    "tr"=>if let Some(t)=tables.last_mut(){t.rows.push(std::mem::take(&mut t.row).join(" | "));},
    "tbl"=>{let t=tables.pop().ok_or("DOCX 表格结构损坏")?;if let Some(parent)=tables.last_mut(){parent.cell.push(t.rows.join(" / "));}else{blocks.extend(t.rows.iter().map(|s|paragraph(s)));}},
    "body"=>in_body=false,_=>{}
   }}},
   Event::DocType(_)=>return Err("DOCX 不支持 DTD 或外部实体".into()),Event::Eof=>break,_=>{}
  }
 }
 if depth!=0||!paras.is_empty()||!tables.is_empty(){return Err("DOCX XML 未完整结束".into());}
 cancelled(c)?;warnings.insert("复杂排版、分页、字体及精确编号不保留；仅导入正文基本格式。".into());
 Ok((json!({"type":"doc","content":blocks}),warnings.into_iter().collect()))
}
