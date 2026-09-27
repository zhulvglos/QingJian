use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}, os::windows::process::CommandExt, time::{Duration, Instant}};
use tauri::Emitter;

const FILES: &[(&str, &[&str])] = &[
    ("SenseVoiceSmall", &["model.pt", "config.yaml", "configuration.json", "am.mvn", "chn_jpn_yue_eng_ko_spectok.bpe.model"]),
    ("fsmn-vad", &["model.pt", "config.yaml", "configuration.json", "am.mvn"]),
];
pub fn defaults(config: &mut Value) {
    // 旧配置只迁移运行入口，不改写任何已有录音或转写结果。
    config["engine"] = json!("sensevoice");
    if let Some(o) = config.as_object_mut() { o.remove("exe"); o.remove("model"); }

    let root=PathBuf::from(config["root"].as_str().unwrap_or(""));
    for (key, relative) in [("sensePython","sensevoice/python/python.exe"),("senseRuntime","sensevoice/runtime"),("senseModels","sensevoice/models")] {
        if config.get(key).is_none() { config[key]=json!(if root.as_os_str().is_empty(){String::new()}else{root.join(relative).to_string_lossy().to_string()}); }
    }
}
pub fn files(config: &Value) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(config["sensePython"].as_str().unwrap_or(""))];
    let packages = PathBuf::from(config["senseRuntime"].as_str().unwrap_or("")).join("Lib/site-packages");
    for name in ["torch/__init__.py", "torchaudio/__init__.py", "funasr/__init__.py", "sentencepiece/__init__.py", "opencc/__init__.py"] { paths.push(packages.join(name)); }
    let root = PathBuf::from(config["senseModels"].as_str().unwrap_or(""));
    for (model, names) in FILES { for name in *names { paths.push(root.join(model).join(name)); } }
    paths
}
pub fn available(config: &Value) -> bool { files(config).iter().all(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0)) }

// 每次只创建自己的 UUID 任务目录；退出时确认规范化路径仍位于指定 jobs 下再清理。
struct Job { path: PathBuf, parent: PathBuf }
#[repr(C)]#[derive(Default)]struct BasicLimits{process_time:i64,job_time:i64,flags:u32,min_working:usize,max_working:usize,processes:u32,affinity:usize,priority:u32,scheduling:u32}
#[repr(C)]#[derive(Default)]struct ExtendedLimits{basic:BasicLimits,io:[u64;6],process_memory:usize,job_memory:usize,peak_process:usize,peak_job:usize}
#[link(name="kernel32")]extern "system"{fn CreateJobObjectW(attributes:*mut std::ffi::c_void,name:*const u16)->*mut std::ffi::c_void;fn SetInformationJobObject(job:*mut std::ffi::c_void,class:i32,info:*const std::ffi::c_void,size:u32)->i32;fn AssignProcessToJobObject(job:*mut std::ffi::c_void,process:*mut std::ffi::c_void)->i32;fn CloseHandle(handle:isize)->i32;}
struct ChildGroup(*mut std::ffi::c_void);
impl ChildGroup{fn attach(child:&mut std::process::Child)->Result<Self,String>{use std::os::windows::io::AsRawHandle;let job=unsafe{CreateJobObjectW(std::ptr::null_mut(),std::ptr::null())};if job.is_null(){let _=child.kill();return Err("无法建立本地转写进程保护".into());}let group=Self(job);let mut limits=ExtendedLimits::default();limits.basic.flags=0x2000;if unsafe{SetInformationJobObject(job,9,&limits as *const _ as _,std::mem::size_of::<ExtendedLimits>()as u32)}==0||unsafe{AssignProcessToJobObject(job,child.as_raw_handle())}==0{let _=child.kill();return Err("无法保护本地转写子进程，任务未继续".into());}Ok(group)}}
impl Drop for ChildGroup{fn drop(&mut self){unsafe{CloseHandle(self.0 as isize);}}}
impl Drop for Job {
    fn drop(&mut self) {
        if let (Ok(path), Ok(parent)) = (self.path.canonicalize(), self.parent.canonicalize()) {
            if path.parent() == Some(parent.as_path()) { let _ = fs::remove_dir_all(path); }
        }
    }
}
pub fn run(config: &Value, input: &Path, check: bool, app: Option<&tauri::AppHandle>) -> Result<String, String> {
    if let Some(missing) = files(config).iter().find(|p| !p.is_file()) { return Err(format!("SenseVoice 文件缺失：{}", missing.display())); }
    let parent = PathBuf::from(config["root"].as_str().ok_or("缺少音频目录")?).join("jobs");
    if !parent.is_absolute() || parent.to_string_lossy().to_ascii_lowercase().starts_with("c:") { return Err("任务目录必须位于非 C 盘".into()); }
    let job = Job { path: parent.join(uuid::Uuid::new_v4().to_string()), parent };
    fs::create_dir_all(&job.path).map_err(|e| e.to_string())?;
    let source = Path::new(config["senseModels"].as_str().ok_or("缺少模型目录")?);
    for (model, names) in FILES {
        let dest = job.path.join("models").join(model);
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        for name in *names {
            let from = source.join(model).join(name); let to = dest.join(name);
            // 大权重仅读取，优先硬链接避免重复占用空间；配置复制到独立目录。
            if *name != "model.pt" || fs::hard_link(&from, &to).is_err() { fs::copy(&from, &to).map_err(|e| e.to_string())?; }
        }
    }
    fs::copy(input, job.path.join("input.wav")).map_err(|e| e.to_string())?;
    fs::write(job.path.join("worker.py"), include_bytes!("../resources/sensevoice_worker.py")).map_err(|e| e.to_string())?;
    fs::write(job.path.join("request.json"), json!({"runtime":config["senseRuntime"],"check":check}).to_string()).map_err(|e| e.to_string())?;
    let mut command = Command::new(config["sensePython"].as_str().ok_or("缺少运行程序")?);
    crate::sensevoice_env::isolate(&mut command,&job.path,&job.path)?;
    let mut child = command.current_dir(&job.path).args(["-B", "worker.py"])
        .env("TEMP", &job.path).env("TMP", &job.path).env("PYTHONDONTWRITEBYTECODE", "1")
        .stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x08000000)
        .spawn().map_err(|e| format!("SenseVoice 无法启动：{e}；原音频保留"))?;
    let _group=ChildGroup::attach(&mut child)?;
    let started = Instant::now(); let mut integrity_reported = false;
    loop {
        if check && !integrity_reported && job.path.join("integrity.ok").is_file() {
            integrity_reported = true;
            if let Some(app) = app {
                let _ = crate::audio::save_setting(app, "audio_validation", &json!({"fingerprint":crate::audio::local_fingerprint(config),"integrity":true,"transcribed":false}));
                let _ = app.emit("local-model-stage", "完整性检查通过，正在转写样例…");
            }
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() { return Err(format!("SenseVoice 运行失败（退出码 {:?}）；原音频保留", status.code())); }
            break;
        }
        if started.elapsed() > Duration::from_secs(1800) {
            let _ = child.kill(); let _ = child.wait(); return Err("SenseVoice 转写超过30分钟；原音频保留".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let raw = fs::read(job.path.join("response.json")).map_err(|e| format!("SenseVoice 未返回结果：{e}"))?;
    log_origin(config,&job.path)?;
    let result: Value = serde_json::from_slice(&raw).map_err(|e| format!("SenseVoice 返回格式错误：{e}"))?;
    if result["ok"] != true { return Err(format!("SenseVoice：{}；原音频保留", result["error"].as_str().unwrap_or("未知错误"))); }
    Ok(result["text"].as_str().ok_or("SenseVoice 未返回文字")?.to_string())
}

// 长录音每次加载一次模型。每片完成即回调持久化，失败重试不重新处理成功片段。
pub(crate) fn run_batch(config:&Value,inputs:&[(String,PathBuf)],mut completed:impl FnMut(&str,Result<String,String>)->Result<(),String>)->Result<(),String>{
    if !available(config){return Err("SenseVoice 文件不完整，原音频保留".into());}
    let parent=PathBuf::from(config["root"].as_str().ok_or("缺少工作目录")?).join("jobs");
    if !parent.is_absolute()||parent.to_string_lossy().to_ascii_lowercase().starts_with("c:"){return Err("转写任务目录必须位于非 C 盘".into());}
    let job=Job{path:parent.join(uuid::Uuid::new_v4().to_string()),parent};fs::create_dir_all(&job.path).map_err(|e|e.to_string())?;
    let source=Path::new(config["senseModels"].as_str().ok_or("模型目录缺失")?);
    for(model,names)in FILES{let dest=job.path.join("models").join(model);fs::create_dir_all(&dest).map_err(|e|e.to_string())?;for name in *names{let from=source.join(model).join(name);let to=dest.join(name);if *name!="model.pt"||fs::hard_link(&from,&to).is_err(){fs::copy(from,to).map_err(|e|e.to_string())?;}}}
    let mut segments=Vec::new();for (i,(id,path))in inputs.iter().enumerate(){let name=format!("segment-{i}.wav");let to=job.path.join(&name);if fs::hard_link(path,&to).is_err(){fs::copy(path,to).map_err(|e|e.to_string())?;}segments.push(json!({"id":id,"path":name}));}
    fs::write(job.path.join("worker.py"),include_bytes!("../resources/sensevoice_worker.py")).map_err(|e|e.to_string())?;
    fs::write(job.path.join("batch.py"),include_bytes!("../resources/sensevoice_batch.py")).map_err(|e|e.to_string())?;
    fs::write(job.path.join("request.json"),json!({"runtime":config["senseRuntime"],"check":false,"segments":segments}).to_string()).map_err(|e|e.to_string())?;
    let mut command=Command::new(config["sensePython"].as_str().ok_or("运行环境缺失")?);crate::sensevoice_env::isolate(&mut command,&job.path,&job.path)?;
    let mut child=command.current_dir(&job.path).args(["-B","batch.py"]).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x08000000).spawn().map_err(|e|format!("SenseVoice 无法启动：{e}"))?;
    let _group=ChildGroup::attach(&mut child)?;let mut next=0usize;let mut last_progress=Instant::now();
    loop{
        while next<inputs.len(){let path=job.path.join("results").join(format!("{next}.json"));if !path.is_file(){break;}let value:Value=serde_json::from_slice(&fs::read(&path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let result=if value["ok"]==true{value["text"].as_str().map(str::to_owned).ok_or("片段结果缺少文字".into())}else{Err(format!("SenseVoice：{}",value["error"].as_str().unwrap_or("片段失败")))};
            completed(&inputs[next].0,result)?;next+=1;last_progress=Instant::now();
        }
        if let Some(status)=child.try_wait().map_err(|e|e.to_string())?{
            // 子进程结束时再收取一次已原子提交的末片结果。
            if next<inputs.len()&&job.path.join("results").join(format!("{next}.json")).is_file(){continue;}
            log_origin(config,&job.path)?;
            let response:Value=fs::read(job.path.join("response.json")).ok().and_then(|b|serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
            if !status.success()||response["ok"]!=true||next!=inputs.len(){return Err(format!("本地分段任务未全部完成：{}；音频及成功片段保留",response["error"].as_str().unwrap_or("子进程异常退出")));}return Ok(());
        }
        if last_progress.elapsed()>Duration::from_secs(600){let _=child.kill();let _=child.wait();return Err("单个片段处理超过十分钟；音频和成功片段保留".into());}
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn log_origin(config:&Value,job:&Path)->Result<(),String>{
    let runtime:Value=fs::read(job.join("runtime-origin.json")).ok().and_then(|b|serde_json::from_slice(&b).ok()).unwrap_or(json!({"status":"未产生运行时来源记录"}));
    crate::sensevoice_env::audit(Path::new(config["root"].as_str().ok_or("缺少工作目录")?),"transcription-resources",json!({"configuredPython":config["sensePython"],"configuredRuntime":config["senseRuntime"],"configuredModels":config["senseModels"],"actual":runtime}))
}

#[cfg(test)] mod tests {
    #[test] fn legacy_engine_migrates_without_losing_paths() {
        let mut c=serde_json::json!({"engine":"whisper","exe":"F:\\old.exe","model":"F:\\old.bin","root":"F:\\recordings","senseModels":"F:\\custom"});
        super::defaults(&mut c);
        assert_eq!(c["engine"],"sensevoice"); assert_eq!(c["senseModels"],"F:\\custom");
        assert_eq!(c["root"],"F:\\recordings"); assert!(c.get("exe").is_none()); assert!(c.get("model").is_none());
    }
}
