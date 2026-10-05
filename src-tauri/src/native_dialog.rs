// WASAPI 会把共享工作线程初始化为 MTA；Windows 文件选择器要求 STA。
// 每次选择器使用独立线程，避免录音之后选择器静默返回“取消”。
pub(crate) fn run<T:Send+'static>(action:impl FnOnce()->T+Send+'static)->Result<T,String>{
    std::thread::Builder::new().name("qingjian-file-dialog".into()).spawn(action)
        .map_err(|_|"无法打开文件选择器，请稍后重试".to_string())?
        .join().map_err(|_|"文件选择器异常退出，请重试".to_string())
}
