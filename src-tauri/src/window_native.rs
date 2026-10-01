//! Windows 原生层：拖动生命周期、真实置顶层级和无焦点修复。
use std::{sync::Mutex,time::Instant};
use tauri::Manager;
#[derive(Clone,Copy,Default)]
pub struct Drag {pub serial:u64,pub dragging:bool,pub moved:bool,pub released:Option<Instant>,pub dx:i32,pub dy:i32,custom:bool,anchor:Option<(i32,i32)>,origin:Option<(i32,i32)>}
static DRAG:Mutex<Drag>=Mutex::new(Drag{serial:0,dragging:false,moved:false,released:None,dx:0,dy:0,custom:false,anchor:None,origin:None});
#[repr(C)]struct Point{x:i32,y:i32}
#[repr(C)]struct Rect{left:i32,top:i32,right:i32,bottom:i32}
#[derive(Clone,Copy,Debug,serde::Serialize,PartialEq)]
pub struct Bounds{pub left:i32,pub top:i32,pub right:i32,pub bottom:i32}
impl Bounds{pub fn contains(self,x:i32,y:i32)->bool{x>=self.left&&x<self.right&&y>=self.top&&y<self.bottom}}
#[repr(C)]struct MonitorInfo{size:u32,monitor:Rect,work:Rect,flags:u32}
#[derive(Clone,Copy,serde::Serialize)]pub struct Geometry{pub outer:Bounds,pub visible:Bounds,pub screen:Bounds,pub work:Bounds,pub maximized:bool}
impl From<Rect> for Bounds{fn from(r:Rect)->Self{Self{left:r.left,top:r.top,right:r.right,bottom:r.bottom}}}
#[link(name="dwmapi")]extern "system"{fn DwmGetWindowAttribute(h:isize,key:u32,value:*mut std::ffi::c_void,size:u32)->i32;}
#[link(name="user32")]
extern "system" {
 fn SetWindowPos(h:isize,after:isize,x:i32,y:i32,w:i32,hgt:i32,flags:u32)->i32;
 fn GetWindowLongPtrW(h:isize,index:i32)->isize;
 fn GetForegroundWindow()->isize;fn GetWindow(h:isize,cmd:u32)->isize;
 fn GetWindowThreadProcessId(h:isize,pid:*mut u32)->u32;
 fn IsWindowVisible(h:isize)->i32;
 fn ShowWindow(h:isize,cmd:i32)->i32;
 fn GetCursorPos(p:*mut Point)->i32;fn GetWindowRect(h:isize,r:*mut Rect)->i32;
 fn GetAsyncKeyState(k:i32)->i16;fn SetCapture(h:isize)->isize;fn ReleaseCapture()->i32;
 fn GetClientRect(h:isize,r:*mut Rect)->i32;fn ClientToScreen(h:isize,p:*mut Point)->i32;
 fn MonitorFromWindow(h:isize,flags:u32)->isize;fn GetMonitorInfoW(h:isize,info:*mut MonitorInfo)->i32;
 fn IsZoomed(h:isize)->i32;
}
#[link(name="comctl32")]
extern "system" {
 fn SetWindowSubclass(h:isize,callback:unsafe extern "system" fn(isize,u32,usize,isize,usize,usize)->isize,id:usize,data:usize)->i32;
 fn RemoveWindowSubclass(h:isize,callback:unsafe extern "system" fn(isize,u32,usize,isize,usize,usize)->isize,id:usize)->i32;
 fn DefSubclassProc(h:isize,msg:u32,w:usize,l:isize)->isize;
}
unsafe extern "system" fn subclass(h:isize,msg:u32,w:usize,l:isize,id:usize,_data:usize)->isize{
 // 底部握柄使用原生鼠标捕获，绕过 Windows 标题栏移动循环的顶部限制。
 // 坐标直接来自物理光标位置，跨屏和不同 DPI 时不累积 CSS 像素误差。
 let d=drag();
 if d.custom&&msg==0x0200{
  if let Some((ax,ay))=d.anchor{let mut p=Point{x:0,y:0};let mut r=Rect{left:0,top:0,right:0,bottom:0};
   if GetCursorPos(&mut p)!=0&&GetWindowRect(h,&mut r)!=0{
    if p.x-ax!=r.left||p.y-ay!=r.top{if SetWindowPos(h,0,p.x-ax,p.y-ay,0,0,0x215)!=0{if let Ok(mut d)=DRAG.lock(){d.moved=true;if let Some((x,y))=d.origin{d.dx=p.x-x;d.dy=p.y-y;}}}}
   }
  }return 0;
 }
 if d.custom&&msg==0x0202{
  if let Ok(mut d)=DRAG.lock(){d.custom=false;d.dragging=false;d.anchor=None;d.released=if d.moved{Some(Instant::now())}else{None};}
  ReleaseCapture();return 0;
 }
 if d.custom&&matches!(msg,0x0215|0x001f){if let Ok(mut d)=DRAG.lock(){d.custom=false;d.dragging=false;d.anchor=None;d.released=None;}}
 // 本应用的移动完全由鼠标捕获完成，不能进入 Windows SC_MOVE/Aero Snap。
 // 标题栏按钮仍使用普通点击；只有明确的最大化按钮能执行最大化。
 if (msg==0x0112&&(w&0xfff0)==0xf010)||(msg==0x00a3&&w==2){return 0;}
 // 只把用户的系统移动循环当作拖动；程序恢复位置与缩放不产生隐藏计时。
 if let Ok(mut d)=DRAG.lock(){match msg{
  0x0231=>{d.serial+=1;d.dragging=true;d.moved=false;d.released=None;d.custom=false;d.anchor=None;d.origin=None;d.dx=0;d.dy=0;},
  0x0216=>{d.moved=true;},
  0x0232=>{d.dragging=false;d.released=if d.moved{Some(Instant::now())}else{None};},
  _=>{}
 }}
 if msg==0x0082{RemoveWindowSubclass(h,subclass,id);}
 DefSubclassProc(h,msg,w,l)
}
pub fn attach(w:&tauri::WebviewWindow)->Result<(),String>{
 let h=w.hwnd().map_err(|e|e.to_string())?.0 as isize;
 if unsafe{SetWindowSubclass(h,subclass,1,0)}==0{return Err("无法监听窗口拖动".into());}Ok(())
}
pub fn drag()->Drag{DRAG.lock().map(|d|*d).unwrap_or_default()}
#[tauri::command]
pub fn begin_grip_drag(app:tauri::AppHandle)->Result<(),String>{
 let window=app.get_webview_window("main").ok_or("主窗口不可用")?;
 app.run_on_main_thread(move||{if let Ok(h)=window.hwnd(){unsafe{
  let h=h.0 as isize;let mut p=Point{x:0,y:0};let mut r=Rect{left:0,top:0,right:0,bottom:0};
  // 必须来自仍按下的真实鼠标，不把普通程序定位误判为用户拖动。
  if IsZoomed(h)==0&&GetAsyncKeyState(1)<0&&GetCursorPos(&mut p)!=0&&GetWindowRect(h,&mut r)!=0{
   if let Ok(mut d)=DRAG.lock(){d.serial+=1;d.dragging=true;d.moved=false;d.released=None;d.custom=true;d.anchor=Some((p.x-r.left,p.y-r.top));d.origin=Some((p.x,p.y));d.dx=0;d.dy=0;}
   SetCapture(h);
  }
 }}}).map_err(|e|e.to_string())
}
pub fn geometry(w:&tauri::WebviewWindow)->Result<Geometry,String>{
 let h=w.hwnd().map_err(|e|e.to_string())?.0 as isize;
 unsafe{let mut outer=Rect{left:0,top:0,right:0,bottom:0};if GetWindowRect(h,&mut outer)==0{return Err("无法读取窗口边界".into());}
 let mut visible=Rect{left:0,top:0,right:0,bottom:0};
 // DWM 扩展边界排除了透明缩放框和阴影；失败时使用可见客户端物理坐标。
 if DwmGetWindowAttribute(h,9,&mut visible as *mut _ as *mut _,std::mem::size_of::<Rect>() as u32)!=0{
  let mut p=Point{x:0,y:0};if GetClientRect(h,&mut visible)==0||ClientToScreen(h,&mut p)==0{return Err("无法读取可见窗口边界".into());}
  visible.left+=p.x;visible.right+=p.x;visible.top+=p.y;visible.bottom+=p.y;
 }
 let monitor=MonitorFromWindow(h,2);let mut info=MonitorInfo{size:std::mem::size_of::<MonitorInfo>() as u32,monitor:Rect{left:0,top:0,right:0,bottom:0},work:Rect{left:0,top:0,right:0,bottom:0},flags:0};
 if monitor==0||GetMonitorInfoW(monitor,&mut info)==0{return Err("无法读取窗口所在显示器".into());}
 Ok(Geometry{outer:outer.into(),visible:visible.into(),screen:info.monitor.into(),work:info.work.into(),maximized:IsZoomed(h)!=0})}
}
pub fn toggle_maximize(w:&tauri::WebviewWindow)->Result<bool,String>{let h=w.hwnd().map_err(|e|e.to_string())?.0 as isize;unsafe{ShowWindow(h,if IsZoomed(h)!=0{9}else{3});Ok(IsZoomed(h)!=0)}}
pub fn show_no_activate(w:&tauri::WebviewWindow)->tauri::Result<()>{
 let h=w.hwnd()?.0 as isize;
 // 外侧标签的位置同步频繁；重复 SW_SHOW 会干扰前台应用，只有首次显示才调用。
 unsafe{if IsWindowVisible(h)==0{ShowWindow(h,4);}}Ok(())
}
pub fn hide(w:&tauri::WebviewWindow)->tauri::Result<()>{let h=w.hwnd()?.0 as isize;unsafe{ShowWindow(h,0);}Ok(())}
pub fn topmost(w:&tauri::WebviewWindow,value:bool)->Result<(),String>{
 let h=w.hwnd().map_err(|e|e.to_string())?.0 as isize;
 // NOACTIVATE 防止切换应用、恢复窗口层级时抢走其他软件的输入焦点。
 if unsafe{SetWindowPos(h,if value{-1}else{-2},0,0,0,0,0x213)}==0{return Err("Windows 窗口层级更新失败".into());}Ok(())
}
fn rank(mut h:isize)->u32{let mut n=0;unsafe{while h!=0&&n<4096{h=GetWindow(h,3);n+=1;}}n}
pub fn snapshot(app:&tauri::AppHandle)->serde_json::Value{
 let windows=["main","quick","edge"].into_iter().filter_map(|label|{let w=app.get_webview_window(label)?;let h=w.hwnd().ok()?.0 as isize;
  Some(serde_json::json!({"label":label,"hwnd":h,"topmost":unsafe{GetWindowLongPtrW(h,-20)&8!=0},"visible":unsafe{IsWindowVisible(h)!=0},"rank":rank(h),"geometry":geometry(&w).ok()}))}).collect::<Vec<_>>();
 let fg=unsafe{GetForegroundWindow()};serde_json::json!({"windows":windows,"foreground":fg,"foregroundTopmost":unsafe{GetWindowLongPtrW(fg,-20)&8!=0},"foregroundRank":rank(fg)})
}
pub fn repair(app:&tauri::AppHandle,value:bool)->bool{
 let fg=unsafe{GetForegroundWindow()};let mut pid=0;
 unsafe{GetWindowThreadProcessId(fg,&mut pid);}
 let foreground_normal=fg!=0&&pid!=std::process::id()&&unsafe{GetWindowLongPtrW(fg,-20)&8==0};let mut repaired=false;
 for label in ["main","quick","edge"]{if let Some(w)=app.get_webview_window(label){if let Ok(h)=w.hwnd(){let h=h.0 as isize;
  let actual=unsafe{GetWindowLongPtrW(h,-20)&8!=0};
  // 既检查标志，也检查普通前台窗口是否实际排到了轻笺上方。
  if actual!=value || (value&&foreground_normal&&unsafe{IsWindowVisible(h)!=0}&&rank(fg)<rank(h)) {if topmost(&w,value).is_ok(){repaired=true;}}
 }}}
 repaired
}
