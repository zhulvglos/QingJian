use std::path::PathBuf;
pub struct DataRoot(pub PathBuf);

// 在任何数据库或 WebView 窗口创建前解析目录，避免首次启动先写入旧的 C 盘目录。
pub fn resolve()->Result<PathBuf,String>{
    let exe=std::env::current_exe().map_err(|_|"无法确定程序位置")?;
    let exe_dir=exe.parent().ok_or("无法确定程序目录")?;
    let configured=if let Some(value)=std::env::var_os("QINGJIAN_DATA_ROOT"){
        PathBuf::from(value)
    }else if exe_dir.join("data-root.txt").is_file(){
        let text=std::fs::read_to_string(exe_dir.join("data-root.txt")).map_err(|_|"无法读取数据目录配置")?;
        let value=text.lines().next().unwrap_or("").trim();
        if value.is_empty(){return Err("数据目录配置为空".into());}
        let path=PathBuf::from(value);
        if path.is_absolute(){path}else{exe_dir.join(path)}
    }else if exe_dir.file_name().is_some_and(|name|name.to_string_lossy().eq_ignore_ascii_case("app")){
        exe_dir.parent().ok_or("无法确定应用数据目录")?.join("data")
    }else{
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("无法定位用户目录")?).join("SHUSHIN").join("Qingjian")
    };
    if !configured.is_absolute(){return Err("数据目录必须是绝对路径".into());}
    std::fs::create_dir_all(&configured).map_err(|e|format!("创建数据目录失败：{e}"))?;
    let canonical=configured.canonicalize().map_err(|e|format!("无法访问数据目录：{e}"))?;
    // Windows canonicalize 会加入 \\?\ 前缀，内部仍可使用，但会让路径输入框难以阅读。
    let text=canonical.to_string_lossy();
    if let Some(rest)=text.strip_prefix(r"\\?\UNC\"){return Ok(PathBuf::from(format!(r"\\{}",rest)));}
    if let Some(rest)=text.strip_prefix(r"\\?\"){return Ok(PathBuf::from(rest));}
    Ok(canonical)
}
