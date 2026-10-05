use crate::audio::{self, ProcessingGuard};
use serde_json::{json, Value};
use std::{collections::VecDeque, fs, io::{Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
use tauri::Manager;
use wasapi::*;

fn err(e:impl std::fmt::Display)->String{e.to_string()}
#[derive(Default)]
struct Runtime {
    stop:Option<Arc<AtomicBool>>, pause:Option<Arc<AtomicBool>>, handles:Vec<thread::JoinHandle<()>>,
    id:String, started:Option<Instant>, started_qpc:u64, pauses:Vec<(u64,Option<u64>)>, paused_at:Option<Instant>, paused_total:Duration, final_seconds:Option<f64>, tracks:Value,
}
static RUNTIME:OnceLock<Mutex<Runtime>>=OnceLock::new();
fn runtime()->&'static Mutex<Runtime>{RUNTIME.get_or_init(||Mutex::new(Runtime::default()))}
fn elapsed(r:&Runtime)->f64{r.final_seconds.unwrap_or_else(||r.started.map(|t|t.elapsed().saturating_sub(r.paused_total).saturating_sub(r.paused_at.map(|t|t.elapsed()).unwrap_or_default()).as_secs_f64()).unwrap_or(0.))}
pub fn active()->bool{runtime().lock().map(|r|r.stop.is_some()).unwrap_or(true)}
fn track_update(key:&str,v:Value){if let Ok(mut r)=runtime().lock(){r.tracks[key]=v;}}
fn stop_on_error(key:&str,error:String,flag:&AtomicBool){if let Ok(mut r)=runtime().lock(){r.final_seconds=Some(elapsed(&r));r.tracks[key]=json!({"state":"error","error":error});}flag.store(true,Ordering::Relaxed);}

#[link(name="kernel32")]
extern "system"{fn GetDiskFreeSpaceExW(path:*const u16,available:*mut u64,total:*mut u64,free:*mut u64)->i32;fn QueryPerformanceCounter(value:*mut i64)->i32;fn QueryPerformanceFrequency(value:*mut i64)->i32;}
fn qpc()->u64{let(mut value,mut frequency)=(0i64,0i64);unsafe{QueryPerformanceCounter(&mut value);QueryPerformanceFrequency(&mut frequency);}if frequency<=0{return 0;}(value as u128*10_000_000/frequency as u128)as u64}
fn sample_position(timestamp:u64)->u64{let r=runtime().lock().unwrap();let mut ticks=timestamp.saturating_sub(r.started_qpc);for(start,end)in &r.pauses{if timestamp>*start{ticks=ticks.saturating_sub(end.unwrap_or(timestamp).min(timestamp)-start);}}ticks.saturating_mul(16000)/10_000_000}
struct PcmFile{file:fs::File,samples:u64}
impl PcmFile{
    fn new(path:&Path)->Result<Self,String>{let mut file=fs::File::create(path).map_err(err)?;let mut h=Vec::new();h.extend(b"RIFF");h.extend(36u32.to_le_bytes());h.extend(b"WAVEfmt ");h.extend(16u32.to_le_bytes());h.extend(1u16.to_le_bytes());h.extend(1u16.to_le_bytes());h.extend(16000u32.to_le_bytes());h.extend(32000u32.to_le_bytes());h.extend(2u16.to_le_bytes());h.extend(16u16.to_le_bytes());h.extend(b"data");h.extend(0u32.to_le_bytes());file.write_all(&h).map_err(err)?;Ok(Self{file,samples:0})}
    fn extend(&mut self,samples:u64)->Result<(),String>{if samples>2_000_000_000{return Err("录音已接近 WAV 容量上限，已停止并保留".into());}if samples>self.samples{self.file.set_len(44+samples*2).map_err(err)?;self.samples=samples;}Ok(())}
    fn packet(&mut self,position:u64,data:&[u8])->Result<(),String>{self.extend(position+data.len()as u64/2)?;self.file.seek(SeekFrom::Start(44+position*2)).map_err(err)?;self.file.write_all(data).map_err(err)}
    fn flush(&mut self,durable:bool)->Result<(),String>{self.file.seek(SeekFrom::Start(4)).map_err(err)?;self.file.write_all(&(36+self.samples as u32*2).to_le_bytes()).map_err(err)?;self.file.seek(SeekFrom::Start(40)).map_err(err)?;self.file.write_all(&(self.samples as u32*2).to_le_bytes()).map_err(err)?;self.file.flush().map_err(err)?;if durable{self.file.sync_all().map_err(err)?;}Ok(())}
}
fn space(path:&Path)->Result<u64,String>{use std::os::windows::ffi::OsStrExt;let p:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();let(mut available,mut total,mut free)=(0,0,0);if unsafe{GetDiskFreeSpaceExW(p.as_ptr(),&mut available,&mut total,&mut free)}==0{return Err("无法读取录音目录可用空间".into());}Ok(available)}

fn capture(path:PathBuf,render:bool,device_id:String,stop:Arc<AtomicBool>,pause:Arc<AtomicBool>)->Result<(),String>{
    initialize_mta().ok().map_err(err)?;
    let en=DeviceEnumerator::new().map_err(err)?;
    let d=if device_id.is_empty(){en.get_default_device(&if render{Direction::Render}else{Direction::Capture})}else{en.get_device(&device_id)}.map_err(err)?;
    let name=d.get_friendlyname().map_err(err)?;let mut client=d.get_iaudioclient().map_err(err)?;
    let format=WaveFormat::new(16,16,&SampleType::Int,16000,1,None);
    client.initialize_client(&format,&Direction::Capture,&StreamMode::EventsShared{autoconvert:true,buffer_duration_hns:200_000}).map_err(err)?;
    let event=client.set_get_eventhandle().map_err(err)?;let source=client.get_audiocaptureclient().map_err(err)?;
    let mut writer=PcmFile::new(&path)?;
    let key=if render{"system"}else{"mic"};let mut bytes=VecDeque::new();let mut flushed=Instant::now();let mut synced=Instant::now();let mut peak=0f32;let mut received=0u64;let mut discontinuities=0u64;
    client.start_stream().map_err(err)?;
    let result=(||->Result<(),String>{
        while !stop.load(Ordering::Relaxed){
            let _=event.wait_for_event(100);
            // 暂停仍排空设备缓冲，避免继续时把暂停期间的声音补写进文件。
            while source.get_next_packet_size().map_err(err)?.unwrap_or(0)>0 {
                let info=source.read_from_device_to_deque(&mut bytes).map_err(err)?;
                if info.flags.data_discontinuity&&received>0&&!pause.load(Ordering::Relaxed){discontinuities+=1;}
                if pause.load(Ordering::Relaxed){bytes.clear();continue;}
                let mut packet=bytes.drain(..).collect::<Vec<_>>();if info.flags.silent{packet.fill(0);}for pair in packet.chunks_exact(2){peak=peak.max((i16::from_le_bytes([pair[0],pair[1]])as f32/32768.).abs());}
                let position=if info.flags.timestamp_error||info.timestamp==0{sample_position(qpc()).saturating_sub(packet.len()as u64/2)}else{sample_position(info.timestamp)};
                // 两轨共用设备 QPC 时间轴；无播放声音的区间补零，迟到数据可回填，不能压缩掉静音时间。
                writer.packet(position,&packet)?;received+=packet.len()as u64/2;
            }
            if flushed.elapsed()>=Duration::from_millis(250){
                writer.extend(sample_position(qpc()).saturating_sub(3200))?;writer.flush(false)?;
                track_update(key,json!({"state":if pause.load(Ordering::Relaxed){"paused"}else{"recording"},"device":name,"peak":peak,"samples":writer.samples,"receivedSamples":received,"seconds":writer.samples as f64/16000.,"discontinuities":discontinuities}));
                peak=0.;flushed=Instant::now();
            }
            if synced.elapsed()>=Duration::from_secs(5){
                // 定期向磁盘提交，而非把整段录音留在内存；预留空间用于完成 WAV 头和进度记录。
                writer.flush(true)?;
                if space(path.parent().ok_or("录音目录无效")?)?<64*1024*1024{return Err("磁盘剩余空间不足，已停止采集并保留已写音频".into());}
                synced=Instant::now();
            }
        }Ok(())
    })();
    let _=client.stop_stream();let end={let r=runtime().lock().map_err(err)?;(elapsed(&r)*16000.)as u64};let finished=writer.extend(end).and_then(|_|writer.flush(true));
    result.and(finished)
}

fn checkpoint(app:&tauri::AppHandle,id:&str)->Result<(),String>{
    let (seconds,paused,tracks)={let r=runtime().lock().map_err(err)?;(elapsed(&r),r.paused_at.is_some(),r.tracks.clone())};
    let mut v=audio::session(app,id)?;v["seconds"]=json!(seconds);v["paused"]=json!(paused);v["tracks"]=tracks;v["checkpointAt"]=json!(chrono::Utc::now().to_rfc3339());
    let dir=Path::new(v["temporaryDir"].as_str().ok_or("缺少临时录音目录")?);let temp=dir.join("progress.json.tmp");let mut f=fs::File::create(&temp).map_err(err)?;f.write_all(v.to_string().as_bytes()).map_err(err)?;f.sync_all().map_err(err)?;drop(f);fs::rename(temp,dir.join("progress.json")).map_err(err)?;
    audio::put_session(app,&v)
}

#[tauri::command]
pub fn start_recording(mic_device:Option<String>,system_device:Option<String>,mode:Option<String>,online_revision:Option<String>,fallback_revision:Option<String>,app:tauri::AppHandle)->Result<Value,String>{
    let _guard=ProcessingGuard::acquire()?;audio::stop_playback();
    let config=audio::audio_config(app.clone())?;let root=PathBuf::from(config["root"].as_str().unwrap_or(""));
    if !root.is_absolute()||root.to_string_lossy().to_ascii_lowercase().starts_with("c:"){return Err("请在设置 → 语音选择非 C 盘录音目录".into());}
    fs::create_dir_all(&root).map_err(err)?;if space(&root)?<128*1024*1024{return Err("录音目录可用空间不足 128 MB".into());}
    let mut r=runtime().lock().map_err(err)?;if r.stop.is_some(){return Err("已有录音，请先结束当前任务".into());}
    let id=uuid::Uuid::new_v4().to_string();let dir=root.join("temporary-recordings").join(&id);fs::create_dir_all(&dir).map_err(err)?;
    let v=json!({"id":id,"createdAt":chrono::Utc::now().to_rfc3339(),"state":"recording","mic":dir.join("mic.wav"),"system":dir.join("system.wav"),"versions":[],"error":"","storage":"temporary","temporaryDir":dir,"mode":mode.unwrap_or("local".into()),"onlineRevision":online_revision,"fallbackRevision":fallback_revision,"captureVersion":2});audio::put_session(&app,&v)?;
    let stop=Arc::new(AtomicBool::new(false));let pause=Arc::new(AtomicBool::new(false));
    *r=Runtime{stop:Some(stop.clone()),pause:Some(pause.clone()),started:Some(Instant::now()),started_qpc:qpc(),id:id.clone(),tracks:json!({"mic":{"state":"starting"},"system":{"state":"starting"}}),..Runtime::default()};
    for(key,render)in [("mic",false),("system",true)]{
        let path=PathBuf::from(v[key].as_str().unwrap());let flag=stop.clone();let paused=pause.clone();let device=if render{system_device.clone()}else{mic_device.clone()}.unwrap_or_default();
        r.handles.push(thread::spawn(move||{if let Err(e)=capture(path,render,device,flag.clone(),paused){stop_on_error(key,e,&flag);}}));
    }
    let flag=stop.clone();let checkpoint_app=app.clone();
    r.handles.push(thread::spawn(move||{let mut last=Instant::now();while !flag.load(Ordering::Relaxed){thread::sleep(Duration::from_millis(200));if last.elapsed()>=Duration::from_secs(5){if let Err(e)=checkpoint(&checkpoint_app,&id){stop_on_error("storage",e,&flag);}last=Instant::now();}}}));
    Ok(v)
}
#[tauri::command]
pub fn pause_recording(paused:bool)->Result<(),String>{let mut r=runtime().lock().map_err(err)?;if r.stop.as_ref().is_none_or(|f|f.load(Ordering::Relaxed)){return Err("没有正在采集的录音".into());}if paused&&r.paused_at.is_none(){r.paused_at=Some(Instant::now());r.pauses.push((qpc(),None));}else if !paused{if let Some(t)=r.paused_at.take(){r.paused_total+=t.elapsed();if let Some(p)=r.pauses.last_mut(){p.1=Some(qpc());}}}r.pause.as_ref().unwrap().store(paused,Ordering::Relaxed);Ok(())}
#[tauri::command]
pub fn recording_status()->Result<Value,String>{let r=runtime().lock().map_err(err)?;Ok(json!({"active":r.stop.is_some(),"captureStopped":r.stop.as_ref().is_some_and(|s|s.load(Ordering::Relaxed)),"paused":r.paused_at.is_some(),"id":r.id,"seconds":elapsed(&r),"tracks":r.tracks}))}
#[tauri::command]
pub async fn stop_recording(app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{
    let _guard=ProcessingGuard::acquire()?;let(id,handles,seconds)={let mut r=runtime().lock().map_err(err)?;r.stop.as_ref().ok_or("当前没有录音")?.store(true,Ordering::Relaxed);let seconds=elapsed(&r);r.final_seconds=Some(seconds);(r.id.clone(),std::mem::take(&mut r.handles),seconds)};
    for h in handles{let _=h.join();}let mut v=audio::session(&app,&id)?;let mut r=runtime().lock().map_err(err)?;
    let errors=r.tracks.as_object().map(|m|m.values().filter_map(|t|t["error"].as_str()).collect::<Vec<_>>().join("；")).unwrap_or_default();
    v["state"]=json!(if errors.is_empty(){"stopped"}else{"partial"});v["error"]=json!(errors);v["seconds"]=json!(seconds);v["tracks"]=r.tracks.clone();v["paused"]=json!(false);v["stoppedAt"]=json!(chrono::Utc::now().to_rfc3339());audio::put_session(&app,&v)?;*r=Runtime::default();Ok(v)
}).await.map_err(err)?}

// 仅修复本程序创建、异常中断的临时 PCM WAV 头；不改动旧版文件或用户导出的录音。
fn recover_wav(path:&Path)->Result<(),String>{
    let mut f=fs::OpenOptions::new().read(true).write(true).open(path).map_err(err)?;let len=f.metadata().map_err(err)?.len();let mut header=[0u8;12];f.read_exact(&mut header).map_err(err)?;
    if &header[..4]!=b"RIFF"||&header[8..]!=b"WAVE"{return Err("音轨不是可恢复的 WAV".into());}
    let mut offset=12u64;
    while offset+8<=len{f.seek(SeekFrom::Start(offset)).map_err(err)?;let mut chunk=[0u8;8];f.read_exact(&mut chunk).map_err(err)?;let n=u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as u64;
        if &chunk[..4]==b"data"{let count=(len-offset-8)/2*2;if count>u32::MAX as u64{return Err("音轨超出 WAV 恢复范围".into());}f.seek(SeekFrom::Start(offset+4)).map_err(err)?;f.write_all(&(count as u32).to_le_bytes()).map_err(err)?;f.seek(SeekFrom::Start(4)).map_err(err)?;f.write_all(&((len-8) as u32).to_le_bytes()).map_err(err)?;f.sync_all().map_err(err)?;return Ok(());}
        offset+=8+n+(n%2);
    }Err("音轨缺少 PCM 数据段".into())
}
pub fn recover_sessions(app:&tauri::AppHandle)->Result<(),String>{
    let rows={let db=app.state::<crate::database::Database>();let c=db.0.lock().map_err(err)?;let mut s=c.prepare("SELECT payload FROM audio_sessions").map_err(err)?;let rows=s.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;rows};
    for raw in rows{let mut v:Value=serde_json::from_str(&raw).map_err(err)?;if v["state"]!="recording"{continue;}
        let mut errors=Vec::new();if v["storage"]=="temporary"&&v["captureVersion"]==2{if let Some(dir)=v["temporaryDir"].as_str().and_then(|p|Path::new(p).canonicalize().ok()){
            if dir.file_name().and_then(|n|n.to_str())==v["id"].as_str()&&dir.parent().and_then(|p|p.file_name()).and_then(|n|n.to_str())==Some("temporary-recordings"){
                for key in ["mic","system"]{if let Some(p)=v[key].as_str().and_then(|p|Path::new(p).canonicalize().ok()){if p.parent()==Some(dir.as_path())&&p.file_name().and_then(|n|n.to_str())==Some(&format!("{key}.wav")){if let Err(e)=recover_wav(&p){errors.push(format!("{key}：{e}"));}}}}
            }
        }}
        let seconds=["mic","system"].iter().filter_map(|k|v[k].as_str()).filter_map(|p|hound::WavReader::open(p).ok()).map(|r|r.duration() as f64/r.spec().sample_rate as f64).fold(0f64,f64::max);
        v["seconds"]=json!(seconds);v["state"]=json!("interrupted");v["error"]=json!(format!("上次录音异常中断，已保留可恢复音频。{}",errors.join("；")));v["recoveredAt"]=json!(chrono::Utc::now().to_rfc3339());audio::put_session(app,&v)?;
    }Ok(())
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn timestamped_pcm_keeps_silence_and_accepts_late_packets(){let dir=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir(&dir).unwrap();let path=dir.join("timeline.wav");let mut w=PcmFile::new(&path).unwrap();w.packet(100,&11i16.to_le_bytes()).unwrap();w.extend(200).unwrap();w.packet(0,&99i16.to_le_bytes()).unwrap();w.flush(true).unwrap();drop(w);let samples=hound::WavReader::open(&path).unwrap().samples::<i16>().collect::<Result<Vec<_>,_>>().unwrap();assert_eq!(samples.len(),200);assert_eq!(samples[0],99);assert_eq!(samples[100],11);assert_eq!(samples[199],0);fs::remove_file(path).unwrap();fs::remove_dir(dir).unwrap();}
    #[test]fn interrupted_wav_header_recovers_existing_samples(){
        let dir=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir(&dir).unwrap();let path=dir.join("mic.wav");
        let mut w=hound::WavWriter::create(&path,hound::WavSpec{channels:1,sample_rate:16000,bits_per_sample:16,sample_format:hound::SampleFormat::Int}).unwrap();for n in 0..160{w.write_sample(n as i16).unwrap();}w.finalize().unwrap();
        let before=fs::read(&path).unwrap();let mut f=fs::OpenOptions::new().write(true).open(&path).unwrap();f.seek(SeekFrom::Start(40)).unwrap();f.write_all(&0u32.to_le_bytes()).unwrap();drop(f);recover_wav(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(),before);assert_eq!(hound::WavReader::open(&path).unwrap().duration(),160);fs::remove_file(path).unwrap();fs::remove_dir(dir).unwrap();
    }
}

#[tauri::command]
pub fn get_recording_inputs(app:tauri::AppHandle)->Result<Value,String>{crate::audio::setting(&app,"recording_inputs")}
#[tauri::command]
pub fn save_recording_inputs(app:tauri::AppHandle,mic_device:String,system_device:String,mode:String)->Result<(),String>{
 if audio::active()||audio::processing(){return Err("录音或处理过程中不能修改录音配置".into());}
 if mic_device.len()>2048||system_device.len()>2048||!["local","online"].contains(&mode.as_str()){return Err("录音配置无效".into());}
 crate::audio::save_setting(&app,"recording_inputs",&json!({"micDevice":mic_device,"systemDevice":system_device,"mode":mode}))
}
