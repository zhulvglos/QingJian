//! 导入预览只驻留内存。确认才备份并写入，取消或迟到的解析结果不能写库。
use crate::{database::{Database,insert_new_item},document_formats::{self as formats,Entry,FilePreview}};
use serde::Deserialize;
use serde_json::{json,Value};
use std::{collections::HashSet,io::Read,path::Path,sync::{Arc,Mutex,OnceLock,atomic::{AtomicBool,Ordering}}};
use tauri::{Emitter,Manager};
pub const MAX_FILE:u64=10*1024*1024;
pub const MAX_BATCH:u64=30*1024*1024;
const MAX_FILES:usize=20;
struct Control{id:String,cancel:Arc<AtomicBool>,phase:&'static str,files:Vec<FilePreview>,cancelled_ids:HashSet<String>}
fn control()->&'static Mutex<Control>{static STATE:OnceLock<Mutex<Control>>=OnceLock::new();STATE.get_or_init(||Mutex::new(Control{id:String::new(),cancel:Arc::new(AtomicBool::new(false)),phase:"idle",files:vec![],cancelled_ids:HashSet::new()}))}
fn check(c:&AtomicBool)->Result<(),String>{if c.load(Ordering::SeqCst){Err("已取消导入解析，未写入数据".into())}else{Ok(())}}
pub fn saving()->bool{control().lock().map(|s|s.phase=="saving").unwrap_or(true)}
pub struct ExternalSaveGuard;
impl Drop for ExternalSaveGuard{fn drop(&mut self){if let Ok(mut s)=control().lock(){if s.id=="news-collection"{s.phase="done";s.id.clear();}}}}
pub fn begin_external_save()->Result<ExternalSaveGuard,String>{let mut s=control().lock().map_err(|_|"保存状态不可用")?;if matches!(s.phase,"saving"|"parsing"|"ready"){return Err("请先完成或取消当前导入".into());}s.id="news-collection".into();s.phase="saving";Ok(ExternalSaveGuard)}
pub fn cancel(){if let Ok(mut s)=control().lock(){if s.phase!="saving"{s.cancel.store(true,Ordering::SeqCst);s.files.clear();s.phase="cancelled";}}}
fn error_file(path:&str)->FilePreview{let p=Path::new(path);FilePreview{id:uuid::Uuid::new_v4().to_string(),file_name:p.file_name().unwrap_or_default().to_string_lossy().into(),format:p.extension().unwrap_or_default().to_string_lossy().to_lowercase(),entries:vec![],warnings:vec![],error:None,fields:vec![],can_map:false,can_as_text:false,raw_json:None}}
fn bounded_read(path:&Path,c:&AtomicBool)->Result<Vec<u8>,String>{
 check(c)?;if path.to_string_lossy().starts_with("\\\\"){return Err("只支持本地文件，不从网络共享读取文档".into());}
 let mut f=std::fs::File::open(path).map_err(|_|"文件无法读取，请检查权限或是否仍存在")?;let meta=f.metadata().map_err(|_|"无法读取文件属性")?;
 if !meta.is_file(){return Err("请选择普通文件".into());}if meta.len()>MAX_FILE{return Err("单文件超过 10 MiB 上限".into());}
 let mut out=Vec::with_capacity(meta.len() as usize);let mut buf=[0u8;64*1024];
 loop{check(c)?;let n=f.read(&mut buf).map_err(|_|"读取文件失败")?;if n==0{break;}if out.len()+n>MAX_FILE as usize{return Err("文件读取期间超过 10 MiB 上限".into());}out.extend_from_slice(&buf[..n]);}Ok(out)
}
fn decoded(data:&[u8])->Result<String,String>{
 let s=std::str::from_utf8(data).map_err(|_|"编码错误：只支持 UTF-8 或 UTF-8 BOM，请转换编码后重试")?.trim_start_matches('\u{feff}');
 if s.chars().any(|c|c.is_control()&&!matches!(c,'\n'|'\r'|'\t')){return Err("文件包含非文本控制字符，不能作为普通文档导入".into());}
 Ok(s.replace("\r\n","\n").replace('\r',"\n"))
}
fn json_structure(raw:&Value)->Result<(),String>{
 let mut pending=vec![(raw,0usize)];let mut count=0usize;
 while let Some((v,depth))=pending.pop(){count+=1;if depth>64||count>100_000{return Err("JSON 深度或结构数量超过解析上限".into());}
  match v{Value::Object(o)=>{if o.len()>100||o.keys().any(|s|s.chars().count()>128){return Err("JSON 单个对象最多 100 个字段，字段名最多 128 字".into());}pending.extend(o.values().map(|v|(v,depth+1)));},Value::Array(a)=>pending.extend(a.iter().map(|v|(v,depth+1))),_=>{}}
 }Ok(())
}
fn parse_file(path:&str,c:&AtomicBool)->FilePreview{
 let mut file=error_file(path);let p=Path::new(path);let stem=p.file_stem().unwrap_or_default().to_string_lossy().to_string();
 let result=(||->Result<(),String>{
  if !["txt","md","markdown","docx","json"].contains(&file.format.as_str()){return Err(if file.format=="qjbackup"{".qjbackup 请使用设置 → 文件中的备份恢复入口".into()}else if file.format=="doc"||file.format=="docm"{"不支持旧版 .doc 或宏文件，请另存为未加密 .docx".into()}else{"仅支持 TXT、Markdown、DOCX 和 JSON".into()});}
  let data=bounded_read(p,c)?;check(c)?;
  if file.format=="json"{
   let source=decoded(&data)?;let raw:Value=serde_json::from_str(&source).map_err(|e|format!("JSON 格式错误（第 {} 行，第 {} 列）",e.line(),e.column()))?;
   json_structure(&raw)?;file.can_as_text=true;file.raw_json=Some(raw.clone());
   match formats::json_entries(&raw,&file.id,&stem,None,None,false){Ok((entries,warnings,fields,_))=>{file.entries=entries;file.warnings=warnings;file.fields=fields;file.can_map=!file.fields.is_empty();},Err(e)=>file.error=Some(e)}
   // 即使标准形状解析失败，也只列出实际存在的顶层字段供用户查看。
   if file.fields.is_empty(){if let Some(o)=raw.as_object(){file.fields=o.keys().cloned().collect();}}
  }else{
   let (document,warnings)=if file.format=="docx"{crate::document_docx::parse(&data,c)?}else{
    let source=decoded(&data)?;if source.len()>formats::MAX_BODY{return Err("正文超过 500 KiB 上限，请拆分后导入".into());}
    if file.format=="txt"{if source.lines().count()>formats::MAX_NODES{return Err("TXT 行数超过结构上限".into());}(formats::plain_document(&source),vec![])}else{formats::markdown(&source)?}
   };
   let body=formats::text_of(&document);file.entries.push(formats::entry(format!("{}:0",file.id),stem,body,document));file.warnings=warnings;
  }check(c)?;Ok(())
 })();if let Err(e)=result{file.error=Some(e);}file
}
fn limits(files:&[FilePreview])->Result<(),String>{
 let entries=files.iter().map(|f|f.entries.len()).sum::<usize>();if entries>formats::MAX_ENTRIES{return Err("合计超过 200 条导入上限，请减少文件或拆分 JSON".into());}
 let bytes=files.iter().flat_map(|f|&f.entries).map(|e|e.body.len()+e.body_json.as_ref().map_or(0,String::len)+e.title.len()).sum::<usize>();
 if bytes>32*1024*1024{return Err("解析后的正文总量超过 32 MiB 上限，请分批导入".into());}Ok(())
}
fn response(s:&Control)->Value{json!({"requestId":s.id,"files":s.files})}
#[tauri::command]
pub async fn choose_document_files(app:tauri::AppHandle)->Result<Option<Vec<String>>,String>{
 let directory=app.state::<crate::data_root::DataRoot>().0.clone();
 tauri::async_runtime::spawn_blocking(move||crate::native_dialog::run(move||rfd::FileDialog::new().set_directory(directory).set_title("选择要导入的文件").add_filter("文档和轻笺备份",&["txt","md","markdown","docx","json","qjbackup"]).pick_files().map(|p|p.into_iter().map(|p|p.to_string_lossy().to_string()).collect()))).await.map_err(|_|"文件选择器不可用".to_string())?
}
#[tauri::command]
pub async fn preview_documents(request_id:String,paths:Vec<String>)->Result<Value,String>{
 uuid::Uuid::parse_str(&request_id).map_err(|_|"预览会话无效")?;
 if paths.is_empty()||paths.len()>MAX_FILES{return Err("请选择 1–20 个文件".into());}
 let cancel={let mut s=control().lock().map_err(|_|"导入状态不可用")?;if s.phase=="saving"{return Err("正在保存导入内容".into());}if s.cancelled_ids.contains(&request_id)||(s.id==request_id&&s.cancel.load(Ordering::SeqCst)){return Err("该导入会话已取消".into());}s.cancel.store(true,Ordering::SeqCst);s.id=request_id.clone();s.cancel=Arc::new(AtomicBool::new(false));s.phase="parsing";s.files.clear();s.cancel.clone()};
 let result=tauri::async_runtime::spawn_blocking(move||{
  let mut total=0u64;let mut unique=HashSet::new();let mut files=vec![];
  for path in paths{check(&cancel)?;if !unique.insert(path.clone()){continue;}
   if let Ok(meta)=std::fs::metadata(&path){if meta.len()<=MAX_FILE{total=total.checked_add(meta.len()).ok_or("文件体积超限")?;}}if total>MAX_BATCH{return Err("文件总量超过 30 MiB 上限".into());}
   files.push(parse_file(&path,&cancel));limits(&files)?;
  }check(&cancel)?;Ok::<_,String>(files)
 }).await.map_err(|_|"文档解析任务异常")?;
 let mut s=control().lock().map_err(|_|"导入状态不可用")?;if s.id!=request_id{return Err("预览已被新的会话替代".into());}check(&s.cancel)?;
 match result{Ok(files)=>{s.files=files;s.phase="ready";Ok(response(&s))},Err(e)=>{s.phase="failed";Err(e)}}
}
#[tauri::command]
pub fn cancel_document_import(request_id:String)->Result<(),String>{
 uuid::Uuid::parse_str(&request_id).map_err(|_|"取消会话无效")?;
 let mut s=control().lock().map_err(|_|"导入状态不可用")?;
 if s.id==request_id{if s.phase=="saving"{return Err("正在事务保存，请等待保存完成".into());}s.cancel.store(true,Ordering::SeqCst);s.files.clear();s.phase="cancelled";}
 // 旧弹窗的迟到取消不能取消新会话；也记住先取消后开始的会话。
 if s.cancelled_ids.len()>=256{s.cancelled_ids.clear();}s.cancelled_ids.insert(request_id);Ok(())
}
#[tauri::command]
pub async fn remap_document_json(request_id:String,file_id:String,title_field:Option<String>,body_field:Option<String>,as_text:bool)->Result<Value,String>{
 let (mut files,cancel)={let s=control().lock().map_err(|_|"导入状态不可用")?;if s.id!=request_id||s.phase!="ready"{return Err("请重新选择文件并预览".into());}check(&s.cancel)?;(s.files.clone(),s.cancel.clone())};
 let updated=tauri::async_runtime::spawn_blocking(move||{
  check(&cancel)?;let f=files.iter_mut().find(|f|f.id==file_id).ok_or("文件预览已失效")?;let raw=f.raw_json.as_ref().ok_or("仅 JSON 支持字段映射")?;
  for field in [&title_field,&body_field].into_iter().flatten(){if !f.fields.contains(field){return Err("只能选择实际存在的字段".into());}}
  let stem=Path::new(&f.file_name).file_stem().unwrap_or_default().to_string_lossy();
  match formats::json_entries(raw,&f.id,&stem,title_field.as_deref(),body_field.as_deref(),as_text){Ok((entries,warnings,_,_))=>{f.entries=entries;f.warnings=warnings;f.error=None;},Err(e)=>{f.entries.clear();f.error=Some(e);}}
  limits(&files)?;check(&cancel)?;Ok::<_,String>(files)
 }).await.map_err(|_|"JSON 映射任务异常")??;
 let mut s=control().lock().map_err(|_|"导入状态不可用")?;if s.id!=request_id||s.phase!="ready"{return Err("该预览已取消或失效".into());}check(&s.cancel)?;s.files=updated;Ok(response(&s))
}
#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Selection{pub entry_id:String,pub title:String,pub kind:String}
pub(crate) fn insert_batch(conn:&mut rusqlite::Connection,selected:&[(Entry,Selection)])->Result<Vec<Value>,String>{
 let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|"无法开始导入事务，已有内容未修改")?;let mut imported=vec![];
 for (e,s) in selected{let id=insert_new_item(&tx,&s.kind,&s.title,&e.body,e.body_json.as_deref()).map_err(|_|"保存失败，整批已回滚，已有内容未修改")?;imported.push(json!({"id":id,"kind":s.kind}));}
 tx.commit().map_err(|_|"提交失败，整批已回滚，已有内容未修改")?;Ok(imported)
}
#[tauri::command]
pub async fn commit_documents(request_id:String,selections:Vec<Selection>,preferences:Value,app:tauri::AppHandle)->Result<Value,String>{
 let selected={let mut s=control().lock().map_err(|_|"导入状态不可用")?;
  if s.id!=request_id||s.phase!="ready"{return Err("请重新预览后确认导入".into());}check(&s.cancel)?;
  if selections.is_empty()||selections.len()>formats::MAX_ENTRIES{return Err("请勾选 1–200 条有效内容".into());}let mut ids=HashSet::new();let mut selected=vec![];
  for selection in selections{if !ids.insert(selection.entry_id.clone()){return Err("同一预览条目不能重复勾选".into());}if !["sticky","note"].contains(&selection.kind.as_str())||selection.title.chars().count()>500{return Err("目标栏目或标题无效".into());}
   let e=s.files.iter().filter(|f|f.error.is_none()).flat_map(|f|&f.entries).find(|e|e.id==selection.entry_id&&e.error.is_none()).ok_or("所选内容无效，请检查预览")?.clone();selected.push((e,selection));
  }s.phase="saving";selected
 };
 let worker_app=app.clone();let result=tauri::async_runtime::spawn_blocking(move||{
  let db=worker_app.state::<Database>();let mut conn=db.0.lock().map_err(|_|"数据库不可用，未导入")?;
  // 备份失败即停止。和原备份恢复共用可校验的 .qjbackup，不写出任何 API Key。
  let backup=crate::backup::before_document_import(&conn,preferences,&worker_app.state::<crate::data_root::DataRoot>().0)?;
  let imported=insert_batch(&mut conn,&selected)?;drop(conn);
  let _=worker_app.emit_to("quick","quick-items-changed",true);
  Ok::<_,String>(json!({"items":imported,"sticky":imported.iter().filter(|r|r["kind"]=="sticky").count(),"note":imported.iter().filter(|r|r["kind"]=="note").count(),"backupPath":backup}))
 }).await.map_err(|_|"文档保存任务异常，需检查导入结果".to_string()).and_then(|r|r);
 if let Ok(mut s)=control().lock(){if s.id==request_id{s.phase=if result.is_ok(){"done"}else{"ready"};if result.is_ok(){s.files.clear();}}}result
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn invalid_encoding_never_becomes_garbled_text(){assert!(decoded(&[0xff,0xfe]).is_err());assert_eq!(decoded(b"\xef\xbb\xbfabc\r\ndef").unwrap(),"abc\ndef");assert!(decoded(b"a\x00b").is_err());}
 #[test]fn transaction_failure_rolls_back_and_never_reuses_external_id(){let mut db=rusqlite::Connection::open_in_memory().unwrap();crate::database::migrate(&mut db).unwrap();db.execute_batch("INSERT INTO items(id,kind,title,body,created_at,updated_at) VALUES('existing','note','原文','原正文','2026-01-01','2026-01-01'); CREATE TRIGGER fail_import BEFORE INSERT ON items WHEN NEW.title='失败' BEGIN SELECT RAISE(ABORT,'mock failure'); END;").unwrap();
  let e=formats::entry("external-id".into(),"新条目".into(),"正文".into(),formats::plain_document("正文"));let good=Selection{entry_id:e.id.clone(),title:"正常".into(),kind:"sticky".into()};let bad=Selection{entry_id:e.id.clone(),title:"失败".into(),kind:"note".into()};assert!(insert_batch(&mut db,&[(e.clone(),good),(e.clone(),bad)]).is_err());assert_eq!(db.query_row("SELECT count(*) FROM items",[],|r|r.get::<_,i64>(0)).unwrap(),1);
  let ids=insert_batch(&mut db,&[(e,Selection{entry_id:"external-id".into(),title:"正常".into(),kind:"note".into()})]).unwrap();assert_ne!(ids[0]["id"],"external-id");assert_eq!(db.query_row("SELECT body FROM items WHERE id='existing'",[],|r|r.get::<_,String>(0)).unwrap(),"原正文");}
}
