use std::sync::Mutex;
use tauri::{
    Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

#[derive(Clone, Copy)]
enum QuickSide {
    Left,
    Right,
}

impl QuickSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

pub struct QuickState {
    side: QuickSide,
    expanded: bool,
    pin_count: usize,
    content_height: f64,
    drag_active: bool,
}

impl Default for QuickState {
    fn default() -> Self {
        Self {
            side: QuickSide::Right,
            expanded: false,
            pin_count: 0,
            content_height: 66.0,
            drag_active: false,
        }
    }
}

pub fn create_quick(app: &mut tauri::App) -> tauri::Result<()> {
    let main = app.get_webview_window("main").expect("缺少主窗口");
    // Windows owner 关系让外侧标签位于主窗口上方，并随主窗口最小化。
    WebviewWindowBuilder::new(app, "quick", WebviewUrl::App("quick.html".into()))
        .parent(&main)?
        .title("轻笺快捷标签")
        .inner_size(46.0, 118.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    sync_quick(&app.handle())
}

pub fn sync_quick(app: &tauri::AppHandle) -> tauri::Result<()> {
    let (Some(main), Some(quick)) = (
        app.get_webview_window("main"),
        app.get_webview_window("quick"),
    ) else {
        return Ok(());
    };
    if crate::shell::is_hidden(app) || !main.is_visible()? || main.is_minimized()? {
        quick.hide()?;
        return Ok(());
    }

    let Some(monitor) = main.current_monitor()? else {
        return Ok(());
    };
    let position = main.outer_position()?;
    let size = main.outer_size()?;
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let area_left = area.position.x;
    let area_top = area.position.y;
    let area_right = area_left + area.size.width as i32;
    let area_bottom = area_top + area.size.height as i32;
    let main_right = position.x + size.width as i32;
    let left_gap = position.x - area_left;
    let right_gap = area_right - main_right;
    let top_gap = position.y - area_top;
    let near_edge = (20.0 * scale).round() as i32;

    let state = app.state::<Mutex<QuickState>>();
    let mut state = state.lock().expect("快捷标签位置状态不可用");
    // 只在靠边时按当前显示器工作区换边；居中时沿用最近一次确定的位置。
    if left_gap <= near_edge && (right_gap > near_edge || left_gap <= right_gap) {
        state.side = QuickSide::Right;
    } else if right_gap <= near_edge {
        state.side = QuickSide::Left;
    } else if top_gap <= near_edge {
        state.side = if left_gap <= right_gap {
            QuickSide::Right
        } else {
            QuickSide::Left
        };
    }

    let collapsed_width = (46.0 * scale).round() as i32;
    let desired_width = if state.drag_active {
        (136.0 * scale).round() as i32
    } else if state.expanded {
        (124.0 * scale).round() as i32
    } else {
        collapsed_width
    };
    // 中间位置展开空间不足时选更宽的一侧，保证面板仍在主窗口外。
    let mut side = state.side;
    let mut free = match side {
        QuickSide::Left => left_gap,
        QuickSide::Right => right_gap,
    };
    let other_free = match side {
        QuickSide::Left => right_gap,
        QuickSide::Right => left_gap,
    };
    if free < desired_width && other_free > free {
        side = match side {
            QuickSide::Left => QuickSide::Right,
            QuickSide::Right => QuickSide::Left,
        };
        state.side = side;
        free = other_free;
    }
    let width = if state.expanded || state.drag_active {
        desired_width.min(free.max(collapsed_width))
    } else {
        collapsed_width
    };
    let expanded_height = state.content_height.max(10.0 + state.pin_count.max(1) as f64 * 52.0).min(540.0);
    let height = if state.drag_active { expanded_height.max(118.0) } else if state.expanded { expanded_height } else { 118.0 };
    let height = ((height * scale).round() as i32).min((area_bottom - area_top - 24).max(64));
    // 外侧窗口只跨过几像素边框，减小可见缝隙，同时保留主窗口大部分缩放热区。
    let overlap = (3.0 * scale).round() as i32;
    let x = match side {
        QuickSide::Left => position.x - width + overlap,
        QuickSide::Right => main_right - overlap,
    };
    let y = (position.y + (size.height as i32 - height) / 2)
        .clamp(area_top, (area_bottom - height).max(area_top));
    drop(state);

    quick.set_size(PhysicalSize::new(width as u32, height as u32))?;
    quick.set_position(PhysicalPosition::new(x, y))?;
    quick.show()?;
    quick.emit("quick-side", side.as_str())?;
    Ok(())
}

#[tauri::command]
pub fn get_quick_side(state: tauri::State<'_, Mutex<QuickState>>) -> String {
    state.lock().expect("快捷标签位置状态不可用").side.as_str().into()
}

#[tauri::command]
pub fn set_quick_expanded(app: tauri::AppHandle, expanded: bool) -> Result<(), String> {
    app.state::<Mutex<QuickState>>()
        .lock()
        .map_err(|_| "快捷标签状态不可用".to_string())?
        .expanded = expanded;
    sync_quick(&app).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_quick_content_count(app: tauri::AppHandle, count: usize) -> Result<(), String> {
    app.state::<Mutex<QuickState>>()
        .lock()
        .map_err(|_| "快捷标签状态不可用".to_string())?
        .pin_count = count;
    sync_quick(&app).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_quick_content_height(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    if !height.is_finite() { return Err("快捷标签高度无效".into()); }
    app.state::<Mutex<QuickState>>()
        .lock().map_err(|_| "快捷标签状态不可用".to_string())?
        .content_height = height.clamp(58.0, 540.0);
    sync_quick(&app).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_quick_drag_active(app: tauri::AppHandle, active: bool) -> Result<(), String> {
    app.state::<Mutex<QuickState>>()
        .lock().map_err(|_| "快捷标签状态不可用".to_string())?
        .drag_active = active;
    sync_quick(&app).map_err(|error| error.to_string())
}
