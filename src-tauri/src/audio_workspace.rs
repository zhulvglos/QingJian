use crate::audio;
use serde_json::{json,Value};
use std::{collections::BTreeMap,path::Path};
use tauri::Manager;
fn clock(ms:u64)->String{let s=ms/1000;format!("{:02}:{:02}:{:02}",s/3600,s/60%60,s%60)}
pub(crate) fn duration_seconds(v:&Value)->Option<f64>{
    // captureVersion=2 的 seconds 已扣除暂停；旧版不拿开始/结束时间猜测时长。
    let recorded=v["effectiveSeconds"].as_f64().or_else(||if v["captureVersion"].as_u64().unwrap_or(0)>=2{v["seconds"].as_f64()}else{None});
    if let Some(n)=recorded.filter(|n|n.is_finite()&&*n>=0.){return Some(n);}
    ["mic","system"].iter().filter_map(|k|v[k].as_str()).filter_map(|p|hound::WavReader::open(p).ok()).filter(|r|r.spec().sample_rate>0).map(|r|r.duration()as f64/r.spec().sample_rate as f64).reduce(f64::max)
}
pub(crate) fn with_metadata(mut v:Value)->Value{v["durationSeconds"]=json!(duration_seconds(&v));v}
#[cfg(test)]mod duration_tests{
 use super::*;
 #[test]fn reliable_paused_duration_is_not_wall_clock(){assert_eq!(duration_seconds(&json!({"captureVersion":2,"seconds":17.2,"createdAt":"2026-01-01T00:00:00Z","stoppedAt":"2026-01-01T01:00:00Z"})),Some(17.2));assert_eq!(duration_seconds(&json!({"captureVersion":2,"seconds":8000.})),Some(8000.));}
 #[test]fn unknown_legacy_duration_is_not_invented(){assert_eq!(duration_seconds(&json!({"seconds":999,"mic":"missing.wav","system":"absent.wav"})),None);}
 #[test]fn parallel_tracks_use_max_not_sum(){let dir=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());std::fs::create_dir(&dir).unwrap();for (name,n)in[("mic",16000),("system",32000)]{let mut w=hound::WavWriter::create(dir.join(format!("{name}.wav")),hound::WavSpec{channels:1,sample_rate:16000,bits_per_sample:16,sample_format:hound::SampleFormat::Int}).unwrap();for _ in 0..n{w.write_sample(0i16).unwrap();}w.finalize().unwrap();}assert_eq!(duration_seconds(&json!({"mic":dir.join("mic.wav"),"system":dir.join("system.wav")})),Some(2.));for n in["mic","system"]{std::fs::remove_file(dir.join(format!("{n}.wav"))).unwrap();}std::fs::remove_dir(dir).unwrap();}
}
#[tauri::command]
pub fn list_recording_headers(app:tauri::AppHandle)->Result<Vec<Value>,String>{
    let db=app.state::<crate::database::Database>();let c=db.0.lock().map_err(|e|e.to_string())?;let mut q=c.prepare("SELECT payload FROM audio_sessions ORDER BY created_at DESC").map_err(|e|e.to_string())?;
    let rows=q.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;let mut result=Vec::new();
    for row in rows{let v:Value=serde_json::from_str(&row.map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;result.push(json!({"id":v["id"],"createdAt":v["createdAt"],"seconds":v["seconds"],"state":v["state"],"storage":v["storage"]}));}Ok(result)
}

#[tauri::command]
pub fn save_recording_text(id:String,version_id:String,text:String,edits:Option<BTreeMap<String,String>>,app:tauri::AppHandle)->Result<Value,String>{
    let _guard=audio::ProcessingGuard::acquire()?;if text.len()>4_000_000{return Err("文字超过4MB，草稿保留".into());}
    let v=audio::session(&app,&id)?;let original=v["versions"].as_array().and_then(|vs|vs.iter().find(|s|s["id"]==version_id)).ok_or("原版本不存在，未覆盖内容")?;
    let mut source=original["source"].clone();if !source.is_object(){source=json!({"kind":"historical","model":original["engine"]});}
    source["edited"]=json!(true);source["editedFrom"]=json!(version_id);source["engine"]=original["engine"].clone();
    let saved_text=if let Some(edits)=edits{
        let rows=source["segments"].as_array_mut().ok_or("此历史版本没有分段数据，请直接编辑全文")?;
        for key in edits.keys(){if !rows.iter().any(|s|s["id"]==*key){return Err("待保存片段不存在，草稿保留".into());}}
        for s in rows.iter_mut(){if let Some(text)=edits.get(s["id"].as_str().unwrap_or("")){if s["state"]!="done"{return Err("尚未转写成功的片段不能伪装成识别成功".into());}if s["originalText"].is_null(){s["originalText"]=s["text"].clone();}s["text"]=json!(text);s["edited"]=json!(true);}}
        rows.iter().filter(|s|s["state"]=="done"&&!s["text"].as_str().unwrap_or("").trim().is_empty()).map(|s|format!("【{}–{} · {}】\n{}",clock(s["startMs"].as_u64().unwrap_or(0)),clock(s["endMs"].as_u64().unwrap_or(0)),if s["track"]=="mic"{"麦克风"}else{"系统声音"},s["text"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n\n")
    }else{text};
    audio::add_version(&app,&id,original["kind"].as_str().ok_or("历史版本类型缺失")?,saved_text,source)
}

#[link(name="winmm")]extern "system"{fn mciSendStringW(command:*const u16,result:*mut u16,len:u32,callback:*mut std::ffi::c_void)->u32;}
fn mci(command:&str)->Result<(),String>{let wide:Vec<u16>=command.encode_utf16().chain(Some(0)).collect();let code=unsafe{mciSendStringW(wide.as_ptr(),std::ptr::null_mut(),0,std::ptr::null_mut())};if code!=0{return Err(format!("Windows 音频播放失败（{code}），请检查设备与文件路径"));}Ok(())}
pub fn stop_seek_playback(){let _=mci("stop qingjian_seek");let _=mci("close qingjian_seek");}
#[tauri::command]
pub fn play_recording_at(id:String,track:String,start_ms:u64,end_ms:Option<u64>,app:tauri::AppHandle)->Result<(),String>{
    if audio::active(){return Err("录音过程中暂不重听，避免回放混入录音".into());}if !["mic","system"].contains(&track.as_str()){return Err("音轨类型无效".into());}
    let v=audio::session(&app,&id)?;let path=Path::new(v[&track].as_str().ok_or("原始音轨不可用")?);
    let r=hound::WavReader::open(path).map_err(|_|"原始音轨不存在或不可读取")?;let duration=r.duration()as u64*1000/r.spec().sample_rate as u64;
    if start_ms>=duration{return Err("播放起点超出音轨时长".into());}let end=end_ms.unwrap_or(duration).min(duration);if end<=start_ms{return Err("播放范围无效".into());}
    let path=path.to_str().ok_or("音频路径编码无效")?;if path.contains('"')||path.contains('\0'){return Err("音频路径无效".into());}
    audio::stop_playback();mci(&format!("open \"{path}\" type waveaudio alias qingjian_seek"))?;mci("set qingjian_seek time format milliseconds")?;mci(&format!("play qingjian_seek from {start_ms} to {end}"))
}
