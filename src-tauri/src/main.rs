#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod window_native;
mod native_dialog;

mod sensevoice;
mod voice_reuse;
mod sensevoice_env;
mod recording_flow;
mod sensevoice_install;
use recording_flow::*;
use sensevoice_install::install_sensevoice;

mod quick_window;
mod database;
mod sync_store;
mod cloud;
mod data_root;
mod shell;
mod news;
mod backup;
mod auto_backup;
mod document_export;
mod document_formats;
mod document_docx;
mod document_import;
mod news_collect;
mod calendar;
mod free_api;
mod juya;
mod model_news;
use model_news::{get_model_news,refresh_model_news,model_news_task_status,cancel_model_news};
mod audio;
mod audio_segments;
mod audio_summary;
mod audio_workspace;
mod recording_delete;
use recording_delete::delete_recording;
use audio_segments::{get_recording,retry_audio_segment,reset_transcription_job};
use audio_workspace::{list_recording_headers,save_recording_text,play_recording_at};
mod recording_capture;
use recording_capture::*;
mod online;
use online::*;
use audio::*;
use backup::{apply_backup_media,backup_entries,choose_backup_path,export_backup,preview_backup,import_backup};
use calendar::{get_holidays,refresh_holidays};
use free_api::{get_free_api_cache,refresh_free_api};
use shell::apply_backup_window;
use shell::{get_shell_settings,set_shell_setting,set_shell_busy,get_shell_diagnostics,reveal_shell};
use news::{get_news_cache,get_news_update_status,refresh_news,open_news_link};

use std::sync::Mutex;
use database::{cancel_reminder, complete_reminder, completed_reminder_count, delete_item_forever, get_item, list_items, list_pins, list_reminders, list_trashed, move_item, pin_item, restore_item, save_item, save_reminder, set_item_pinned, trash_item, unpin_item};
use quick_window::{create_quick, get_quick_side, set_quick_content_count, set_quick_content_height, set_quick_drag_active, set_quick_expanded, sync_quick, QuickState};

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, WindowEvent,
};

#[repr(C)]
struct CursorPoint { x: i32, y: i32 }

#[link(name = "user32")]
extern "system" {
    fn GetCursorPos(point: *mut CursorPoint) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
}

#[tauri::command]
fn complete_quick_drag(id: String, app: tauri::AppHandle) -> Result<bool, String> {
    // Esc 会在鼠标仍按住时结束原生拖动；此时即使指针位于标签上也必须视为取消。
    if unsafe { GetAsyncKeyState(0x01) } < 0 { return Ok(false); }
    let quick = app.get_webview_window("quick").ok_or("快捷标签窗口不存在")?;
    let position = quick.outer_position().map_err(|error| error.to_string())?;
    let size = quick.outer_size().map_err(|error| error.to_string())?;
    let mut cursor = CursorPoint { x: 0, y: 0 };
    // Windows WebView2 跨窗口拖放有时不向目标页发送 drop；以鼠标释放位置判定外侧目标。
    if unsafe { GetCursorPos(&mut cursor) } == 0 { return Err("无法读取鼠标位置".into()); }
    let inside = cursor.x >= position.x && cursor.x < position.x + size.width as i32
        && cursor.y >= position.y && cursor.y < position.y + size.height as i32;
    if !inside { return Ok(false); }
    database::pin_item(id, app.state::<database::Database>())?;
    app.emit_to("quick", "quick-items-changed", true).map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
fn finish_leave(action: &str, app: tauri::AppHandle) -> Result<(), String> {
    match action {
        "hide" => {
            shell::restore(&app);
            if let Some(quick) = app.get_webview_window("quick") { window_native::hide(&quick).map_err(|e| e.to_string())?; }
            if let Some(main) = app.get_webview_window("main") { main.hide().map_err(|e| e.to_string())?; }
            Ok(())
        }
        "quit" => {
            if document_import::saving(){return Err("文档正在事务保存，请等待完成后退出".into());}
            if audio::active(){return Err("录音尚未停止，请先停止并保存音频".into());}
            if audio::processing(){return Err("录音或转写处理正在进行，请等待结果保存后退出".into());}
            if online::processing(){return Err("正式转写或纪要请求正在进行，请等待结果保存后退出".into());}
            // 可放弃的连接测试与资讯刷新先取消；异步请求会丢弃，不能把迟到响应写为成功。
            document_import::cancel();online::cancel_tests();model_news::cancel();news::cancel();app.exit(0);Ok(())
        }
        _ => Err("无效的离开操作".into()),
    }
}
#[tauri::command]
fn task_status()->serde_json::Value{serde_json::json!({"modelNews":model_news::snapshot(),"textTest":online::online_test_status("text".into()).unwrap_or_default(),"formalOnline":online::task_snapshot(),"audio":audio::task_snapshot(),"recording":audio::active()})}

fn show_main(app: &tauri::AppHandle) {
    shell::restore(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        shell::reassert(app);
        let _ = sync_quick(app);
    }
}

fn main() {
    let Some(instance)=shell::single_instance().expect("无法建立轻笺单实例") else{return};
    let base_root=data_root::resolve().expect("无法定位轻笺数据目录");
    // 账号工作区在创建数据库与 WebView 前选定，切换必须重启，避免旧缓存串号。
    let (cloud_state,data_root)=cloud::initialize(&base_root).expect("无法定位账号工作区");
    // 将本进程及其子进程可控制的临时文件也引导到选定的数据根目录。
    let temp=data_root.join("temp");
    std::fs::create_dir_all(&temp).expect("无法创建轻笺临时目录");
    std::env::set_var("TEMP",&temp);
    std::env::set_var("TMP",&temp);
    tauri::Builder::default()
        .manage(instance)
        .manage(cloud_state)
        .manage(Mutex::new(QuickState::default()))
        .invoke_handler(tauri::generate_handler![cloud::cloud_status,cloud::cloud_configure,cloud::cloud_send_code,cloud::cloud_verify,cloud::cloud_logout,cloud::cloud_restart,cloud::cloud_link_local,cloud::cloud_sync,cloud::cloud_resolve_conflict,auto_backup::test_auto_backup_tick,auto_backup::backup_preferences,auto_backup::auto_backup_status,auto_backup::configure_auto_backup,auto_backup::backup_now,auto_backup::open_backup_folder,document_export::choose_export_folder,document_export::export_documents,document_import::choose_document_files,document_import::preview_documents,document_import::remap_document_json,document_import::cancel_document_import,document_import::commit_documents,news_collect::collect_news,window_native::begin_grip_drag,shell::toggle_shell_maximize,shell::set_background_transparency,delete_recording,list_recording_headers,get_recording,retry_audio_segment,reset_transcription_job,save_recording_text,play_recording_at,pause_recording,get_model_news,refresh_model_news,model_news_task_status,cancel_model_news,task_status,preview_audio_inputs,audio_devices,get_recording_inputs,save_recording_inputs,transcribe_recording,set_recording_route,save_recording_audio,open_recording_folder,discard_recording_audio,install_sensevoice,online_config,save_online_config,test_online_model,online_test_status,cancel_online_test,process_online,audio_config,save_audio_config,choose_audio_path,start_recording,stop_recording,recording_status,list_recordings,play_recording,stop_playback,transcribe_local,check_local_model,get_shell_settings,set_shell_setting,set_shell_busy,get_shell_diagnostics,reveal_shell,get_news_cache,get_news_update_status,refresh_news,open_news_link,quick_window::set_quick_menu_size,quick_window::set_quick_compact_width,quick_window::set_quick_hover_width,get_quick_side, set_quick_expanded, set_quick_content_count, set_quick_content_height, set_quick_drag_active, list_items, list_trashed, save_item, get_item, list_pins, pin_item, unpin_item, trash_item, restore_item, delete_item_forever, set_item_pinned, move_item, list_reminders, save_reminder, cancel_reminder, complete_reminder, completed_reminder_count, complete_quick_drag, finish_leave,apply_backup_media,backup_entries,choose_backup_path,export_backup,preview_backup,import_backup,get_holidays,refresh_holidays,get_free_api_cache,refresh_free_api,apply_backup_window])
        .setup(move |app| {
            let main=tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::App("index.html".into()))
                .title("轻笺 V1").inner_size(327.0,720.0).min_inner_size(326.0,480.0)
                .decorations(false).transparent(true).background_color(tauri::utils::config::Color(0,0,0,0)).resizable(true).visible(false)
                .data_directory(data_root.join("webview")).build()?;
            let _=main;
            app.manage(data_root::DataRoot(data_root.clone()));
            let (database, path) = database::open(&data_root).map_err(std::io::Error::other)?;
            if data_root!=base_root {sync_store::install(&*database.0.lock().map_err(|_|std::io::Error::other("数据库不可用"))?).map_err(std::io::Error::other)?;}
            app.manage(database);
            recording_delete::recover_pending(app.handle()).map_err(std::io::Error::other)?;
            recording_capture::recover_sessions(app.handle()).map_err(std::io::Error::other)?;
            eprintln!("轻笺数据文件：{}", path.display());
            create_quick(app)?;
            shell::initialize(app.handle()).map_err(std::io::Error::other)?;
            auto_backup::start(app.handle().clone());
            cloud::start(app.handle().clone());
            news::start_background(app.handle().clone());
            show_main(app.handle());
            model_news::start_background(app.handle().clone());
            let open = MenuItem::with_id(app, "open", "打开轻笺", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            // 托盘退出也先交给前端处理未保存草稿。
            TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("缺少应用图标").clone())
                .tooltip("轻笺 V1")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => {
                        show_main(app);
                        let _ = app.emit_to("main", "shell-leave-request", "quit");
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        // 无论标题栏还是系统关闭，都先询问未保存草稿。
                        api.prevent_close();
                        let _ = window.emit("shell-leave-request", "hide");
                    }
                    WindowEvent::Moved(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. } => {
                        let _ = sync_quick(&window.app_handle());
                    }
                    _ => {}
                }
            } else if window.label() == "quick" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("轻笺启动失败");
}
