use std::{fs,path::{Path,PathBuf},io::{Read,Write,BufRead,BufReader},process::{Command,Stdio},os::windows::process::CommandExt,time::{Duration,Instant}};
use serde_json::{json,Value};
use tauri::Emitter;
use sha2::{Sha256,Digest};
use crate::audio;
fn err(e:impl std::fmt::Display)->String{e.to_string()}
fn stage(app:&tauri::AppHandle,s:&str){let _=app.emit("local-model-stage",s);}
fn non_c(p:&Path)->Result<(),String>{if !p.is_absolute()||p.to_string_lossy().to_lowercase().starts_with("c:"){Err("请选择非 C 盘安装目录".into())}else{Ok(())}}
fn download(app:&tauri::AppHandle,url:&str,path:&Path,expected:Option<&str>)->Result<(),String>{
    non_c(path)?;if path.is_file(){if let Some(expected)=expected{if digest(path)?==expected{return Ok(())}}}
    let mut r=reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(25)).timeout(Duration::from_secs(1800)).build().map_err(err)?.get(url).send().map_err(|e|format!("下载连接失败：{e}"))?.error_for_status().map_err(err)?;
    let total=r.content_length();let temp=path.with_extension("download");let mut f=fs::File::create(&temp).map_err(err)?;let mut b=[0;65536];let mut count=0u64;let mut last=Instant::now();loop{let n=r.read(&mut b).map_err(err)?;if n==0{break;}f.write_all(&b[..n]).map_err(err)?;count+=n as u64;if last.elapsed()>Duration::from_millis(400){stage(app,&format!("下载 {}：{:.1} / {} MiB",path.file_name().unwrap().to_string_lossy(),count as f64/1048576.,total.map(|x|format!("{:.1}",x as f64/1048576.)).unwrap_or("未知".into())));last=Instant::now();}}
    f.sync_all().map_err(err)?;drop(f);if total.is_some_and(|n|n!=count){return Err("下载不完整，已有文件保留，可重试".into());}if let Some(h)=expected{if digest(&temp)?!=h{return Err("下载文件校验失败，未安装".into());}}fs::rename(temp,path).map_err(err)?;
    let config=audio::audio_config(app.clone())?;crate::sensevoice_env::audit(Path::new(config["root"].as_str().ok_or("缺少保存目录")?),"download",json!({"url":url,"path":path,"bytes":count,"sha256":digest(path)?}))?;Ok(())
}
fn digest(p:&Path)->Result<String,String>{let mut f=fs::File::open(p).map_err(err)?;let mut h=Sha256::new();let mut b=[0;65536];loop{let n=f.read(&mut b).map_err(err)?;if n==0{break;}h.update(&b[..n]);}Ok(format!("{:x}",h.finalize()))}
fn command(app:&tauri::AppHandle,mut c:Command,root:&Path)->Result<(),String>{
    let temp=root.join("temp");fs::create_dir_all(&temp).map_err(err)?;
    // 避免父进程 PowerShell 7 模块路径污染系统 PowerShell 5 的签名检查。
    crate::sensevoice_env::isolate(&mut c,root,&temp)?;
    let config=audio::audio_config(app.clone())?;let user_root=PathBuf::from(config["root"].as_str().ok_or("缺少保存目录")?);
    crate::sensevoice_env::audit(&user_root,"install-command",json!({"executable":c.get_program().to_string_lossy(),"arguments":c.get_args().map(|a|a.to_string_lossy().to_string()).collect::<Vec<_>>(),"temp":temp,"pipCache":root.join("pip-cache")}))?;
    c.stdout(Stdio::piped()).stderr(Stdio::piped()).creation_flags(0x08000000);
    let mut child=c.spawn().map_err(err)?;let stdout=child.stdout.take().unwrap();let stderr=child.stderr.take().unwrap();let a=app.clone();let reader=std::thread::spawn(move||{for line in BufReader::new(stdout).lines().map_while(Result::ok){stage(&a,&line.chars().take(180).collect::<String>());}});let errors=std::thread::spawn(move||{let mut bytes=Vec::new();let _=BufReader::new(stderr).read_to_end(&mut bytes);String::from_utf8_lossy(&bytes).chars().rev().take(1400).collect::<String>().chars().rev().collect::<String>()});
    let started=Instant::now();loop{if let Some(code)=child.try_wait().map_err(err)?{let _=reader.join();let reason=errors.join().unwrap_or_default();return if code.success(){Ok(())}else{Err(format!("安装步骤失败：{reason}；已下载文件保留，可重试"))};}if started.elapsed()>Duration::from_secs(2400){let _=child.kill();let _=child.wait();return Err("安装超时，已有资源保留，请重试".into());}std::thread::sleep(Duration::from_millis(200));}
}
#[repr(C)]struct MemoryStatus{len:u32,load:u32,total:u64,avail:u64,page:u64,avail_page:u64,virt:u64,avail_virt:u64,ext:u64}
#[link(name="kernel32")]extern "system"{fn GetDiskFreeSpaceExW(p:*const u16,avail:*mut u64,total:*mut u64,free:*mut u64)->i32;fn GlobalMemoryStatusEx(s:*mut MemoryStatus)->i32;}
fn resources(root:&Path)->Result<(),String>{
    use std::os::windows::ffi::OsStrExt;
    if std::env::consts::ARCH!="x86_64"||std::env::var("PROCESSOR_ARCHITEW6432").unwrap_or_default().contains("ARM"){return Err("当前自动安装支持 Windows x64；此架构需单独适配".into());}
    let mut m=MemoryStatus{len:std::mem::size_of::<MemoryStatus>() as u32,load:0,total:0,avail:0,page:0,avail_page:0,virt:0,avail_virt:0,ext:0};if unsafe{GlobalMemoryStatusEx(&mut m)}==0{return Err("无法检测可用内存".into());}if m.avail<1_500_000_000{return Err("当前可用内存不足 1.5 GB，请关闭部分程序后重试".into());}
    let w:Vec<u16>=root.as_os_str().encode_wide().chain(Some(0)).collect();let(mut avail,mut total,mut free)=(0,0,0);if unsafe{GetDiskFreeSpaceExW(w.as_ptr(),&mut avail,&mut total,&mut free)}==0{return Err("无法检测磁盘空间".into());}if avail<4_000_000_000{return Err("安装目录至少需要 4 GB 可用空间".into());}Ok(())
}
fn install(app:&tauri::AppHandle)->Result<Value,String>{
    let mut c=audio::audio_config(app.clone())?;let root=PathBuf::from(c["root"].as_str().unwrap_or(""));non_c(&root)?;fs::create_dir_all(&root).map_err(err)?;
    // 已有完整资源直接实际转写验证；失败也不自动重新下载或覆盖用户模型。
    if crate::sensevoice::available(&c){
        stage(app,"检查本机已有 SenseVoice，不重复下载…");
        audio::check_sync(app.clone()).map_err(|e|format!("本机已有语音资源未通过实际转写检查，未重新下载：{e}"))?;
        return audio::audio_config(app.clone());
    }
    stage(app,"检查 Windows 架构、内存与磁盘…");resources(&root)?;
    // 仅认可用户保存的路径；自定义资源不可用时报告错误，不改写其配置或目录。
    crate::sensevoice_env::audit(&root,"install-start",json!({"python":c["sensePython"],"runtime":c["senseRuntime"],"models":c["senseModels"],"root":root,"automaticExternalDiscovery":false}))?;
    if has_custom_paths(&c,&root){return Err("已保存的自定义语音资源未通过检查；原配置与文件已保留，请检查所选路径".into());}
    let owned=root.join("sensevoice");fs::create_dir_all(&owned).map_err(err)?;
    let models=owned.join("models");
    let py=owned.join("python");fs::create_dir_all(&py).map_err(err)?;let exe=py.join("python.exe");
    if !exe.is_file()||!py.join(".signature-verified").is_file(){stage(app,"安装独立 Python 运行环境…");let zip=owned.join("python-3.11.9.zip");if !zip.is_file(){download(app,"https://www.python.org/ftp/python/3.11.9/python-3.11.9-embed-amd64.zip",&zip,None)?;}let mut cmd=Command::new("powershell.exe");cmd.args(["-NoProfile","-NonInteractive","-Command","[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $ErrorActionPreference='Stop'; Expand-Archive -LiteralPath $env:QJ_ZIP -DestinationPath $env:QJ_DEST -Force; $s=Get-AuthenticodeSignature -LiteralPath (Join-Path $env:QJ_DEST 'python.exe'); if($s.Status -ne 'Valid' -or $s.SignerCertificate.Subject -notmatch 'Python Software Foundation'){throw 'Python signature verification failed'}"]).env("QJ_ZIP",zip).env("QJ_DEST",&py);command(app,cmd,&owned)?;fs::write(py.join(".signature-verified"),"Python Software Foundation Authenticode").map_err(err)?;}
    fs::write(py.join("python311._pth"),"python311.zip\n.\nLib/site-packages\n../runtime/Lib/site-packages\nimport site\n").map_err(err)?;
    let runtime=owned.join("runtime");let packages=runtime.join("Lib/site-packages");fs::create_dir_all(&packages).map_err(err)?;
    if !packages.join(".qingjian-dependencies-v1").is_file(){
        stage(app,"准备语音依赖…");let meta:Value=reqwest::blocking::get("https://pypi.org/pypi/pip/25.0.1/json").map_err(err)?.error_for_status().map_err(err)?.json().map_err(err)?;let wheel=meta["urls"].as_array().and_then(|a|a.iter().find(|x|x["filename"].as_str().is_some_and(|s|s.ends_with(".whl")))).ok_or("未找到 pip 安装文件")?;let p=owned.join("pip.whl");download(app,wheel["url"].as_str().ok_or("pip 地址缺失")?,&p,wheel["digests"]["sha256"].as_str())?;
        let mut cmd=Command::new(&exe);cmd.args(["-c","import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])"]).arg(p).arg(py.join("Lib/site-packages"));command(app,cmd,&owned)?;
        // 便携 Python 没有系统 setuptools；先准备纯 Python 构建工具，避免要求用户安装开发环境。
        let mut build_tools=Command::new(&exe);build_tools.args(["-m","pip","install","--report"]).arg(owned.join("build-tools-report.json")).args(["--upgrade","--target"]).arg(py.join("Lib/site-packages")).args(["--index-url","https://pypi.org/simple","setuptools==83.0.0","wheel==0.47.0","packaging==24.2"]);command(app,build_tools,&owned)?;
        let mut cmd=Command::new(&exe);cmd.args(["-m","pip","install","--report"]).arg(owned.join("dependencies-report.json")).args(["--upgrade","--no-build-isolation","--target"]).arg(&packages).args(["--index-url","https://pypi.org/simple","--extra-index-url","https://download.pytorch.org/whl/cpu","torch==2.4.1+cpu","torchaudio==2.4.1+cpu","funasr==1.3.29","numpy<2","opencc-python-reimplemented==0.1.7"]);command(app,cmd,&owned)?;fs::write(packages.join(".qingjian-dependencies-v1"),"python311 torch2.4.1 funasr1.3.29").map_err(err)?;
    }
    {
        for (name,repo,revision,files) in [
            ("SenseVoiceSmall","FunAudioLLM/SenseVoiceSmall","3847d57b6bdf2dd8875cb1508d2af43d80a16bf7",vec!["model.pt","config.yaml","configuration.json","am.mvn","chn_jpn_yue_eng_ko_spectok.bpe.model"]),
            ("fsmn-vad","funasr/fsmn-vad","df20e6b30c653645fa4ff125cacfcabd1020a669",vec!["model.pt","config.yaml","configuration.json","am.mvn"])]{
            let dir=models.join(name);fs::create_dir_all(&dir).map_err(err)?;
            for file in files{let hash=match (name,file){("SenseVoiceSmall","model.pt")=>Some("833ca2dcfdf8ec91bd4f31cfac36d6124e0c459074d5e909aec9cabe6204a3ea"),("fsmn-vad","model.pt")=>Some("b3be75be477f0780277f3bae0fe489f48718f585f3a6e45d7dd1fbb1a4255fc5"),_=>None};download(app,&format!("https://huggingface.co/{repo}/resolve/{revision}/{file}"),&dir.join(file),hash)?;}
        }c["senseModels"]=json!(models);
    }
    c["sensePython"]=json!(exe);c["senseRuntime"]=json!(runtime);audio::save_setting(app,"audio_config",&c)?;stage(app,"实际转写验证…");audio::check_sync(app.clone())?;audio::audio_config(app.clone())
}
fn has_custom_paths(c:&Value,root:&Path)->bool{
    [("sensePython","sensevoice/python/python.exe"),("senseRuntime","sensevoice/runtime"),("senseModels","sensevoice/models")].iter().any(|(key,relative)|Path::new(c[*key].as_str().unwrap_or(""))!=root.join(relative))
}
#[cfg(test)]mod tests{
    #[test]fn custom_paths_are_preserved(){let root=std::path::Path::new("F:/chosen");let mut c=serde_json::json!({"root":root});crate::sensevoice::defaults(&mut c);assert!(!super::has_custom_paths(&c,root));c["senseModels"]=serde_json::json!("F:/explicit-models");let before=c.clone();assert!(super::has_custom_paths(&c,root));assert_eq!(c,before);}
}
#[tauri::command]
pub async fn install_sensevoice(app:tauri::AppHandle)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||{
    let _guard=audio::ProcessingGuard::acquire()?;if audio::active(){return Err("请先停止录音".into());}
    audio::save_setting(&app,"sensevoice_install",&json!({"state":"installing"}))?;let result=install(&app);audio::save_setting(&app,"sensevoice_install",&match &result{Ok(_)=>json!({"state":"ready"}),Err(e)=>json!({"state":"repair","error":e})})?;result
}).await.map_err(err)?}
