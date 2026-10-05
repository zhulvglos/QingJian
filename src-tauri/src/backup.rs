use crate::database::{Database, migrate};
use rusqlite::{Connection, types::{Value as SqlValue, ValueRef}, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, HashSet}, path::Path};
use tauri::{Manager, Emitter};

const TABLES: [(&str,&str);4]=[
    ("items","id,kind,title,body,category_id,created_at,updated_at,deleted_at,revision,origin,origin_id,body_json,is_pinned,sort_order"),
    ("pins","item_id,sort_order,created_at"),
    ("reminders","id,item_id,enabled,repeat_rule,created_at,updated_at"),
    ("reminder_occurrences","id,reminder_id,due_at,status,completed_at,is_current,created_at")
];
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
pub struct Backup { format:String, version:u32, created_at:String, tables:BTreeMap<String,Vec<Vec<Value>>>, settings:Value, checksum:String }
fn checksum(b:&Backup)->String {format!("{:x}",Sha256::digest(serde_json::to_vec(&(&b.format,b.version,&b.created_at,&b.tables,&b.settings)).unwrap()))}
fn err(e:impl std::fmt::Display)->String {e.to_string()}
fn valid_preferences(p:&Value)->Result<(),String>{
    if let Some(engine)=p.get("audioConfig").and_then(|c|c.get("engine")){if !["whisper","sensevoice"].contains(&engine.as_str().unwrap_or("")){return Err("备份中的本地转写引擎无效".into());}}
    let o=p.as_object().ok_or("备份设置格式错误")?;
    if o.keys().any(|k| !["theme","fontSize","window","audioConfig","onlineAudio","onlineText"].contains(&k.as_str())){return Err("备份包含不支持的设置字段".into());}
    for(k,allowed)in [("audioConfig",vec!["root","exe","model","engine","sensePython","senseRuntime","senseModels"]),("onlineAudio",vec!["endpoint","model"]),("onlineText",vec!["endpoint","model"])]{if let Some(v)=p.get(k){let o=v.as_object().ok_or("模型设置格式错误")?;if o.iter().any(|(key,val)|!allowed.contains(&key.as_str())||!val.is_string()){return Err("备份模型设置含未知字段或凭据".into());}}}
    if !["mint_morning","new_leaf","warm_apricot","coral","lilac_mist"].contains(&p["theme"].as_str().unwrap_or("")){return Err("备份主题无效".into());}
    if !["small","standard","large"].contains(&p["fontSize"].as_str().unwrap_or("")){return Err("备份字号无效".into());}
    if let Some(w)=p.get("window") {let o=w.as_object().ok_or("窗口设置格式错误")?;for(k,v)in o {if !["alwaysOnTop","edgeHide","autoStart","x","y","height","transparency"].contains(&k.as_str()){return Err("不支持的窗口设置".into());} if k=="transparency"&&v.as_u64().is_none_or(|n|n>70||n%5!=0){return Err("窗口透明度无效".into());} if ["alwaysOnTop","edgeHide","autoStart"].contains(&k.as_str())&&!v.is_boolean(){return Err("窗口开关值无效".into());}}}
    Ok(())
}
pub(crate) fn snapshot(conn:&Connection, preferences:Value)->Result<Backup,String>{
    valid_preferences(&preferences)?;
    // 同一读取事务包含全部关联表，不复制可能仍有 WAL 内容的数据库主文件。
    let tx=conn.unchecked_transaction().map_err(err)?;
    let mut tables=BTreeMap::new();
    for (name,cols) in TABLES {let mut stmt=tx.prepare(&format!("SELECT {cols} FROM {name} ORDER BY 1")).map_err(err)?;let count=stmt.column_count();
        let rows=stmt.query_map([],|r|{let mut row=Vec::new();for i in 0..count {row.push(match r.get_ref(i)?{ValueRef::Null=>Value::Null,ValueRef::Integer(n)=>json!(n),ValueRef::Real(n)=>json!(n),ValueRef::Text(s)=>json!(String::from_utf8_lossy(s)),ValueRef::Blob(_)=>return Err(rusqlite::Error::InvalidQuery)});}Ok(row)}).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
        tables.insert(name.to_string(),rows);
    }
    let mut settings=preferences;
    let raw:Option<String>=tx.query_row("SELECT value FROM app_settings WHERE key='window'",[],|r|r.get(0)).ok();
    if let Some(raw)=raw {if let Ok(w)=serde_json::from_str::<Value>(&raw){let mut safe=serde_json::Map::new();for k in ["alwaysOnTop","edgeHide","autoStart","x","y","height","transparency"] {if let Some(v)=w.get(k){safe.insert(k.into(),v.clone());}} settings["window"]=Value::Object(safe);}}
    // 仅备份公开配置，不导出密钥、测试通过标志或历史请求内容。
    for(db_key,out_key,fields)in [("audio_config","audioConfig",vec!["root","engine","sensePython","senseRuntime","senseModels"]),("online_audio","onlineAudio",vec!["endpoint","model"]),("online_text","onlineText",vec!["endpoint","model"])]{if let Ok(raw)=tx.query_row("SELECT value FROM app_settings WHERE key=?1",[db_key],|r|r.get::<_,String>(0)){if let Ok(v)=serde_json::from_str::<Value>(&raw){let mut safe=serde_json::Map::new();for field in fields{if let Some(value)=v[field].as_str(){safe.insert(field.into(),json!(value));}}if out_key=="audioConfig"{safe.insert("engine".into(),json!("sensevoice"));}settings[out_key]=json!(safe);}}}
    tx.commit().map_err(err)?;
    let mut b=Backup{format:"qingjian-backup".into(),version:2,created_at:chrono::Utc::now().to_rfc3339(),tables,settings,checksum:String::new()};b.checksum=checksum(&b);Ok(b)
}
fn insert(conn:&Connection,name:&str,cols:&str,row:&[Value])->Result<(),String>{
    let expected=cols.split(',').count();if row.len()!=expected{return Err(format!("{name} 字段数量不符"));}
    let values=row.iter().map(|v|match v {Value::Null=>Ok(SqlValue::Null),Value::String(s)=>Ok(SqlValue::Text(s.clone())),Value::Number(n)=>n.as_i64().map(SqlValue::Integer).ok_or("整数超出范围".to_string()),_=>Err("字段类型无效".into())}).collect::<Result<Vec<_>,String>>()?;
    let placeholders=vec!["?";expected].join(",");conn.execute(&format!("INSERT INTO {name}({cols}) VALUES ({placeholders})"),params_from_iter(values)).map_err(|e|format!("{name} 关系或字段无效：{e}"))?;Ok(())
}
fn validate(b:&Backup)->Result<(),String>{
    if b.format!="qingjian-backup"||![1,2].contains(&b.version){return Err("仅支持轻笺新版 .qjbackup 格式版本 1、2；不支持旧版 JSON 或 Markdown".into());}
    if b.checksum!=checksum(b){return Err("备份完整性校验失败，文件可能已损坏或被修改".into());}
    if !b.settings.is_null(){valid_preferences(&b.settings)?;}else if b.version==1{return Err("版本1缺少设置".into());}
    if b.tables.len()!=4||TABLES.iter().any(|(n,_)|!b.tables.contains_key(*n)){return Err("备份缺少必要的数据表".into());}
    let mut test=Connection::open_in_memory().map_err(err)?;test.execute_batch("PRAGMA foreign_keys=ON").map_err(err)?;migrate(&mut test)?;
    let tx=test.transaction().map_err(err)?;
    for(name,cols)in TABLES {if b.tables[name].len()>100_000{return Err("备份条目数量超过支持上限".into());}for row in &b.tables[name]{insert(&tx,name,cols,row)?;}}
    for row in &b.tables["items"] {
        if row[0].as_str().is_none_or(|s|s.is_empty())||!row[2].is_string()||!row[3].is_string(){return Err("内容 ID、标题或正文类型错误".into());}
        if let Some(s)=row[11].as_str(){let v:Value=serde_json::from_str(s).map_err(|_|"正文格式损坏")?;if v["format"]!="qingjian-rich-v1"||v["document"]["type"]!="doc"||!v["document"]["content"].is_array(){return Err("不支持的正文格式".into());}}
        for index in [5,6] {chrono::DateTime::parse_from_rfc3339(row[index].as_str().ok_or("内容时间字段错误")?).map_err(|_|"内容时间无效")?;}
    }
    for row in &b.tables["reminder_occurrences"] {chrono::DateTime::parse_from_rfc3339(row[2].as_str().ok_or("提醒时间字段错误")?).map_err(|_|"提醒时间无效")?;}
    // 提醒归属、唯一当前发生记录、固定关系由同一内存库外键和唯一约束验证。
    Ok(())
}
pub(crate) fn read(path:&str)->Result<Backup,String>{
    let p=Path::new(path);if p.extension().and_then(|x|x.to_str())!=Some("qjbackup"){return Err("请选择 .qjbackup 文件".into());}
    if std::fs::metadata(p).map_err(err)?.len()>50_000_000{return Err("备份文件超过 50MB 上限".into());}
    let b:Backup=serde_json::from_slice(&std::fs::read(p).map_err(err)?).map_err(|e|format!("备份格式无效：{e}"))?;validate(&b)?;Ok(b)
}
pub(crate) fn write(path:&Path,b:&Backup)->Result<(),String>{
    let bytes=serde_json::to_vec_pretty(b).map_err(err)?;
    // 与读取上限一致：超限在创建文件前拒绝，避免周期失败留下大文件并耗尽磁盘。
    if bytes.len()>50_000_000{return Err("内容备份超过 50MB 上限，未创建文件；请减少附件文本或分批导出".into());}
    validate(b)?;
    let tmp=path.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    let result=(||{use std::io::Write;let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(err)?;f.write_all(&bytes).map_err(err)?;f.sync_all().map_err(err)?;drop(f);std::fs::rename(&tmp,path).map_err(|e|format!("写入备份失败（请选择新文件名）：{e}"))})();
    if result.is_err(){let _=std::fs::remove_file(&tmp);}result
}
// 普通文档导入只复用内容快照，不接入备份恢复的外部 ID 或冲突覆盖流程。
pub(crate) fn before_document_import(conn:&Connection,preferences:Value,root:&Path)->Result<std::path::PathBuf,String>{
    let before=snapshot(conn,preferences)?;
    let dir=root.join("backups");std::fs::create_dir_all(&dir).map_err(err)?;
    let path=dir.join(format!("文档导入前-{}.qjbackup",uuid::Uuid::new_v4()));
    write(&path,&before)?;Ok(path)
}
#[tauri::command]
pub async fn choose_backup_path(save:bool)->Result<Option<String>,String>{
    tauri::async_runtime::spawn_blocking(move||crate::native_dialog::run(move||{let d=rfd::FileDialog::new().add_filter("轻笺备份",&["qjbackup"]);let p=if save{d.set_file_name(format!("轻笺备份-{}.qjbackup",chrono::Local::now().format("%Y%m%d-%H%M%S"))).save_file()}else{d.pick_file()};p.map(|p|p.to_string_lossy().to_string())})).await.map_err(err)?
}
#[tauri::command]
pub fn export_backup(path:String,preferences:Value,selected_ids:Option<Vec<String>>,include_settings:Option<bool>,database:tauri::State<'_,Database>)->Result<String,String>{
    if Path::new(&path).extension().and_then(|s|s.to_str())!=Some("qjbackup"){return Err("备份扩展名必须为 .qjbackup".into());}
    let conn=database.0.lock().map_err(err)?;let mut b=snapshot(&conn,preferences)?;drop(conn);select_items(&mut b,selected_ids)?;if include_settings==Some(false){b.settings=Value::Null;}b.checksum=checksum(&b);write(Path::new(&path),&b)?;Ok(path)
}
// 所选主记录向下筛选关系，绝不将未选内容随关联一起带出。
pub(crate) fn select_items(b:&mut Backup,ids:Option<Vec<String>>)->Result<(),String>{
    let Some(ids)=ids else{return Ok(())};let ids:HashSet<String>=ids.into_iter().collect();
    if ids.iter().any(|id|!b.tables["items"].iter().any(|r|r[0].as_str()==Some(id))){return Err("所选条目已变化，请重新读取清单".into());}
    b.tables.get_mut("items").unwrap().retain(|r|ids.contains(r[0].as_str().unwrap_or("")));
    b.tables.get_mut("pins").unwrap().retain(|r|ids.contains(r[0].as_str().unwrap_or("")));
    b.tables.get_mut("reminders").unwrap().retain(|r|ids.contains(r[1].as_str().unwrap_or("")));
    let reminder_ids:HashSet<String>=b.tables["reminders"].iter().filter_map(|r|r[0].as_str().map(str::to_owned)).collect();
    b.tables.get_mut("reminder_occurrences").unwrap().retain(|r|reminder_ids.contains(r[1].as_str().unwrap_or("")));Ok(())
}
pub(crate) fn selected_bytes(mut b:Backup,settings:bool)->Result<Vec<u8>,String>{if !settings{b.settings=Value::Null;}b.checksum=checksum(&b);validate(&b)?;let bytes=serde_json::to_vec_pretty(&b).map_err(err)?;if bytes.len()>50_000_000{return Err("所选备份超过 50MB 上限，请分批导出".into());}Ok(bytes)}
pub(crate) fn content_fingerprint(b:&Backup)->String{format!("{:x}",Sha256::digest(serde_json::to_vec(&(&b.tables,&b.settings)).unwrap()))}
pub(crate) fn item_count(b:&Backup)->usize{b.tables["items"].len()}
pub(crate) fn set_created_at(b:&mut Backup,time:String){b.created_at=time;b.checksum=checksum(b);}
fn entries(b:&Backup,conn:&Connection)->Result<Vec<Value>,String>{b.tables["items"].iter().map(|r|{let conflict=conn.query_row("SELECT EXISTS(SELECT 1 FROM items WHERE id=?1)",[r[0].as_str().unwrap_or("")],|r|r.get::<_,bool>(0)).map_err(err)?;Ok(json!({"id":r[0],"kind":r[1],"title":r[2],"updatedAt":r[6],"deleted":!r[7].is_null(),"conflict":conflict}))}).collect()}
#[tauri::command]
pub fn backup_entries(preferences:Value,database:tauri::State<'_,Database>)->Result<Vec<Value>,String>{let c=database.0.lock().map_err(err)?;entries(&snapshot(&c,preferences)?,&c)}
#[tauri::command]
pub fn preview_backup(path:String,database:tauri::State<'_,Database>)->Result<Value,String>{
    let b=read(&path)?;let conn=database.0.lock().map_err(err)?;let mut conflicts=0;let mut identical=0;
    for r in &b.tables["items"]{let id=r[0].as_str().ok_or("ID 错误")?;if conn.query_row("SELECT EXISTS(SELECT 1 FROM items WHERE id=?1)",[id],|r|r.get::<_,bool>(0)).map_err(err)?{conflicts+=1;}}
    let current=snapshot(&conn,json!({"theme":"warm_apricot","fontSize":"standard"}))?;
    for r in &b.tables["items"]{if current.tables["items"].contains(r){identical+=1;}}
    Ok(json!({"checksum":b.checksum,"createdAt":b.created_at,"entries":entries(&b,&conn)?,"items":b.tables["items"].len(),"trash":b.tables["items"].iter().filter(|r|!r[7].is_null()).count(),"pins":b.tables["pins"].len(),"reminders":b.tables["reminders"].len(),"occurrences":b.tables["reminder_occurrences"].len(),"conflicts":conflicts,"identical":identical,"settings":b.settings}))
}
pub(crate) fn merge(conn:&mut Connection,b:&Backup,policy:&str)->Result<usize,String>{
    merge_with_settings(conn,b,policy,&[])
}
fn merge_with_settings(conn:&mut Connection,b:&Backup,policy:&str,settings:&[(String,Value)])->Result<usize,String>{
    if !["skip","replace"].contains(&policy){return Err("必须明确选择跳过或替换相同 ID 内容".into());}
    let tx=conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(err)?;let mut accepted=HashSet::new();
    for r in &b.tables["items"]{let id=r[0].as_str().ok_or("ID 错误")?;let exists=tx.query_row("SELECT EXISTS(SELECT 1 FROM items WHERE id=?1)",[id],|r|r.get::<_,bool>(0)).map_err(err)?;
        if exists&&policy=="skip"{continue;}if exists{tx.execute("DELETE FROM items WHERE id=?1",[id]).map_err(err)?;}
        insert(&tx,"items",TABLES[0].1,r)?;accepted.insert(id.to_string());
    }
    let mut reminders=HashSet::new();
    for r in &b.tables["pins"] {if accepted.contains(r[0].as_str().unwrap_or("")){insert(&tx,"pins",TABLES[1].1,r)?;}}
    for r in &b.tables["reminders"] {if accepted.contains(r[1].as_str().unwrap_or("")){insert(&tx,"reminders",TABLES[2].1,r)?;reminders.insert(r[0].as_str().unwrap_or("").to_string());}}
    for r in &b.tables["reminder_occurrences"] {if reminders.contains(r[1].as_str().unwrap_or("")){insert(&tx,"reminder_occurrences",TABLES[3].1,r)?;}}
    // 内容和公开配置共用同一事务，任意数据库写入失败整批回滚。
    for(key,value)in settings{tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value.to_string()]).map_err(err)?;}
    tx.commit().map_err(err)?;Ok(accepted.len())
}
#[tauri::command]
pub fn import_backup(path:String,expected_checksum:String,policy:String,preferences:Value,selected_ids:Option<Vec<String>>,apply_settings:Option<bool>,app:tauri::AppHandle)->Result<Value,String>{
    let mut b=read(&path)?;if b.checksum!=expected_checksum{return Err("文件在预览后发生变化，请重新预览".into());}select_items(&mut b,selected_ids)?;
    let apply=apply_settings==Some(true)&&!b.settings.is_null();
    let mut settings=if apply{prepare_media(&b.settings,&app)?}else{vec![]};
    if apply{settings.push(("appearance".into(),json!({"theme":b.settings["theme"],"fontSize":b.settings["fontSize"]})));}
    let old_window=if apply{Some(serde_json::to_value(crate::shell::get_shell_settings(app.clone())?).map_err(err)?)}else{None};
    let db=app.state::<Database>();let before={let conn=db.0.lock().map_err(err)?;snapshot(&conn,preferences)?};
    let dir=app.state::<crate::data_root::DataRoot>().0.join("backups");std::fs::create_dir_all(&dir).map_err(err)?;
    let backup=dir.join(format!("导入前-{}.qjbackup",uuid::Uuid::new_v4()));write(&backup,&before)?;
    if apply{crate::online::cancel_tests();crate::model_news::cancel();}
    if apply{if let Some(window)=b.settings.get("window"){if let Err(error)=crate::shell::apply_backup_window(app.clone(),window.clone()){if let Some(old)=old_window.clone(){let _=crate::shell::apply_backup_window(app.clone(),old);}return Err(format!("窗口设置应用失败，内容未写入：{error}"));}}}
    let result={let mut conn=db.0.lock().map_err(err)?;merge_with_settings(&mut conn,&b,&policy,&settings)};
    let count=match result{Ok(n)=>n,Err(error)=>{if let Some(old)=old_window{let _=crate::shell::apply_backup_window(app.clone(),old);}return Err(format!("恢复已回滚：{error}"));}};
    let _=app.emit_to("quick","quick-items-changed",true);
    Ok(json!({"imported":count,"skipped":b.tables["items"].len()-count,"backupPath":backup,"settings":b.settings,"settingsApplied":apply}))
}
fn prepare_media(settings:&Value,app:&tauri::AppHandle)->Result<Vec<(String,Value)>,String>{
 valid_preferences(settings)?;if crate::audio::active()||crate::audio::processing()||crate::online::processing(){return Err("请先完成录音或正式模型任务，再应用全局设置；内容未写入".into());}let mut updates=vec![];
 if let Some(config)=settings.get("audioConfig"){let mut c=config.clone();for key in ["root","sensePython","senseRuntime","senseModels"]{if let Some(path)=c[key].as_str().filter(|s|!s.is_empty()){if !Path::new(path).is_absolute()||path.to_ascii_lowercase().starts_with("c:"){return Err("备份的语音资源路径必须位于非 C 盘；内容未写入".into());}}}if c["root"].as_str().unwrap_or("").is_empty(){return Err("备份的语音资源保存目录缺失；内容未写入".into());}crate::sensevoice::defaults(&mut c);updates.push(("audio_config".into(),c));updates.push(("audio_validation".into(),json!({})));}
 for (out,key)in [("onlineAudio","online_audio"),("onlineText","online_text")]{if let Some(config)=settings.get(out){let endpoint=config["endpoint"].as_str().unwrap_or("").trim_end_matches('/');let url=reqwest::Url::parse(endpoint).map_err(|_|"备份的模型地址无效；内容未写入")?;let local=std::env::var_os("QINGJIAN_TEST_MODE").is_some()&&url.scheme()=="http"&&matches!(url.host_str(),Some("localhost"|"127.0.0.1"));if (url.scheme()!="https"&&!local)||!url.username().is_empty()||url.password().is_some()||url.query().is_some()||url.fragment().is_some()||config["model"].as_str().unwrap_or("").trim().is_empty(){return Err("备份模型连接配置无效；内容未写入".into());}let current=crate::audio::setting(app,key)?;validate_endpoint_restore(current["endpoint"].as_str(),endpoint)?;
 // 不访问 Windows 凭据接口。地址不变时沿用原凭据；地址不同则在任何写入前拒绝，防止密钥误发。
 updates.push((key.into(),json!({"endpoint":endpoint,"model":config["model"],"revision":uuid::Uuid::new_v4().to_string(),"tested":false,"testedAt":Value::Null})));}}
 Ok(updates)
}
fn validate_endpoint_restore(current:Option<&str>,wanted:&str)->Result<(),String>{if current.filter(|s|!s.is_empty()).is_some_and(|s|s.trim_end_matches('/')!=wanted){Err("备份服务地址与当前地址不同。为保护系统凭据，请取消应用全局设置，恢复内容后在模型设置手动核对；未写入任何内容".into())}else{Ok(())}}
#[tauri::command]
pub fn apply_backup_media(settings:Value,app:tauri::AppHandle)->Result<(),String>{let updates=prepare_media(&settings,&app)?;crate::online::cancel_tests();crate::model_news::cancel();let db=app.state::<Database>();let mut c=db.0.lock().map_err(err)?;let tx=c.transaction().map_err(err)?;for(key,value)in updates{tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value.to_string()]).map_err(err)?;}tx.commit().map_err(err)}

#[cfg(test)] mod tests {
    use super::*;
    #[test]fn endpoint_restore_preserves_credential_identity(){assert!(validate_endpoint_restore(Some("https://example.test/v1/"),"https://example.test/v1").is_ok());assert!(validate_endpoint_restore(Some("https://example.test/v1"),"https://other.test/v1").is_err());assert!(validate_endpoint_restore(None,"https://example.test/v1").is_ok());}
    #[test]fn configuration_failure_rolls_back_content(){let(mut c,mut b)=sample();b.tables.get_mut("items").unwrap()[0][2]=json!("不应提交");c.execute_batch("CREATE TRIGGER reject_appearance BEFORE INSERT ON app_settings WHEN NEW.key='appearance' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();assert!(merge_with_settings(&mut c,&b,"replace",&[("appearance".into(),json!({"theme":"warm_apricot","fontSize":"standard"}))]).is_err());assert_eq!(c.query_row("SELECT title FROM items",[],|r|r.get::<_,String>(0)).unwrap(),"保留标题");assert_eq!(c.query_row("SELECT count(*) FROM pins",[],|r|r.get::<_,i64>(0)).unwrap(),1);}
    #[test]fn local_engine_settings_survive_backup_without_credentials(){
        let (c,_)=sample();
        let config=json!({"root":"F:\\audio","exe":"F:\\whisper.exe","model":"F:\\base.bin","engine":"sensevoice","sensePython":"F:\\python.exe","senseRuntime":"F:\\runtime","senseModels":"F:\\models"});
        c.execute("INSERT INTO app_settings(key,value) VALUES('audio_config',?1)",[config.to_string()]).unwrap();
        let b=snapshot(&c,json!({"theme":"warm_apricot","fontSize":"standard"})).unwrap();
        let mut expected=config.clone();crate::sensevoice::defaults(&mut expected);validate(&b).unwrap();assert_eq!(b.settings["audioConfig"],expected);
        let mut invalid=b.clone();invalid.settings["audioConfig"]["engine"]=json!("unknown");assert!(valid_preferences(&invalid.settings).is_err());
        invalid.settings["audioConfig"]=json!({"root":"F:\\audio","exe":"F:\\old.exe","model":"F:\\old.bin"});assert!(valid_preferences(&invalid.settings).is_ok());
        invalid.settings["audioConfig"]["apiKey"]=json!("must-not-export");assert!(valid_preferences(&invalid.settings).is_err());
    }
    fn sample()->(Connection,Backup){let mut c=Connection::open_in_memory().unwrap();c.execute_batch("PRAGMA foreign_keys=ON").unwrap();migrate(&mut c).unwrap();c.execute_batch("INSERT INTO items(id,kind,title,body,created_at,updated_at) VALUES('a','sticky','保留标题','正文','2026-09-26T00:00:00Z','2026-09-26T00:00:00Z'); INSERT INTO pins VALUES('a',3,'2026-09-26T00:00:00Z');").unwrap();let b=snapshot(&c,json!({"theme":"warm_apricot","fontSize":"standard"})).unwrap();(c,b)}
    #[test]fn restore_dedup_and_conflicts(){let(mut c,b)=sample();validate(&b).unwrap();assert_eq!(merge(&mut c,&b,"skip").unwrap(),0);c.execute("UPDATE items SET title='修改'",[]).unwrap();assert_eq!(merge(&mut c,&b,"replace").unwrap(),1);assert_eq!(c.query_row("SELECT title FROM items",[],|r|r.get::<_,String>(0)).unwrap(),"保留标题");assert_eq!(c.query_row("SELECT count(*) FROM pins",[],|r|r.get::<_,i64>(0)).unwrap(),1);}
    #[test]fn failure_rolls_back(){let(mut c,mut b)=sample();b.tables.get_mut("pins").unwrap()[0][0]=json!("missing");assert!(validate(&b).is_err());b.tables.get_mut("items").unwrap()[0][2]=json!("不应提交");b.tables.get_mut("reminders").unwrap().push(vec![json!("r"),json!("a"),json!(9),json!("once"),json!("t"),json!("t")]);assert!(merge(&mut c,&b,"replace").is_err());assert_eq!(c.query_row("SELECT title FROM items",[],|r|r.get::<_,String>(0)).unwrap(),"保留标题");}
    #[test]fn selection_never_includes_unselected_relations(){let(c,_)=sample();c.execute_batch("INSERT INTO items(id,kind,title,body,created_at,updated_at) VALUES('b','note','不导出','私有正文','2026-09-26T00:00:00Z','2026-09-26T00:00:00Z'); INSERT INTO pins VALUES('b',4,'2026-09-26T00:00:00Z');").unwrap();let mut b=snapshot(&c,json!({"theme":"warm_apricot","fontSize":"standard"})).unwrap();select_items(&mut b,Some(vec!["a".into()])).unwrap();assert_eq!(b.tables["items"].len(),1);assert_eq!(b.tables["pins"].len(),1);assert_eq!(b.tables["pins"][0][0],"a");b.settings=Value::Null;b.checksum=checksum(&b);validate(&b).unwrap();assert!(!serde_json::to_string(&b).unwrap().contains("私有正文"));assert!(select_items(&mut b,Some(vec!["不存在".into()])).is_err());}
    #[test]fn version_one_remains_readable(){let(_,mut b)=sample();b.version=1;b.checksum=checksum(&b);validate(&b).unwrap();}

}
