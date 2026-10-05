use std::{path::Path,io::{Write,Cursor}};
use serde_json::{Value,json};
use tauri::Manager;
use crate::database::Database;

fn xml(s:&str)->String{s.chars().filter(|c|*c=='\n'||*c=='\t'||*c>=' ').collect::<String>().replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;")}
fn node_text(v:&Value)->String{if let Some(s)=v["text"].as_str(){return s.into();}if v["type"]=="hardBreak"{return "\n".into();}v["content"].as_array().map(|a|a.iter().map(node_text).collect::<String>()).unwrap_or_default()}
fn markdown(v:&Value)->String{
 let content=v["content"].as_array();match v["type"].as_str().unwrap_or(""){
  "text"=>{let mut t=v["text"].as_str().unwrap_or("").to_owned();if let Some(m)=v["marks"].as_array(){for mark in m{t=match mark["type"].as_str(){Some("bold")=>format!("**{t}**"),Some("italic")=>format!("*{t}*"),Some("code")=>format!("`{t}`"),_=>t};}}t},
  "heading"=>format!("{} {}\n\n","#".repeat(v["attrs"]["level"].as_u64().unwrap_or(1).clamp(1,6) as usize),content.map(|a|a.iter().map(markdown).collect::<String>()).unwrap_or_default()),
  "paragraph"=>format!("{}\n\n",content.map(|a|a.iter().map(markdown).collect::<String>()).unwrap_or_default()),
  "listItem"=>format!("- {}\n",node_text(v)),"taskItem"=>format!("- [{}] {}\n",if v["attrs"]["checked"]==true{"x"}else{" "},node_text(v)),"hardBreak"=>"\n".into(),
  _=>content.map(|a|a.iter().map(markdown).collect::<String>()).unwrap_or_default()
 }
}
fn docx(title:&str,body:&str)->Result<Vec<u8>,String>{
 let mut zip=zip::ZipWriter::new(Cursor::new(Vec::new()));let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
 let text=std::iter::once(title).chain(body.lines()).map(|s|format!("<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",xml(s))).collect::<String>();
 for(name,data)in [("[Content_Types].xml",r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_owned()),("_rels/.rels",r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_owned()),("word/document.xml",format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{text}<w:sectPr/></w:body></w:document>"))]{zip.start_file(name,options).map_err(|e|e.to_string())?;zip.write_all(data.as_bytes()).map_err(|e|e.to_string())?;}
 Ok(zip.finish().map_err(|e|e.to_string())?.into_inner())
}
fn safe_name(s:&str)->String{let s=s.chars().map(|c|if c<' '||"<>:\"/\\|?*".contains(c){'_'}else{c}).take(80).collect::<String>();let s=s.trim_matches([' ','.']);if s.is_empty(){"无标题".into()}else{let base=s.split('.').next().unwrap_or("").to_uppercase();if ["CON","PRN","AUX","NUL","COM1","COM2","COM3","COM4","LPT1","LPT2","LPT3"].contains(&base.as_str()){format!("_{s}")}else{s.into()}}}
#[tauri::command]
pub async fn choose_export_folder()->Result<Option<String>,String>{tauri::async_runtime::spawn_blocking(||crate::native_dialog::run(||rfd::FileDialog::new().set_title("选择导出文件夹（文件同名时自动编号，不覆盖）").pick_folder().map(|p|p.to_string_lossy().into_owned()))).await.map_err(|e|e.to_string())?}
#[tauri::command]
pub async fn export_documents(app:tauri::AppHandle,directory:String,format:String,selected_ids:Vec<String>,preferences:Value,include_settings:bool)->Result<Value,String>{
 tauri::async_runtime::spawn_blocking(move||{
  if !["md","txt","json","docx","qjbackup"].contains(&format.as_str()){return Err("导出格式不支持".to_string());}if selected_ids.is_empty()||selected_ids.len()>10000{return Err("请选择 1～10000 条内容".into());}
  let dir=Path::new(&directory);if !dir.is_dir(){return Err("导出目录不可用".into());}
  let db=app.state::<Database>();let c=db.0.lock().map_err(|_|"数据库不可用")?;
  let mut backup=crate::backup::snapshot(&c,preferences)?;crate::backup::select_items(&mut backup,Some(selected_ids.clone()))?;
  let mut records=Vec::new();for id in &selected_ids{records.push(c.query_row("SELECT title,body,body_json FROM items WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).map_err(|e|e.to_string())?);}drop(c);
  // 先准备完整结果，再逐个以 create_new 写入；失败仅回滚本次新建文件，不碰用户原文件。
  let mut payloads=Vec::new();if format=="qjbackup"{payloads.push(("轻笺备份".into(),crate::backup::selected_bytes(backup,include_settings)?));}else{for(title,body,rich)in records{let bytes=match format.as_str(){"json"=>serde_json::to_vec_pretty(&json!({"title":title,"content":body})).map_err(|e|e.to_string())?,"docx"=>docx(&title,&body)?,"md"=>{let doc=rich.as_deref().and_then(|s|serde_json::from_str::<Value>(s).ok());format!("# {}\n\n{}",title,doc.as_ref().map(|v|markdown(&v["document"])).unwrap_or(body.clone())).into_bytes()},_=>format!("{title}\n\n{body}").into_bytes()};payloads.push((safe_name(&title),bytes));}}
  let mut created=Vec::new();let result=(||{for(name,bytes)in payloads{let mut index=0;let(mut f,p)=loop{let suffix=if index==0{String::new()}else{format!(" ({index})")};let p=dir.join(format!("{name}{suffix}.{format}"));match std::fs::OpenOptions::new().write(true).create_new(true).open(&p){Ok(f)=>break(f,p),Err(e)if e.kind()==std::io::ErrorKind::AlreadyExists=>{index+=1;if index>10000{return Err("同名文件数量过多".into());}},Err(e)=>return Err(e.to_string())}};created.push(p);f.write_all(&bytes).map_err(|e|e.to_string())?;f.sync_all().map_err(|e|e.to_string())?;}Ok(json!({"count":selected_ids.len(),"files":created}))})();if result.is_err(){for p in created{let _=std::fs::remove_file(p);}}result
 }).await.map_err(|e|e.to_string())?
}
#[cfg(test)]mod tests{use super::*;#[test]fn formats_are_real(){assert!(docx("中文标题","第一行\n第二行").unwrap().starts_with(b"PK"));assert_eq!(safe_name("CON"),"_CON");assert_eq!(safe_name("a/b"),"a_b");assert_eq!(markdown(&json!({"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"标题"}]})),"## 标题\n\n");}}
