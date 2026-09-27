use crate::{database::Database, quick_window::sync_quick};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{collections::hash_map::DefaultHasher, hash::{Hash, Hasher}, sync::Mutex, time::{Duration, Instant}};
use tauri::{Manager, PhysicalPosition, PhysicalSize};
use winreg::{enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE}, RegKey};

#[repr(C)] struct Point { x:i32,y:i32 }
#[link(name="user32")]
extern "system" { fn GetCursorPos(p:*mut Point)->i32; fn GetAsyncKeyState(key:i32)->i16; }
#[link(name="kernel32")]
extern "system" {
    fn CreateMutexW(a:*const std::ffi::c_void,owner:i32,name:*const u16)->isize;
    fn CreateEventW(a:*const std::ffi::c_void,manual:i32,initial:i32,name:*const u16)->isize;
    fn GetLastError()->u32; fn SetEvent(h:isize)->i32; fn WaitForSingleObject(h:isize,ms:u32)->u32; fn CloseHandle(h:isize)->i32;
}
pub struct Instance { mutex:isize, event:isize }
impl Drop for Instance { fn drop(&mut self){unsafe{CloseHandle(self.event);CloseHandle(self.mutex);}} }
pub fn single_instance() -> Result<Option<Instance>,String> {
    let mut hash=DefaultHasher::new(); std::env::var_os("LOCALAPPDATA").hash(&mut hash);
    let name=format!("Local\\SHUSHIN.Qingjian.{:x}",hash.finish());
    let wide=|s:String|s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mutex_name=wide(name.clone()); let event_name=wide(name+".show");
    unsafe {
        let mutex=CreateMutexW(std::ptr::null(),0,mutex_name.as_ptr());
        if mutex==0 {return Err(format!("单实例锁创建失败：{}",GetLastError()));}
        let existing=GetLastError()==183;
        let event=CreateEventW(std::ptr::null(),0,0,event_name.as_ptr());
        if event==0 {CloseHandle(mutex);return Err("唤起事件创建失败".into());}
        if existing {SetEvent(event);CloseHandle(event);CloseHandle(mutex);return Ok(None);}
        Ok(Some(Instance{mutex,event}))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default,rename_all="camelCase")]
pub struct Settings { always_on_top:bool, edge_hide:bool, auto_start:bool, x:Option<i32>, y:Option<i32>, height:f64, error:String }
impl Default for Settings { fn default()->Self{Self{always_on_top:false,edge_hide:false,auto_start:true,x:None,y:None,height:720.0,error:String::new()}} }
pub struct Runtime { settings:Settings, hidden:Option<(i32,i32)>, busy:bool, outside_since:Option<Instant>, cooldown:Instant, last_save:Instant }
fn registry_name()->String {if std::env::var_os("QINGJIAN_TEST_MODE").is_some(){"QingjianV1_Test".into()}else{"QingjianV1".into()}}
fn startup_command()->Result<String,String>{Ok(format!("\"{}\" --autostart",std::env::current_exe().map_err(|e|e.to_string())?.display()))}
fn set_autostart(enabled:bool)->Result<(),String> {
    let (key,_)=RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Run",KEY_SET_VALUE).map_err(|e|format!("无法修改当前用户启动项：{e}"))?;
    if enabled {key.set_value(registry_name(),&startup_command()?).map_err(|e|format!("启用开机启动失败：{e}"))}
    else {match key.delete_value(registry_name()){Ok(())=>Ok(()),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(()),Err(e)=>Err(format!("关闭开机启动失败：{e}"))}}
}
fn autostart_actual()->bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Run",KEY_READ).ok()
        .and_then(|k|k.get_value::<String,_>(registry_name()).ok()).zip(startup_command().ok()).is_some_and(|(a,b)|a==b)
}
fn persist(app:&tauri::AppHandle,settings:&Settings)->Result<(),String>{
    let db=app.state::<Database>(); let conn=db.0.lock().map_err(|_|"数据库暂不可用")?;
    conn.execute("INSERT INTO app_settings(key,value) VALUES ('window',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(settings).map_err(|e|e.to_string())?]).map_err(|e|format!("窗口设置保存失败：{e}"))?;Ok(())
}
pub fn is_hidden(app:&tauri::AppHandle)->bool {app.try_state::<Mutex<Runtime>>().is_some_and(|s|s.lock().map(|s|s.hidden.is_some()).unwrap_or(false))}
pub fn restore(app:&tauri::AppHandle){
    let pos=app.try_state::<Mutex<Runtime>>().and_then(|s|{let mut r=s.lock().ok()?;r.cooldown=Instant::now()+Duration::from_millis(1200);r.outside_since=None;r.hidden.take()});
    if let (Some(w),Some((x,y)))=(app.get_webview_window("main"),pos){let _=w.set_position(PhysicalPosition::new(x,y));}
    let _=sync_quick(app);
}
pub fn initialize(app:&tauri::AppHandle)->Result<(),String>{
    let raw:Option<String>={let db=app.state::<Database>();let c=db.0.lock().map_err(|_|"数据库暂不可用")?;c.query_row("SELECT value FROM app_settings WHERE key='window'",[],|r|r.get(0)).optional().map_err(|e|e.to_string())?};
    let mut settings=match raw{Some(s)=>serde_json::from_str::<Settings>(&s).map_err(|e|format!("窗口设置损坏：{e}"))?,None=>Settings::default()};
    settings.error=set_autostart(settings.auto_start).err().unwrap_or_default();
    let main=app.get_webview_window("main").ok_or("缺少主窗口")?;
    let monitors=main.available_monitors().map_err(|e|e.to_string())?;
    let monitor=monitors.iter().find(|m|{let a=m.work_area();settings.x.zip(settings.y).is_some_and(|(x,y)|x>=a.position.x&&y>=a.position.y&&x<a.position.x+a.size.width as i32&&y<a.position.y+a.size.height as i32)})
        .or_else(||monitors.first()).ok_or("无法读取显示器")?;
    let a=monitor.work_area();let scale=monitor.scale_factor();
    let max_height=a.size.height as f64/scale;
    // Windows 无边框 WebView 的像素取整向上；向下取整避免 150% 下变成 328 CSS px。
    let width=(327.0*scale).floor() as u32;let height=(settings.height.clamp(480.0_f64.min(max_height),max_height)*scale).round() as u32;
    let x=settings.x.unwrap_or(a.position.x+40).clamp(a.position.x,(a.position.x+a.size.width as i32-width as i32).max(a.position.x));
    let y=settings.y.unwrap_or(a.position.y+20).clamp(a.position.y,(a.position.y+a.size.height as i32-height as i32).max(a.position.y));
    main.set_size(PhysicalSize::new(width,height)).map_err(|e|e.to_string())?;
    // 可见范围按含原生缩放边框的外框计算，避免恢复到屏幕右下方时越界。
    let outer=main.outer_size().map_err(|e|e.to_string())?;
    let x=x.min((a.position.x+a.size.width as i32-outer.width as i32).max(a.position.x));
    let y=y.min((a.position.y+a.size.height as i32-outer.height as i32).max(a.position.y));
    main.set_position(PhysicalPosition::new(x,y)).map_err(|e|e.to_string())?;
    main.set_always_on_top(settings.always_on_top).map_err(|e|e.to_string())?;
    if let Some(q)=app.get_webview_window("quick"){q.set_always_on_top(settings.always_on_top).map_err(|e|e.to_string())?;}
    persist(app,&settings)?;
    app.manage(Mutex::new(Runtime{settings,hidden:None,busy:false,outside_since:None,cooldown:Instant::now()+Duration::from_secs(2),last_save:Instant::now()}));
    let handle=app.clone();
    std::thread::spawn(move || loop { std::thread::sleep(Duration::from_millis(180)); let a=handle.clone(); let _=handle.run_on_main_thread(move||tick(&a)); });
    Ok(())
}
#[tauri::command]
pub fn get_shell_settings(app:tauri::AppHandle)->Result<Settings,String>{let mut s=app.state::<Mutex<Runtime>>().lock().map_err(|_|"窗口状态不可用")?.settings.clone();s.auto_start=autostart_actual();Ok(s)}
#[tauri::command]
pub fn set_shell_setting(app:tauri::AppHandle,key:String,value:bool)->Result<Settings,String>{
    let mut s=app.state::<Mutex<Runtime>>().lock().map_err(|_|"窗口状态不可用")?.settings.clone();
    match key.as_str(){
        "alwaysOnTop"=>{for label in ["main","quick"]{if let Some(w)=app.get_webview_window(label){w.set_always_on_top(value).map_err(|e|e.to_string())?;}}s.always_on_top=value;},
        "edgeHide"=>{restore(&app);s.edge_hide=value;},
        "autoStart"=>{set_autostart(value)?;s.auto_start=value;},
        _=>return Err("未知窗口设置".into())
    }
    s.error.clear();persist(&app,&s)?;app.state::<Mutex<Runtime>>().lock().map_err(|_|"窗口状态不可用")?.settings=s;
    get_shell_settings(app)
}
#[tauri::command]
pub fn set_shell_busy(app:tauri::AppHandle,busy:bool){if let Some(s)=app.try_state::<Mutex<Runtime>>(){if let Ok(mut r)=s.lock(){r.busy=busy;}}}

#[tauri::command]
pub fn apply_backup_window(app:tauri::AppHandle,settings:serde_json::Value)->Result<(),String>{
    // 只有用户勾选“应用备份设置”后调用；不从备份执行任意注册表或路径操作。
    for key in ["alwaysOnTop","edgeHide","autoStart"]{if let Some(value)=settings[key].as_bool(){set_shell_setting(app.clone(),key.into(),value)?;}}
    let main=app.get_webview_window("main").ok_or("窗口不可用")?;
    restore(&app);
    if let Some(m)=main.current_monitor().map_err(|e|e.to_string())?{
        let a=m.work_area();let scale=m.scale_factor();let inner=main.inner_size().map_err(|e|e.to_string())?;
        let height=settings["height"].as_f64().unwrap_or(inner.height as f64/scale).clamp(480.0_f64.min(a.size.height as f64/scale),a.size.height as f64/scale);
        main.set_size(PhysicalSize::new(inner.width,(height*scale).floor()as u32)).map_err(|e|e.to_string())?;
        let size=main.outer_size().map_err(|e|e.to_string())?;let old=main.outer_position().map_err(|e|e.to_string())?;
        let x=settings["x"].as_i64().and_then(|v|i32::try_from(v).ok()).unwrap_or(old.x).clamp(a.position.x,(a.position.x+a.size.width as i32-size.width as i32).max(a.position.x));
        let y=settings["y"].as_i64().and_then(|v|i32::try_from(v).ok()).unwrap_or(old.y).clamp(a.position.y,(a.position.y+a.size.height as i32-size.height as i32).max(a.position.y));
        main.set_position(PhysicalPosition::new(x,y)).map_err(|e|e.to_string())?;
        let runtime=app.state::<Mutex<Runtime>>();let mut r=runtime.lock().map_err(|_|"窗口状态不可用")?;r.settings.x=Some(x);r.settings.y=Some(y);r.settings.height=height;let s=r.settings.clone();drop(r);persist(&app,&s)?;
    }Ok(())
}

fn contains(w:&tauri::WebviewWindow,p:&Point)->bool {if !w.is_visible().unwrap_or(false){return false;}w.outer_position().ok().zip(w.outer_size().ok()).is_some_and(|(a,b)|p.x>=a.x&&p.y>=a.y&&p.x<a.x+b.width as i32&&p.y<a.y+b.height as i32)}
fn tick(app:&tauri::AppHandle){
    if let Some(instance)=app.try_state::<Instance>(){if unsafe{WaitForSingleObject(instance.event,0)}==0 {restore(app);crate::show_main(app);}}
    let Some(main)=app.get_webview_window("main")else{return};
    if !main.is_visible().unwrap_or(false)||main.is_minimized().unwrap_or(false){return;}
    let (Ok(pos),Ok(size),Ok(Some(monitor)))=(main.outer_position(),main.outer_size(),main.current_monitor())else{return};
    let mut p=Point{x:0,y:0};if unsafe{GetCursorPos(&mut p)}==0{return;}
    let mouse_down=unsafe{GetAsyncKeyState(1)}<0;
    let over=contains(&main,&p)||app.get_webview_window("quick").is_some_and(|q|contains(&q,&p));
    let state=app.state::<Mutex<Runtime>>();let Ok(mut r)=state.lock()else{return};
    if r.hidden.is_some(){let reveal=over||!r.settings.edge_hide||r.busy;drop(r);if reveal{restore(app);}return;}
    let a=monitor.work_area();let scale=monitor.scale_factor();let now=Instant::now();
    if now.duration_since(r.last_save)>Duration::from_secs(2)&&!mouse_down {
        r.last_save=now;let h=main.inner_size().map(|s|s.height as f64/scale).unwrap_or(720.0);
        if r.settings.x!=Some(pos.x)||r.settings.y!=Some(pos.y)||(r.settings.height-h).abs()>1.0{r.settings.x=Some(pos.x);r.settings.y=Some(pos.y);r.settings.height=h;let copy=r.settings.clone();drop(r);if let Err(e)=persist(app,&copy){if let Ok(mut r)=state.lock(){r.settings.error=e;}}return;}
    }
    if !r.settings.edge_hide||r.busy||mouse_down||over||now<r.cooldown {r.outside_since=None;return;}
    let start=*r.outside_since.get_or_insert(now);if now.duration_since(start)<Duration::from_millis(900){return;}
    let threshold=(12.0*scale)as i32;let strip=(6.0*scale)as i32;
    let target=if (pos.x-a.position.x).abs()<=threshold{Some((a.position.x-size.width as i32+strip,pos.y))}
        else if (a.position.x+a.size.width as i32-pos.x-size.width as i32).abs()<=threshold{Some((a.position.x+a.size.width as i32-strip,pos.y))}
        else if (pos.y-a.position.y).abs()<=threshold{Some((pos.x,a.position.y-size.height as i32+strip))}else{None};
    if let Some((x,y))=target{r.hidden=Some((pos.x,pos.y));drop(r);if let Some(q)=app.get_webview_window("quick"){let _=q.hide();}if main.set_position(PhysicalPosition::new(x,y)).is_err(){restore(app);}}
}
