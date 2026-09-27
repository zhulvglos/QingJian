use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf},io::Read,os::windows::process::CommandExt};
use tauri::Emitter;
use wasapi::*;
use sha2::{Digest,Sha256};
use crate::audio::{self,ProcessingGuard};
fn err(e:impl std::fmt::Display)->String{e.to_string()}

#[tauri::command]
pub async fn audio_devices()->Result<Value,String>{tauri::async_runtime::spawn_blocking(||{
    initialize_mta().ok().map_err(err)?;let en=DeviceEnumerator::new().map_err(err)?;let mut out=json!({});
    for (key,dir) in [("mic",Direction::Capture),("system",Direction::Render)] {
        let collection=en.get_device_collection(&dir).map_err(err)?;let mut list=Vec::new();
        for i in 0..collection.get_nbr_devices().map_err(err)? {let d=collection.get_device_at_index(i).map_err(err)?;list.push(json!({"id":d.get_id().map_err(err)?,"name":d.get_friendlyname().map_err(err)?}));}
        out[key]=json!(list);
    }Ok(out)
}).await.map_err(err)?}

// 整个停止后流程持有同一任务锁；本地样例预检失败才允许一次在线回退。
#[tauri::command]
pub async fn transcribe_recording(id:String,app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{
    let _guard=ProcessingGuard::acquire()?;if audio::active(){return Err("请先停止录音".into());}
    let mut v=audio::session(&app,&id)?;
    let send=|revision:&str|->Result<Value,String>{
        let c=crate::online::online_config("audio".into(),app.clone())?;
        if c["tested"]!=true||c["hasKey"]!=true||c["revision"]!=revision{return Err("在线配置不可用或已变更；请配置并测试语音模型，录音仍保留".into());}
        let _=app.emit("recording-stage",json!({"id":id,"message":format!("在线转写 · {}",c["model"].as_str().unwrap_or(""))}));
        crate::online::process_sync(id.clone(),"audio".into(),None,revision.into(),None,app.clone())
    };
    let result=if v["mode"]=="online" {
        send(v["onlineRevision"].as_str().unwrap_or(""))
    }else{
        let _=app.emit("recording-stage",json!({"id":id,"message":"正在检查 SenseVoice…"}));
        match audio::check_sync(app.clone()) {
            Ok(_)=>{let _=app.emit("recording-stage",json!({"id":id,"message":"本地转写 · SenseVoiceSmall"}));audio::local_sync(id.clone(),app.clone())},
            Err(reason)=>{
                if let Some(revision)=v["fallbackRevision"].as_str().filter(|s|!s.is_empty()).map(str::to_owned){
                    v["fallbackReason"]=json!(reason);audio::put_session(&app,&v)?;
                    let _=app.emit("recording-stage",json!({"id":id,"message":"本地预检失败，正在使用已授权的在线语音模型…"}));send(&revision)
                }else{Err(format!("本地不可用：{reason}。未获授权的在线服务不会上传；请配置语音模型后重试，音频已保留"))}
            }
        }
    };
    if let Err(ref e)=result{let mut latest=audio::session(&app,&id)?;latest["error"]=json!(e);audio::put_session(&app,&latest)?;}
    result
}).await.map_err(err)?}

// 只变更本次的发送授权；旧录音重试也须在当前页面展示相同告知。
#[tauri::command]
pub fn set_recording_route(id:String,mode:String,revision:Option<String>,fallback:Option<String>,app:tauri::AppHandle)->Result<(),String>{
    let _guard=ProcessingGuard::acquire()?;if audio::active(){return Err("录音中不能变更转写方式".into());}
    if !["local","online"].contains(&mode.as_str()){return Err("转写方式无效".into());}
    let mut v=audio::session(&app,&id)?;v["mode"]=json!(mode);v["onlineRevision"]=json!(revision);v["fallbackRevision"]=json!(fallback);v["error"]=json!("");v.as_object_mut().unwrap().remove("fallbackReason");audio::put_session(&app,&v)
}
fn verify_wav(path:&Path)->Result<(),String>{let r=hound::WavReader::open(path).map_err(|_|"音轨缺失或损坏；没有显示保存成功")?;if r.duration()==0{return Err("音轨没有有效声音数据；两路尚未完整保存".into());}Ok(())}
fn hash(path:&Path)->Result<Vec<u8>,String>{let mut f=fs::File::open(path).map_err(err)?;let mut h=Sha256::new();let mut b=[0;65536];loop{let n=f.read(&mut b).map_err(err)?;if n==0{break;}h.update(&b[..n]);}Ok(h.finalize().to_vec())}
fn copy_pair(v:&Value,base:&Path)->Result<PathBuf,String>{
    let id=v["id"].as_str().ok_or("录音ID缺失")?;uuid::Uuid::parse_str(id).map_err(err)?;
    for k in ["mic","system"]{verify_wav(Path::new(v[k].as_str().ok_or("音轨路径缺失")?))?;}
    let dest=base.join(format!("轻笺录音-{}",uuid::Uuid::new_v4()));fs::create_dir(&dest).map_err(err)?;
    // 两轨均复制并校验后，才更新数据库为已保存。失败不删除原始临时文件。
    for k in ["mic","system"]{let from=Path::new(v[k].as_str().unwrap());let to=dest.join(format!("{k}.wav"));fs::copy(from,&to).map_err(|e|format!("复制失败：{e}；原音频保留，目标未标记为完整保存"))?;fs::OpenOptions::new().write(true).open(&to).map_err(err)?.sync_all().map_err(err)?;if hash(from)?!=hash(&to)?{return Err("音轨校验失败；原音频保留".into());}}
    Ok(dest)
}
#[tauri::command]
pub async fn save_recording_audio(id:String,app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{
    let _guard=ProcessingGuard::acquire()?;if audio::active(){return Err("请先停止录音".into());}let mut v=audio::session(&app,&id)?;
    let Some(base)=rfd::FileDialog::new().set_title("选择保存双路录音的目录").pick_folder()else{return Ok(v)};
    let dest=copy_pair(&v,&base)?;let old=v.clone();v["mic"]=json!(dest.join("mic.wav"));v["system"]=json!(dest.join("system.wav"));v["savedDir"]=json!(dest);v["storage"]=json!("saved");v["savedAt"]=json!(chrono::Utc::now().to_rfc3339());audio::put_session(&app,&v)?;
    // 数据库已指向经验证的双轨副本之后，只清理本次托管临时文件。
    if old["storage"]=="temporary" {if let Some(dir)=old["temporaryDir"].as_str().and_then(|p|Path::new(p).canonicalize().ok()) {
        if dir.file_name().and_then(|n|n.to_str())==Some(&id)&&dir.parent().and_then(|p|p.file_name()).and_then(|n|n.to_str())==Some("temporary-recordings") {
            for k in ["mic","system"] {if let Some(p)=old[k].as_str().and_then(|p|Path::new(p).canonicalize().ok()){if p.parent()==Some(dir.as_path())&&p.file_name().and_then(|n|n.to_str())==Some(&format!("{k}.wav")){let _=fs::remove_file(p);}}}let _=fs::remove_dir(dir);
        }
    }}Ok(v)
}).await.map_err(err)?}
#[tauri::command]
pub fn open_recording_folder(id:String,app:tauri::AppHandle)->Result<(),String>{
    let v=audio::session(&app,&id)?;
    if matches!(v["storage"].as_str(),Some("temporary"|"discarded")){return Err("未保存录音".into());}
    // 只定位当前记录实际关联的文件；旧记录没有 savedDir 时也不猜测默认目录。
    let paths=["mic","system"].iter().filter_map(|k|v[k].as_str()).filter(|s|!s.is_empty()).map(Path::new).collect::<Vec<_>>();
    if paths.iter().any(|p|!p.is_file()){return Err("录音文件不存在".into());}
    let p=paths.first().ok_or("录音文件不存在")?;
    let path=p.to_str().ok_or("录音文件路径无效")?;if path.contains('"')||path.contains('\0'){return Err("录音文件路径无效".into());}
    // Explorer 要求 /select 与带引号的路径在同一参数中，否则只打开目录而不选中文件。
    std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).creation_flags(0x08000000).spawn().map_err(|_|"无法打开录音目录")?;Ok(())
}
#[tauri::command]
pub fn discard_recording_audio(id:String,app:tauri::AppHandle)->Result<Value,String>{
    let _guard=ProcessingGuard::acquire()?;if audio::active()||crate::online::processing(){return Err("音频仍在使用，请任务结束后再处理".into());}audio::stop_playback();let mut v=audio::session(&app,&id)?;
    if v["storage"]!="temporary"{return Err("仅能清理尚未保留的临时录音；已保存录音和旧版历史不删除".into());}
    let dir=PathBuf::from(v["temporaryDir"].as_str().ok_or("不是受管理的临时录音")?).canonicalize().map_err(err)?;
    if dir.file_name().and_then(|n|n.to_str())!=Some(&id)||dir.parent().and_then(|p|p.file_name()).and_then(|n|n.to_str())!=Some("temporary-recordings"){return Err("临时路径校验失败，未删除".into());}
    // 只删除当前录音的两份已知文件，不递归删除目录，不触及已导出录音。
    for k in ["mic","system"]{let p=PathBuf::from(v[k].as_str().ok_or("音轨路径缺失")?);if p.exists(){let real=p.canonicalize().map_err(err)?;if real.parent()!=Some(dir.as_path())||real.file_name().and_then(|n|n.to_str())!=Some(&format!("{k}.wav")){return Err("音轨不属于当前临时目录，未删除".into());}}}
    for k in ["mic","system"]{let p=Path::new(v[k].as_str().unwrap());if p.exists(){fs::remove_file(p).map_err(err)?;}}
    let _=fs::remove_dir(&dir);v["storage"]=json!("discarded");audio::put_session(&app,&v)?;Ok(v)
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn empty_track_never_reports_pair_saved(){let dir=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir(&dir).unwrap();let p=dir.join("empty.wav");hound::WavWriter::create(&p,hound::WavSpec{channels:1,sample_rate:16000,bits_per_sample:16,sample_format:hound::SampleFormat::Int}).unwrap().finalize().unwrap();let v=json!({"id":uuid::Uuid::new_v4().to_string(),"mic":p,"system":p});assert!(copy_pair(&v,&dir).is_err());fs::remove_file(p).unwrap();fs::remove_dir(dir).unwrap();}
}
#[tauri::command]
pub async fn preview_audio_inputs(mic_device:Option<String>,system_device:Option<String>)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{
    if audio::active()||audio::processing(){return Ok(json!({}));}
    initialize_mta().ok().map_err(err)?;let en=DeviceEnumerator::new().map_err(err)?;let mut out=json!({});
    for (key,dir,id) in [("mic",Direction::Capture,mic_device),("system",Direction::Render,system_device)]{
        let result=(||{let d=if let Some(id)=id{en.get_device(&id)}else{en.get_default_device(&dir)}.map_err(err)?;let mut client=d.get_iaudioclient().map_err(err)?;
            let format=WaveFormat::new(16,16,&SampleType::Int,16000,1,None);client.initialize_client(&format,&Direction::Capture,&StreamMode::EventsShared{autoconvert:true,buffer_duration_hns:200_000}).map_err(err)?;let event=client.set_get_eventhandle().map_err(err)?;let capture=client.get_audiocaptureclient().map_err(err)?;client.start_stream().map_err(err)?;
            let mut bytes=std::collections::VecDeque::new();let start=std::time::Instant::now();let mut peak=0f32;let read=(||{while start.elapsed()<std::time::Duration::from_millis(180){let _=event.wait_for_event(60);capture.read_from_device_to_deque(&mut bytes).map_err(err)?;while bytes.len()>=2{let n=i16::from_le_bytes([bytes.pop_front().unwrap(),bytes.pop_front().unwrap()]);peak=peak.max((n as f32/32768.).abs());}}Ok::<(),String>(())})();let _=client.stop_stream();read?;Ok::<Value,String>(json!({"peak":peak,"state":"ready"}))})();
        out[key]=result.unwrap_or_else(|e|json!({"state":"error","error":e}));
    }Ok(out)
}).await.map_err(err)?}
