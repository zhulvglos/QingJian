use crate::{audio,database::Database};
use rusqlite::{Connection,OptionalExtension,params};
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf},io::{Read,Write}};
use sha2::{Digest,Sha256};
use tauri::Manager;
fn err(e:impl std::fmt::Display)->String{e.to_string()}
fn hash(p:&Path)->Result<String,String>{let mut f=fs::File::open(p).map_err(err)?;let mut h=Sha256::new();let mut b=[0;65536];loop{let n=f.read(&mut b).map_err(err)?;if n==0{break;}h.update(&b[..n]);}Ok(format!("{:x}",h.finalize()))}
fn equivalent(a:&Path,b:&Path)->bool{a.canonicalize().unwrap_or_else(|_|a.to_owned()).to_string_lossy().eq_ignore_ascii_case(&b.canonicalize().unwrap_or_else(|_|b.to_owned()).to_string_lossy())}

// 文件系统与 SQLite 不能共用事务。先保留经哈希核验的副本，并记录恢复任务；
// 数据库事务未提交则恢复原音频，已提交则只清理副本，绝不递归删除目录。
fn recover_job(c:&Connection,key:&str,job:&Value)->Result<(),String>{
 let dir=PathBuf::from(job["dir"].as_str().ok_or("删除恢复目录缺失")?);
 if !dir.is_absolute()||dir.to_string_lossy().to_ascii_lowercase().starts_with("c:")||dir.parent().and_then(Path::file_name).and_then(|x|x.to_str())!=Some("record-delete-jobs")||uuid::Uuid::parse_str(dir.file_name().and_then(|x|x.to_str()).unwrap_or("")).is_err(){return Err("删除恢复路径无效，文件未改动".into());}
 let raw:Option<String>=c.query_row("SELECT payload FROM audio_sessions WHERE id=?1",[job["id"].as_str()],|r|r.get(0)).optional().map_err(err)?;
 let record:Option<Value>=raw.map(|s|serde_json::from_str(&s)).transpose().map_err(err)?;
 let files=job["files"].as_array().ok_or("删除恢复清单损坏")?;
 for(i,item)in files.iter().enumerate(){
  let backup=dir.join(format!("{i}.wav"));let original=PathBuf::from(item["original"].as_str().ok_or("原文件路径缺失")?);
  if let Some(ref v)=record{
   if !["mic","system"].iter().any(|k|v[k].as_str().is_some_and(|p|equivalent(Path::new(p),&original))){return Err("录音路径已经变化，恢复副本保留".into());}
   if original.exists(){if hash(&original)?!=item["hash"].as_str().unwrap_or(""){return Err("原位置存在不同文件，未覆盖；恢复副本保留".into());}}
   else{
    if hash(&backup)?!=item["hash"].as_str().unwrap_or(""){return Err("恢复副本校验失败".into());}
    let mut input=fs::File::open(&backup).map_err(err)?;let mut output=fs::OpenOptions::new().write(true).create_new(true).open(&original).map_err(err)?;std::io::copy(&mut input,&mut output).map_err(err)?;output.flush().map_err(err)?;output.sync_all().map_err(err)?;
   }
  }
 }
 for i in 0..files.len(){let p=dir.join(format!("{i}.wav"));if p.is_file(){fs::remove_file(p).map_err(err)?;}}
 if dir.exists(){fs::remove_dir(&dir).map_err(err)?;}c.execute("DELETE FROM app_settings WHERE key=?1",[key]).map_err(err)?;Ok(())
}
pub fn recover_pending(app:&tauri::AppHandle)->Result<(),String>{
 let db=app.state::<Database>();let c=db.0.lock().map_err(err)?;let rows={let mut q=c.prepare("SELECT key,value FROM app_settings WHERE key LIKE 'audio_delete:%'").map_err(err)?;let r=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;r};
 for(key,raw)in rows{let job:Value=serde_json::from_str(&raw).map_err(err)?;if let Err(e)=recover_job(&c,&key,&job){if let Some(id)=job["id"].as_str(){if let Ok(raw)=c.query_row("SELECT payload FROM audio_sessions WHERE id=?1",[id],|r|r.get::<_,String>(0)){if let Ok(mut v)=serde_json::from_str::<Value>(&raw){v["error"]=json!(format!("上次删除未完成：{e}"));c.execute("UPDATE audio_sessions SET payload=?1 WHERE id=?2",params![v.to_string(),id]).map_err(err)?;}}}}}Ok(())
}
#[tauri::command]
pub async fn delete_recording(id:String,delete_audio:bool,app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{
 let _guard=audio::ProcessingGuard::acquire()?;if audio::active()||crate::online::processing(){return Err("请先停止录音并等待当前任务结束，再删除记录".into());}audio::stop_playback();
 recover_pending(&app)?;
 let root=audio::setting(&app,"audio_config")?["root"].as_str().unwrap_or("").to_owned();
 let db=app.state::<Database>();let mut c=db.0.lock().map_err(err)?;
 let key=format!("audio_delete:{id}");if c.query_row("SELECT 1 FROM app_settings WHERE key=?1",[&key],|r|r.get::<_,i32>(0)).optional().map_err(err)?.is_some(){return Err("上次删除仍待恢复，请检查音频权限；记录未再次删除".into());}
 let raw:String=c.query_row("SELECT payload FROM audio_sessions WHERE id=?1",[&id],|r|r.get(0)).map_err(|_|"录音记录不存在")?;let v:Value=serde_json::from_str(&raw).map_err(err)?;
 let mut paths:Vec<PathBuf>=Vec::new();let mut shared=0;
 if delete_audio{
  let others={let mut q=c.prepare("SELECT payload FROM audio_sessions WHERE id<>?1").map_err(err)?;let rows=q.query_map([&id],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;rows.into_iter().map(|s|serde_json::from_str::<Value>(&s).map_err(err)).collect::<Result<Vec<_>,_>>()?};
  for name in ["mic","system"]{if let Some(s)=v[name].as_str(){let p=PathBuf::from(s);if !p.exists(){continue;}if !p.is_absolute()||!p.is_file()||fs::symlink_metadata(&p).map_err(err)?.file_type().is_symlink(){return Err("音频路径异常，未删除记录或文件".into());}
   if others.iter().any(|o|["mic","system"].iter().any(|k|o[k].as_str().is_some_and(|s|equivalent(&p,Path::new(s))))){shared+=1;continue;}
   if !paths.iter().any(|q|equivalent(q,&p)){paths.push(p);}
  }}
 }
 let mut job=None;
 if !paths.is_empty(){let root=PathBuf::from(root);if !root.is_absolute()||root.to_string_lossy().to_ascii_lowercase().starts_with("c:"){return Err("请先选择非 C 盘语音工作目录，以安全处理音频删除".into());}
  let dir=root.join("record-delete-jobs").join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(&dir).map_err(err)?;let mut files=Vec::new();
  for(i,p)in paths.iter().enumerate(){let backup=dir.join(format!("{i}.wav"));fs::copy(p,&backup).map_err(|e|format!("无法准备音频恢复副本：{e}；原文件及记录未删除"))?;fs::OpenOptions::new().write(true).open(&backup).map_err(err)?.sync_all().map_err(err)?;let digest=hash(p)?;if hash(&backup)?!=digest{return Err("音频副本核验失败，未删除原文件".into());}files.push(json!({"original":p,"hash":digest}));}
  let j=json!({"id":id,"dir":dir,"files":files});c.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2)",params![key,j.to_string()]).map_err(err)?;job=Some(j);
 }
 let outcome=(||{let tx=c.transaction().map_err(err)?;for p in &paths{fs::remove_file(p).map_err(|e|format!("录音文件无法删除：{e}"))?;}tx.execute("DELETE FROM audio_sessions WHERE id=?1",[&id]).map_err(err)?;tx.commit().map_err(err)})();
 let cleanup=job.as_ref().map(|j|recover_job(&c,&key,j)).transpose();
 match outcome{Err(e)=>Err(format!("删除失败：{e}；{}",match cleanup{Ok(_)=>"记录保留，音频已还原".into(),Err(r)=>format!("恢复尚未完成：{r}，副本保留，重启后重试")})),Ok(())=>Ok(json!({"deleted":true,"sharedFilesRetained":shared,"warning":cleanup.err().map(|_|"记录已删除，恢复副本待下次启动清理")}))}
}).await.map_err(err)?}
