use crate::database::Database;
use serde_json::{json,Value};
use rusqlite::{params,OptionalExtension};
use std::{path::{Path,PathBuf},sync::{atomic::{AtomicBool,Ordering},Mutex}};
use tauri::Manager;

fn err(e:impl std::fmt::Display)->String{e.to_string()}
static PROCESSING:AtomicBool=AtomicBool::new(false);
static PROCESSING_SINCE:Mutex<Option<String>>=Mutex::new(None);
static LAST_PROCESSING:Mutex<Option<(String,String)>>=Mutex::new(None);
pub fn processing()->bool{PROCESSING.load(Ordering::Relaxed)}
pub(crate) struct ProcessingGuard;
impl ProcessingGuard { pub fn acquire()->Result<Self,String>{if PROCESSING.swap(true,Ordering::SeqCst){Err("已有音频任务正在处理，请稍候".into())}else{if let Ok(mut since)=PROCESSING_SINCE.lock(){*since=Some(chrono::Utc::now().to_rfc3339());}Ok(Self)}} }
impl Drop for ProcessingGuard{fn drop(&mut self){if let Ok(mut since)=PROCESSING_SINCE.lock(){if let Some(start)=since.take(){if let Ok(mut last)=LAST_PROCESSING.lock(){*last=Some((start,chrono::Utc::now().to_rfc3339()));}}}PROCESSING.store(false,Ordering::SeqCst);}}
pub fn task_snapshot()->Value{let since=PROCESSING_SINCE.lock().ok().and_then(|s|s.clone());let last=LAST_PROCESSING.lock().ok().and_then(|s|s.clone());json!({"kind":"audio_processing","label":"录音或转写处理","status":if processing(){"running"}else if last.is_some(){"finished"}else{"idle"},"startedAt":since.or_else(||last.as_ref().map(|s|s.0.clone())),"endedAt":last.map(|s|s.1),"canCancel":false})}
pub fn active()->bool{crate::recording_capture::active()}
pub fn setting(app:&tauri::AppHandle,key:&str)->Result<Value,String>{let db=app.state::<Database>();let c=db.0.lock().map_err(err)?;let raw:Option<String>=c.query_row("SELECT value FROM app_settings WHERE key=?1",[key],|r|r.get(0)).optional().map_err(err)?;raw.map(|s|serde_json::from_str(&s).map_err(err)).unwrap_or(Ok(json!({})))}
pub fn save_setting(app:&tauri::AppHandle,key:&str,v:&Value)->Result<(),String>{app.state::<Database>().0.lock().map_err(err)?.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,serde_json::to_string(v).map_err(err)?]).map_err(err)?;Ok(())}
fn non_c(p:&Path)->Result<(),String>{if !p.is_absolute()||p.to_string_lossy().to_ascii_lowercase().starts_with("c:"){return Err("请选择非 C 盘绝对路径".into());}Ok(())}
#[tauri::command]pub fn audio_config(app:tauri::AppHandle)->Result<Value,String>{let mut v=setting(&app,"audio_config")?;if v["root"].as_str().is_none_or(str::is_empty){v["root"]=json!(app.state::<crate::data_root::DataRoot>().0.join("voice"));}let previous=v.clone();crate::sensevoice::defaults(&mut v);if v!=previous{save_setting(&app,"audio_config",&v)?;}v.as_object_mut().map(|o|o.remove("validation"));v["available"]=json!(crate::sensevoice::available(&v));let checked=setting(&app,"audio_validation")?;if checked["fingerprint"]==json!(local_fingerprint(&v)){v["validation"]=checked;}v["installation"]=setting(&app,"sensevoice_install")?;v["processing"]=json!(processing());Ok(v)}
#[tauri::command]pub fn save_audio_config(config:Value,app:tauri::AppHandle)->Result<Value,String>{if active()||processing(){return Err("请先停止录音或等待模型任务完成，再修改路径".into());}let mut v=json!({});for k in ["root","sensePython","senseRuntime","senseModels"]{let s=config[k].as_str().unwrap_or("");if !s.is_empty(){non_c(Path::new(s))?;}v[k]=json!(s);}if v["root"]==""{return Err("请选择音频与模型保存目录".into());}// 兼容旧备份的引擎字段，统一迁移到 SenseVoice，保留录音和历史版本。
v["engine"]=json!("sensevoice");for k in ["sensePython","senseRuntime","senseModels"]{if v[k]==""{v.as_object_mut().unwrap().remove(k);}}crate::sensevoice::defaults(&mut v);save_setting(&app,"audio_config",&v)?;audio_config(app)}
#[tauri::command]pub async fn choose_audio_path(kind:String)->Result<Option<String>,String>{tauri::async_runtime::spawn_blocking(move||{let d=rfd::FileDialog::new();let p=match kind.as_str(){"root"|"senseRuntime"|"senseModels"=>d.pick_folder(),"sensePython"=>d.add_filter("运行程序",&["exe"]).pick_file(),_=>return Err("不支持的路径类型".into())};if let Some(p)=p{non_c(&p)?;Ok(Some(p.to_string_lossy().to_string()))}else{Ok(None)}}).await.map_err(err)?}
pub fn session(app:&tauri::AppHandle,id:&str)->Result<Value,String>{let raw:String=app.state::<Database>().0.lock().map_err(err)?.query_row("SELECT payload FROM audio_sessions WHERE id=?1",[id],|r|r.get(0)).map_err(err)?;serde_json::from_str(&raw).map_err(err)}
pub fn put_session(app:&tauri::AppHandle,v:&Value)->Result<(),String>{app.state::<Database>().0.lock().map_err(err)?.execute("INSERT INTO audio_sessions(id,created_at,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![v["id"].as_str(),v["createdAt"].as_str(),serde_json::to_string(v).map_err(err)?]).map_err(err)?;Ok(())}
#[tauri::command]pub fn list_recordings(app:tauri::AppHandle)->Result<Vec<Value>,String>{let is_active=active();let db=app.state::<Database>();let c=db.0.lock().map_err(err)?;let mut s=c.prepare("SELECT payload FROM audio_sessions ORDER BY created_at DESC").map_err(err)?;let rows=s.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;rows.iter().map(|r|{let mut v:Value=serde_json::from_str(r).map_err(err)?;if v["state"]=="recording"&&!is_active{v["state"]=json!("interrupted");}for k in ["mic","system"]{v[format!("{k}Bytes")]=json!(std::fs::metadata(v[k].as_str().unwrap_or("")).map(|m|m.len()).unwrap_or(0));}Ok(v)}).collect()}
// 使用Windows原生WAV播放，避免机器未关联.wav时无法重听；不打开外部程序。
#[link(name="winmm")]extern "system"{fn PlaySoundW(sound:*const u16,module:*mut std::ffi::c_void,flags:u32)->i32;}
#[tauri::command]pub fn stop_playback(){crate::audio_workspace::stop_seek_playback();unsafe{PlaySoundW(std::ptr::null(),std::ptr::null_mut(),0);}}
#[tauri::command]pub fn play_recording(id:String,track:String,app:tauri::AppHandle)->Result<(),String>{
 if active(){return Err("请先停止录音再重听，避免回放混入录音".into());}if !["mic","system"].contains(&track.as_str()){return Err("音轨无效".into());}let v=session(&app,&id)?;let p=Path::new(v[&track].as_str().ok_or("音频路径缺失")?);if !p.is_file(){return Err("音频不存在；请检查原保存目录".into());}use std::os::windows::ffi::OsStrExt;let wide:Vec<u16>=p.as_os_str().encode_wide().chain(Some(0)).collect();if wide.len()>256{return Err("音频路径过长，Windows播放接口不支持；请使用较短的非C盘保存目录".into());}if unsafe{PlaySoundW(wide.as_ptr(),std::ptr::null_mut(),0x20003)}==0{return Err("音频播放失败，请检查输出设备与WAV文件".into());}Ok(())
}
pub fn add_version(app:&tauri::AppHandle,id:&str,kind:&str,text:String,mut source:Value)->Result<Value,String>{
 // 同一数据库锁内读取并追加，避免同时完成的本地/在线任务覆盖彼此版本。
 let engine=if kind=="local"{Some(source.get("engine").cloned().unwrap_or_else(||json!("sensevoice")))}else{None};let db=app.state::<Database>();let c=db.0.lock().map_err(err)?;let raw:String=c.query_row("SELECT payload FROM audio_sessions WHERE id=?1",[id],|r|r.get(0)).map_err(err)?;let mut v:Value=serde_json::from_str(&raw).map_err(err)?;
 if kind=="online"{if let Some(reason)=v.get("fallbackReason"){source["fallbackReason"]=reason.clone();}}v["error"]=json!("");
 v["versions"].as_array_mut().ok_or("版本记录损坏")?.push(json!({"id":uuid::Uuid::new_v4().to_string(),"kind":kind,"engine":engine,"source":source,"createdAt":chrono::Utc::now().to_rfc3339(),"text":text}));c.execute("UPDATE audio_sessions SET payload=?1 WHERE id=?2",params![serde_json::to_string(&v).map_err(err)?,id]).map_err(err)?;Ok(v)
}
// 分段转写保留真实音轨来源，不据此推断发言人身份。
pub(crate) fn local_sync(id:String,app:tauri::AppHandle)->Result<Value,String>{crate::audio_segments::transcribe(&app,&id,"local",None)}
#[tauri::command]pub async fn transcribe_local(id:String,app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{let _guard=ProcessingGuard::acquire()?;local_sync(id,app)}).await.map_err(err)?}
// 校验结果绑定路径、长度与修改时间，文件变化后不能沿用旧的成功状态。
pub(crate) fn local_fingerprint(config:&Value)->String{
 let paths=crate::sensevoice::files(config);
 paths.iter().map(|p|{let meta=std::fs::metadata(p).ok();format!("{}:{:?}:{:?}",p.display(),meta.as_ref().map(|m|m.len()),meta.and_then(|m|m.modified().ok()))}).collect::<Vec<_>>().join("|")
}
pub(crate) fn check_sync(app:tauri::AppHandle)->Result<String,String>{ if active(){return Err("请先停止录音".into());}
 let c=audio_config(app.clone())?;
 save_setting(&app,"audio_validation",&json!({}))?;
 if c["available"]!=true{return Err("运行程序或模型文件不存在".into());}
 let root=PathBuf::from(c["root"].as_str().ok_or("缺少保存目录")?).join("checks");non_c(&root)?;std::fs::create_dir_all(&root).map_err(err)?;
 let p=root.join(format!("sample-{}.wav",uuid::Uuid::new_v4()));
 std::fs::write(&p,include_bytes!("../fixtures/asr-connection-test.wav")).map_err(err)?;
 let result=crate::sensevoice::run(&c,&p,true,Some(&app));let _=std::fs::remove_file(&p);let text=result?;
 // 使用程序自带语音核验实际识别，不把文件存在或静音推理当成转写通过。
 if ["语音","测试","今天","任务","核对","模型","连接"].iter().filter(|word|text.contains(**word)).count()<2{return Err("模型已运行，但测试语音未正确识别；请核对模型与语言".into());}
 save_setting(&app,"audio_validation",&json!({"fingerprint":local_fingerprint(&c),"integrity":c["engine"]=="sensevoice","transcribed":true}))?;
 Ok("实际转写通过".into())
}
#[tauri::command]pub async fn check_local_model(app:tauri::AppHandle)->Result<String,String>{tauri::async_runtime::spawn_blocking(move||{let _guard=ProcessingGuard::acquire()?;check_sync(app)}).await.map_err(err)?}
// 唯一本地引擎为 SenseVoice，失败保留原始音频，不自动转为在线上传。
