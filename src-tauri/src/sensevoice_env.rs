//! 语音子进程只使用显式配置的运行程序、依赖及当前工作目录下的缓存。
use std::{fs,io::Write,path::Path,process::Command};
use serde_json::{json,Value};

pub fn isolate(command:&mut Command,root:&Path,temp:&Path)->Result<(),String>{
    fs::create_dir_all(temp).map_err(|e|e.to_string())?;
    // 清除父进程的Python/Conda/pip配置；不修改系统环境或已有用户资源。
    for (key,_) in std::env::vars_os(){let name=key.to_string_lossy().to_ascii_uppercase();if name.starts_with("PYTHON")||name.starts_with("PIP_")||name.starts_with("CONDA")||name=="VIRTUAL_ENV"||name=="PSMODULEPATH"{command.env_remove(key);}}
    let windows=std::env::var_os("SystemRoot").ok_or("无法定位Windows系统目录")?;
    let system=std::path::PathBuf::from(windows);
    let mut paths=vec![system.join("System32"),system.clone(),system.join("System32/WindowsPowerShell/v1.0")];
    if let Some(parent)=Path::new(command.get_program()).parent().filter(|p|p.is_absolute()){paths.insert(0,parent.to_path_buf());}
    command.env("PATH",std::env::join_paths(paths).map_err(|e|e.to_string())?)
        .env("TEMP",temp).env("TMP",temp).env("PYTHONNOUSERSITE","1")
        .env("PYTHONDONTWRITEBYTECODE","1").env("PYTHONIOENCODING","utf-8")
        .env("PIP_CONFIG_FILE","NUL").env("PIP_DISABLE_PIP_VERSION_CHECK","1")
        .env("PIP_CACHE_DIR",root.join("pip-cache"));
    for name in ["HF_HOME","HF_HUB_CACHE","HUGGINGFACE_HUB_CACHE","MODELSCOPE_CACHE","NUMBA_CACHE_DIR","MPLCONFIGDIR","TORCH_HOME","XDG_CACHE_HOME"]{command.env(name,root.join("cache"));}
    Ok(())
}

pub fn audit(root:&Path,event:&str,details:Value)->Result<(),String>{
    let directory=root.join("sensevoice/logs");fs::create_dir_all(&directory).map_err(|e|e.to_string())?;
    let mut f=fs::OpenOptions::new().append(true).create(true).open(directory.join("resources.jsonl")).map_err(|e|e.to_string())?;
    // 只写预先选取的资源字段，不转储环境变量、API凭据或录音内容。
    writeln!(f,"{}",json!({"at":chrono::Utc::now().to_rfc3339(),"event":event,"details":details})).map_err(|e|e.to_string())
}

#[cfg(test)]mod tests{
    #[test]fn python_isolation_is_explicit(){
        let root=std::env::temp_dir().join(format!("qj-env-{}",uuid::Uuid::new_v4()));
        let mut c=std::process::Command::new(root.join("python/python.exe"));
        super::isolate(&mut c,&root,&root.join("temp")).unwrap();
        let env:std::collections::HashMap<_,_>=c.get_envs().map(|(k,v)|(k.to_string_lossy().to_string(),v.map(|s|s.to_string_lossy().to_string()))).collect();
        assert_eq!(env["PYTHONNOUSERSITE"],Some("1".into()));
        assert_eq!(env["PIP_CONFIG_FILE"],Some("NUL".into()));
        assert!(env["PIP_CACHE_DIR"].as_ref().unwrap().starts_with(root.to_str().unwrap()));
        std::fs::remove_dir(root.join("temp")).unwrap();std::fs::remove_dir(root).unwrap();
    }
}
