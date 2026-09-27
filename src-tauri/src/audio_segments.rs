use crate::audio;
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::{fs,io::Read,path::{Path,PathBuf}};
use tauri::Emitter;
fn err(e:impl std::fmt::Display)->String{e.to_string()}
fn hash_file(path:&Path)->Result<String,String>{let mut f=fs::File::open(path).map_err(err)?;let mut h=Sha256::new();let mut b=[0;65536];loop{let n=f.read(&mut b).map_err(err)?;if n==0{break;}h.update(&b[..n]);}Ok(format!("{:x}",h.finalize()))}
fn label(track:&str)->&'static str{if track=="mic"{"麦克风"}else{"系统声音"}}
fn clock(ms:u64)->String{let s=ms/1000;format!("{:02}:{:02}:{:02}",s/3600,s/60%60,s%60)}
struct TempDir{path:PathBuf,parent:PathBuf}
impl Drop for TempDir{fn drop(&mut self){if let(Ok(p),Ok(root))=(self.path.canonicalize(),self.parent.canonicalize()){if p.parent()==Some(root.as_path()){let _=fs::remove_dir_all(p);}}}}

fn check_format(r:&hound::WavReader<std::io::BufReader<fs::File>>)->Result<(),String>{let s=r.spec();if s.channels!=1||s.sample_rate!=16000||s.bits_per_sample!=16||s.sample_format!=hound::SampleFormat::Int{return Err("长录音需要 16 kHz 单声道 PCM WAV，原音频未改动".into());}Ok(())}
fn boundary(samples:&[i16])->usize{
    if samples.len()<960000{return samples.len();}
    // 优先在末五秒的低音量处断句。相邻片段不重叠、不留缝，避免人为复制边界文字。
    for start in (880000..samples.len().saturating_sub(1600)).step_by(1600).rev(){let block=&samples[start..start+1600];let energy=block.iter().map(|x|(*x as f64).powi(2)).sum::<f64>()/block.len()as f64;if energy.sqrt()<164.{return start+800;}}
    samples.len()
}
fn plan(path:&Path,track:&str)->Result<Vec<Value>,String>{
    let mut reader=hound::WavReader::open(path).map_err(err)?;check_format(&reader)?;let total=reader.duration();let mut start=0u32;let mut result=Vec::new();
    while start<total{reader.seek(start).map_err(err)?;let samples=reader.samples::<i16>().take(960000).collect::<Result<Vec<_>,_>>().map_err(err)?;if samples.is_empty(){break;}let end=start+boundary(&samples)as u32;result.push(json!({"id":format!("{track}:{start}"),"track":track,"startSample":start,"endSample":end,"startMs":start as u64*1000/16000,"endMs":end as u64*1000/16000,"state":"pending","text":""}));start=end;}
    Ok(result)
}
fn write_clip(path:&Path,segment:&Value,to:&Path)->Result<(),String>{let mut r=hound::WavReader::open(path).map_err(err)?;check_format(&r)?;let start=segment["startSample"].as_u64().ok_or("片段起点缺失")?;let end=segment["endSample"].as_u64().ok_or("片段终点缺失")?;if end<=start||end-start>960000{return Err("片段范围无效".into());}r.seek(start as u32).map_err(err)?;let mut w=hound::WavWriter::create(to,r.spec()).map_err(err)?;let mut n=0;for sample in r.samples::<i16>().take((end-start)as usize){w.write_sample(sample.map_err(err)?).map_err(err)?;n+=1;}if n!=end-start{return Err("片段源文件不完整，未提交转写".into());}w.finalize().map_err(err)}

pub(crate) fn transcribe(app:&tauri::AppHandle,id:&str,kind:&str,only:Option<&str>)->Result<Value,String>{
    if audio::active(){return Err("请先停止录音".into());}
    let mut v=audio::session(app,id)?;let local=audio::audio_config(app.clone())?;
    let config=if kind=="local"{local.clone()}else{crate::online::online_config("audio".into(),app.clone())?};
    if kind=="local"&&config["available"]!=true{return Err("SenseVoice 文件不完整，音频保留".into());}
    if kind=="online"&&(config["tested"]!=true||config["hasKey"]!=true){return Err("在线语音配置尚未通过测试".into());}
    let source=if kind=="local"{json!({"kind":"local","model":"SenseVoiceSmall","engine":"sensevoice","fingerprint":audio::local_fingerprint(&config)})}else{json!({"kind":"online","model":config["model"],"endpoint":config["endpoint"],"revision":config["revision"]})};
    let mut hashes=Vec::new();for track in ["mic","system"]{hashes.push(hash_file(Path::new(v[track].as_str().ok_or("音轨缺失")?))?);}
    let fingerprint=format!("{:x}",Sha256::digest(format!("{source}|{hashes:?}|segments-v1").as_bytes()));
    let same=v["segmentJob"]["fingerprint"]==fingerprint;
    if only.is_some()&&!same{return Err("录音或模型配置已变化，请选择重新转写".into());}
    let mut segments=if same{v["segmentJob"]["segments"].as_array().cloned().ok_or("片段进度损坏")?}else{
        let mut rows=Vec::new();for track in ["mic","system"]{rows.extend(plan(Path::new(v[track].as_str().unwrap()),track)?);}rows.sort_by_key(|s|(s["startMs"].as_u64().unwrap_or(0),s["track"].as_str().unwrap_or("").to_owned()));rows
    };
    if segments.is_empty(){return Err("没有可转写的采样数据，音频保留".into());}
    if only.is_some_and(|key|!segments.iter().any(|s|s["id"]==key)){return Err("片段不存在".into());}
    let parent=PathBuf::from(local["root"].as_str().ok_or("缺少非 C 盘工作目录")?).join("jobs");if !parent.is_absolute()||parent.to_string_lossy().to_ascii_lowercase().starts_with("c:"){return Err("转写工作目录必须位于非 C 盘".into());}
    let temp=TempDir{path:parent.join(uuid::Uuid::new_v4().to_string()),parent};fs::create_dir_all(&temp.path).map_err(err)?;
    let mut inputs=Vec::new();for(i,s)in segments.iter().enumerate(){if s["state"]=="done"||only.is_some_and(|key|s["id"]!=key){continue;}let path=temp.path.join(format!("{i}.wav"));write_clip(Path::new(v[s["track"].as_str().unwrap()].as_str().unwrap()),s,&path)?;inputs.push((s["id"].as_str().unwrap().to_owned(),path));}
    if inputs.is_empty(){return Ok(v);}
    v["segmentJob"]=json!({"fingerprint":fingerprint,"source":source,"segments":segments,"state":"processing"});audio::put_session(app,&v)?;
    let mut complete=|key:&str,result:Result<String,String>|->Result<(),String>{
        let s=segments.iter_mut().find(|s|s["id"]==key).ok_or("片段标识变化")?;
        match result{Ok(text)=>{s["state"]=json!("done");s["text"]=json!(text);s["error"]=Value::Null;},Err(e)=>{s["state"]=json!("failed");s["error"]=json!(e);}}
        s["source"]=source.clone();let mut latest=audio::session(app,id)?;latest["segmentJob"]["segments"]=json!(segments);audio::put_session(app,&latest)?;
        let done=segments.iter().filter(|s|s["state"]=="done").count();let _=app.emit("recording-stage",json!({"id":id,"message":format!("转写 {done}/{} 段",segments.len())}));Ok(())
    };
    let result=if kind=="local"{crate::sensevoice::run_batch(&config,&inputs,&mut complete)}else{
        let mut result=Ok(());for(key,path)in &inputs{let response=crate::online::audio_chunk(&config,path);if let Err(e)=complete(key,response){result=Err(e);break;}}result
    };
    drop(complete);
    let missing=segments.iter().filter(|s|s["state"]!="done").map(|s|json!({"id":s["id"],"track":s["track"],"startMs":s["startMs"],"endMs":s["endMs"],"error":s["error"]})).collect::<Vec<_>>();
    let mut latest=audio::session(app,id)?;latest["segmentJob"]["state"]=json!(if missing.is_empty(){"complete"}else{"partial"});latest["error"]=json!(result.err().unwrap_or_else(||if missing.is_empty(){String::new()}else{format!("{} 个片段尚未完成，可单独重试；音频保留",missing.len())}));audio::put_session(app,&latest)?;
    let text=segments.iter().filter(|s|s["state"]=="done"&&!s["text"].as_str().unwrap_or("").trim().is_empty()).map(|s|format!("【{}–{} · {}】\n{}",clock(s["startMs"].as_u64().unwrap_or(0)),clock(s["endMs"].as_u64().unwrap_or(0)),label(s["track"].as_str().unwrap_or("")),s["text"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n\n");
    let mut provenance=source;provenance["segments"]=json!(segments);provenance["missingRanges"]=json!(missing);provenance["inputFingerprint"]=json!(fingerprint);
    // 即使部分失败，成功片段也作为独立版本保留，绝不覆盖此前完整版本。
    let mut saved=audio::add_version(app,id,kind,text,provenance)?;saved["error"]=latest["error"].clone();audio::put_session(app,&saved)?;Ok(saved)
}

#[tauri::command]pub fn get_recording(id:String,app:tauri::AppHandle)->Result<Value,String>{audio::session(&app,&id).map(crate::audio_workspace::with_metadata)}
#[tauri::command]pub async fn retry_audio_segment(id:String,segment_id:String,app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{let _guard=audio::ProcessingGuard::acquire()?;let v=audio::session(&app,&id)?;let kind=v["segmentJob"]["source"]["kind"].as_str().ok_or("没有可重试的片段任务")?;transcribe(&app,&id,kind,Some(&segment_id))}).await.map_err(err)?}
#[tauri::command]pub fn reset_transcription_job(id:String,app:tauri::AppHandle)->Result<(),String>{let _guard=audio::ProcessingGuard::acquire()?;if audio::active(){return Err("请先停止录音".into());}let mut v=audio::session(&app,&id)?;v.as_object_mut().ok_or("录音记录损坏")?.remove("segmentJob");audio::put_session(&app,&v)}

#[cfg(test)]mod tests{use super::*;
#[test]fn segmentation_preserves_every_sample_once(){let mut samples=vec![1000i16;960000];samples[900000..].fill(0);let n=boundary(&samples);assert!((880000..=960000).contains(&n));assert_eq!(boundary(&[1,2,3]),3);}
}
