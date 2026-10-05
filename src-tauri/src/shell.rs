use crate::{database::Database, quick_window::sync_quick};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{collections::hash_map::DefaultHasher, hash::{Hash, Hasher}, sync::Mutex, time::{Duration, Instant}};
use tauri::{Emitter,Manager, PhysicalPosition, PhysicalSize};
use crate::window_native::Bounds;
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
pub struct Settings { transparency:u8,always_on_top:bool, edge_hide:bool, auto_start:bool, x:Option<i32>, y:Option<i32>, height:f64, error:String }
impl Default for Settings { fn default()->Self{Self{transparency:0,always_on_top:false,edge_hide:false,auto_start:std::env::var_os("QINGJIAN_TEST_MODE").is_none(),x:None,y:None,height:720.0,error:String::new()}} }
pub struct Runtime { settings:Settings, hidden:Option<(i32,i32)>, dock:Option<Dock>, sensor_armed:bool, busy:bool, drag_serial:u64, pending:Option<(Edge,Instant,u64,bool)>, last_save:Instant, repairs:u64 }
#[derive(Clone,Copy)]struct Dock{edge:Edge,sensor:Bounds}
#[derive(Clone,Copy,Debug,PartialEq,Serialize)]
#[serde(rename_all="lowercase")]
enum Edge {Left,Right,Top}
fn contact(v:Bounds,s:Bounds,dx:i32,dy:i32)->Option<Edge>{
    // 全部使用物理像素的可见边界；最多允许 1 px 取整误差，不计阴影或缩放框。
    let candidates=[(Edge::Left,s.left-v.left,-dx),(Edge::Right,v.right-s.right,dx),(Edge::Top,s.top-v.top,-dy)];
    candidates.into_iter().filter(|(_,depth,_)|*depth>=-1).max_by(|a,b|{
        // 角落优先遵循实际朝向该边的拖动；同向时比较穿越深度，最终固定排序保证稳定。
        a.2.max(0).cmp(&b.2.max(0)).then(a.1.cmp(&b.1))
    }).map(|c|c.0)
}
fn sensor(edge:Edge,v:Bounds,s:Bounds)->Bounds{
    // 纯光标检测，没有感应窗口，不截获点击；长度仅覆盖隐藏前窗口占用的边缘范围。
    match edge{
        Edge::Left=>Bounds{left:s.left,right:s.left+2,top:v.top.max(s.top),bottom:v.bottom.min(s.bottom)},
        Edge::Right=>Bounds{left:s.right-2,right:s.right,top:v.top.max(s.top),bottom:v.bottom.min(s.bottom)},
        Edge::Top=>Bounds{left:v.left.max(s.left),right:v.right.min(s.right),top:s.top,bottom:s.top+2},
    }
}
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
    let pos=app.try_state::<Mutex<Runtime>>().and_then(|s|{let mut r=s.lock().ok()?;r.pending=None;r.sensor_armed=false;r.hidden.take()});
    if let (Some(w),Some((x,y)))=(app.get_webview_window("main"),pos){
        // 唤出时保留停靠方向，完整可见内容放回该显示器工作区；不可见缩放框允许在外侧。
        let (x,y)=crate::window_native::geometry(&w).map(|g|{
            let left=g.visible.left.clamp(g.work.left,(g.work.right-(g.visible.right-g.visible.left)).max(g.work.left));
            let top=g.visible.top.clamp(g.work.top,(g.work.bottom-(g.visible.bottom-g.visible.top)).max(g.work.top));
            (left-(g.visible.left-g.outer.left),top-(g.visible.top-g.outer.top))
        }).unwrap_or((x,y));
        let _=w.set_position(PhysicalPosition::new(x,y));let _=crate::window_native::show_no_activate(&w);
    }
    let _=sync_quick(app);reassert(app);
}
pub fn reassert(app:&tauri::AppHandle){
    let value=app.try_state::<Mutex<Runtime>>().and_then(|s|s.lock().ok().map(|r|r.settings.always_on_top));
    if let Some(v)=value{crate::window_native::repair(app,v);}
}
#[tauri::command]
pub fn reveal_shell(app:tauri::AppHandle){restore(&app);}
#[tauri::command]
pub fn toggle_shell_maximize(app:tauri::AppHandle)->Result<bool,String>{
    restore(&app);if let Some(s)=app.try_state::<Mutex<Runtime>>(){if let Ok(mut r)=s.lock(){r.pending=None;r.dock=None;}}
    let w=app.get_webview_window("main").ok_or("主窗口不可用")?;
    let result=crate::window_native::toggle_maximize(&w)?;
    // 原生大小切换结束即同步，不依赖延后的框架 Resize/最大化缓存事件。
    sync_quick(&app).map_err(|e|e.to_string())?;reassert(&app);Ok(result)
}
#[tauri::command]
pub fn get_shell_diagnostics(app:tauri::AppHandle)->Result<serde_json::Value,String>{
    let d=crate::window_native::drag();let state=app.try_state::<Mutex<Runtime>>().ok_or("窗口正在初始化，请稍后重试")?;let r=state.lock().map_err(|_|"窗口状态不可用")?;
    let geometry=app.get_webview_window("main").and_then(|w|crate::window_native::geometry(&w).ok());
    Ok(serde_json::json!({"native":crate::window_native::snapshot(&app),"geometry":geometry,"maximized":geometry.is_some_and(|g|g.maximized),"desiredTopmost":r.settings.always_on_top,"hiddenEdge":if r.hidden.is_some(){r.dock.map(|d|d.edge)}else{None},"dockedEdge":r.dock.map(|d|d.edge),"sensor":r.dock.map(|d|d.sensor),"sensorArmed":r.sensor_armed,"dragSerial":d.serial,"dragging":d.dragging,"pending":r.pending.map(|p|p.0),"busy":r.busy,"layerRepairs":r.repairs}))
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
    // WebView 创建会泵送 IPC：先注册运行状态，避免界面已加载但状态尚未注册的竞态。
    persist(app,&settings)?;
    app.manage(Mutex::new(Runtime{settings:settings.clone(),hidden:None,dock:None,sensor_armed:false,busy:false,drag_serial:0,pending:None,last_save:Instant::now(),repairs:0}));
    // 不再创建边缘把手 WebView；主窗口和快捷标签真正隐藏后，屏幕上不留任何标识。
    crate::window_native::attach(&main)?;
    crate::window_native::transparency(app,settings.transparency)?;

    reassert(app);
    let handle=app.clone();
    // 20 毫秒检测间隔避免旧的 100 毫秒轮询额外延长半秒隐藏等待。
    std::thread::spawn(move || loop { std::thread::sleep(Duration::from_millis(20)); let a=handle.clone(); let _=handle.run_on_main_thread(move||tick(&a)); });
    Ok(())
}
#[tauri::command]
pub fn get_shell_settings(app:tauri::AppHandle)->Result<Settings,String>{let mut s=app.try_state::<Mutex<Runtime>>().ok_or("窗口正在初始化，请稍后重试")?.lock().map_err(|_|"窗口状态不可用")?.settings.clone();s.auto_start=autostart_actual();Ok(s)}
#[tauri::command]
pub fn set_shell_setting(app:tauri::AppHandle,key:String,value:bool)->Result<Settings,String>{
    let mut s=app.try_state::<Mutex<Runtime>>().ok_or("窗口正在初始化，请稍后重试")?.lock().map_err(|_|"窗口状态不可用")?.settings.clone();
    match key.as_str(){
        // Tauri 层级切换会隐藏 owned 快捷窗口；统一使用不激活、不改变可见性的原生接口。
        "alwaysOnTop"=>{s.always_on_top=value;},
        "edgeHide"=>{restore(&app);if let Some(state)=app.try_state::<Mutex<Runtime>>(){if let Ok(mut r)=state.lock(){r.pending=None;r.dock=None;}}s.edge_hide=value;},
        "autoStart"=>{set_autostart(value)?;s.auto_start=value;},
        _=>return Err("未知窗口设置".into())
    }
    s.error.clear();persist(&app,&s)?;app.try_state::<Mutex<Runtime>>().ok_or("窗口正在初始化，请稍后重试")?.lock().map_err(|_|"窗口状态不可用")?.settings=s;
    for label in ["main","quick","edge"]{if key=="alwaysOnTop"{if let Some(w)=app.get_webview_window(label){crate::window_native::topmost(&w,value)?;}}}
    get_shell_settings(app)
}
#[tauri::command]
pub fn set_shell_busy(app:tauri::AppHandle,busy:bool){if let Some(s)=app.try_state::<Mutex<Runtime>>(){if let Ok(mut r)=s.lock(){r.busy=busy;}}}

#[tauri::command]
pub fn apply_backup_window(app:tauri::AppHandle,settings:serde_json::Value)->Result<(),String>{
    // 只有用户勾选“应用备份设置”后调用；不从备份执行任意注册表或路径操作。
    for key in ["alwaysOnTop","edgeHide","autoStart"]{if let Some(value)=settings[key].as_bool(){set_shell_setting(app.clone(),key.into(),value)?;}}
    if let Some(value)=settings["transparency"].as_u64(){set_background_transparency(app.clone(),u8::try_from(value).map_err(|_|"透明度无效")?)?;}
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

fn tick(app:&tauri::AppHandle){
    if let Some(instance)=app.try_state::<Instance>(){if unsafe{WaitForSingleObject(instance.event,0)}==0 {restore(app);crate::show_main(app);}}
    let Some(main)=app.get_webview_window("main")else{return};
    let state=app.state::<Mutex<Runtime>>();
    let value=state.lock().map(|r|r.settings.always_on_top).unwrap_or(false);
    if crate::window_native::repair(app,value){if let Ok(mut r)=state.lock(){r.repairs+=1;}}
    let mut p=Point{x:0,y:0};if unsafe{GetCursorPos(&mut p)}==0{return;}
    let d=crate::window_native::drag();let now=Instant::now();
    let protected=crate::audio::active()||crate::audio::processing();
    let Ok(mut r)=state.lock()else{return};
    if r.hidden.is_some(){
        let over=r.dock.is_some_and(|d|d.sensor.contains(p.x,p.y));
        // 隐藏后必须先离开原感应区再进入，防止光标未动就不断隐藏/弹出。
        if !over{r.sensor_armed=true;}
        let reveal=(over&&r.sensor_armed)||!r.settings.edge_hide||r.busy||protected;
        drop(r);if reveal{restore(app);}return;
    }
    if !main.is_visible().unwrap_or(false)||main.is_minimized().unwrap_or(false){r.pending=None;return;}
    let Ok(g)=crate::window_native::geometry(&main)else{r.pending=None;return};
    // 最大化天然接触多条边，不能参与停靠或覆盖普通窗口的恢复位置。
    if g.maximized{r.pending=None;r.dock=None;r.drag_serial=d.serial;return;}
    // 重拖立即取消旧计时。仅处理新一轮 WM_EXITSIZEMOVE，程序移动不触发隐藏。
    if d.dragging{r.pending=None;r.dock=None;return;}
    if d.serial!=r.drag_serial{
        r.drag_serial=d.serial;r.pending=None;
        r.dock=None;
        if r.settings.edge_hide&&!r.busy&&!protected{
            if let Some(released)=d.released{if let Some(edge)=contact(g.visible,g.screen,d.dx,d.dy){
                r.dock=Some(Dock{edge,sensor:sensor(edge,g.visible,g.screen)});r.pending=Some((edge,released,d.serial,false));
            }}
        }
    }
    if !r.settings.edge_hide{r.pending=None;r.dock=None;}
    if r.busy||protected||unsafe{GetAsyncKeyState(1)}<0{r.pending=None;return;}
    if let Some(dock)=r.dock{
        if g.visible.contains(p.x,p.y){
            // 回到窗口内只取消“离开后”的计时，不取消首次拖动松手的计时。
            if r.pending.is_some_and(|p|p.3){r.pending=None;}
        }else if let Some(p)=r.pending.as_mut(){
            // 首次松手后若光标曾离开，返回窗口也应取消原计时；不重置开始时间。
            p.3=true;
        }else{r.pending=Some((dock.edge,now,d.serial,true));}
    }
    if let Some((_,released,serial,_))=r.pending{
        // 三个方向、首次松手与唤出后离开共用唯一的 500 毫秒期限。
        if now.duration_since(released)>=Duration::from_millis(500)&&serial==d.serial{
            r.pending=None;r.hidden=Some((g.outer.left,g.outer.top));r.sensor_armed=!r.dock.is_some_and(|d|d.sensor.contains(p.x,p.y));drop(r);
            // 真正隐藏而不移到相邻显示器；没有任何边缘窗口、残片或色条。
            if let Some(q)=app.get_webview_window("quick"){let _=crate::window_native::hide(&q);}
            let result=crate::window_native::hide(&main);
            if let Err(e)=result{restore(app);if let Ok(mut r)=state.lock(){r.settings.error=e.to_string();}}return;
        }
    }
    if now.duration_since(r.last_save)>Duration::from_secs(2)&&unsafe{GetAsyncKeyState(1)}>=0{
        r.last_save=now;let scale=main.scale_factor().unwrap_or(1.0);let h=main.inner_size().map(|s|s.height as f64/scale).unwrap_or(720.0);
        if r.settings.x!=Some(g.outer.left)||r.settings.y!=Some(g.outer.top)||(r.settings.height-h).abs()>1.0{r.settings.x=Some(g.outer.left);r.settings.y=Some(g.outer.top);r.settings.height=h;let copy=r.settings.clone();drop(r);if let Err(e)=persist(app,&copy){if let Ok(mut r)=state.lock(){r.settings.error=e;}}}
    }
}
#[cfg(test)]mod tests{
 use super::*;
 fn b(left:i32,top:i32,right:i32,bottom:i32)->Bounds{Bounds{left,top,right,bottom}}
 #[test]fn actual_visible_contact_only(){let s=b(0,0,1000,1000);
  assert_eq!(contact(b(20,20,420,700),s,-500,0),None);
  assert_eq!(contact(b(0,20,400,700),s,-500,0),Some(Edge::Left));
  assert_eq!(contact(b(600,20,1000,700),s,500,0),Some(Edge::Right));
  assert_eq!(contact(b(300,0,700,700),s,0,-500),Some(Edge::Top));
  assert_eq!(contact(b(300,300,700,1000),s,0,500),None);
  assert_eq!(contact(b(2,20,402,700),s,-500,0),None);
  assert_eq!(contact(b(1,20,401,700),s,-500,0),Some(Edge::Left));
 }
 #[test]fn corner_follows_drag_direction(){let s=b(0,0,1000,1000);let v=b(-10,-10,400,700);
  assert_eq!(contact(v,s,-400,-20),Some(Edge::Left));assert_eq!(contact(v,s,-20,-400),Some(Edge::Top));
  assert_eq!(contact(b(600,-10,1010,700),s,400,-20),Some(Edge::Right));
 }
 #[test]fn negative_screen_and_limited_sensor(){let s=b(-1920,-300,0,780);let v=b(-1920,20,-1400,700);
  assert_eq!(contact(v,s,-500,0),Some(Edge::Left));let z=sensor(Edge::Left,v,s);
  assert!(z.contains(-1920,200));assert!(!z.contains(-1920,0));assert!(!z.contains(-1917,200));
  assert_eq!(contact(b(1922,100,2500,1000),b(1920,0,5760,2160),-400,0),None);
 }
}

#[tauri::command]
pub fn set_background_transparency(app:tauri::AppHandle,value:u8)->Result<Settings,String>{
 if value>70||value%5!=0{return Err("透明度须为 0%～70%，步进 5%".into());}
 let state=app.try_state::<Mutex<Runtime>>().ok_or("窗口正在初始化")?;
 let mut runtime=state.lock().map_err(|_|"窗口状态不可用")?;
 let mut settings=runtime.settings.clone();settings.transparency=value;
 persist(&app,&settings)?;runtime.settings=settings;drop(runtime);
 crate::window_native::transparency(&app,value)?;
 app.emit("background-transparency",value).map_err(|e|e.to_string())?;
 get_shell_settings(app)
}
