use crate::database::Database;
use chrono::{NaiveDate, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Read, sync::Mutex, time::Duration};
use tauri::Manager;

const API: &str = "https://aihot.virxact.com/api/public/daily";
static FETCH_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    category: String, title: String, summary: String, source: String,
    source_url: Option<String>, permalink: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Daily { date: String, fetched_at: String, source: String, items: Vec<Article> }

fn text(v: &Value, key: &str) -> String { v[key].as_str().unwrap_or("").trim().to_string() }
fn safe_url(s: &str) -> Option<String> {
    reqwest::Url::parse(s).ok().filter(|u| matches!(u.scheme(), "https" | "http") && u.host_str().is_some()).map(|u| u.to_string())
}
fn normalize(raw: Value) -> Result<Daily, String> {
    let date = text(&raw, "date");
    if date.len() != 10 || NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() { return Err("新闻响应缺少有效数据日期".into()); }
    let mut items = Vec::new();
    for section in raw["sections"].as_array().ok_or("新闻响应缺少 sections 数组")? {
        for item in section["items"].as_array().ok_or("新闻分组的 items 格式无效")? {
            let title = text(item, "title");
            if title.is_empty() { continue; }
            items.push(Article { category: text(section,"label"), title, summary: text(item,"summary"),
                source: text(item,"sourceName"), source_url: safe_url(&text(item,"sourceUrl")), permalink: safe_url(&text(item,"permalink")) });
        }
    }
    if items.is_empty() { return Err("新闻响应没有可展示的真实条目".into()); }
    Ok(Daily { date, fetched_at: Utc::now().to_rfc3339(), source: text(&raw["attribution"],"source"), items })
}

#[tauri::command]
pub fn get_news_cache(database: tauri::State<'_, Database>) -> Result<Option<Daily>, String> {
    let db = database.0.lock().map_err(|_| "数据库暂不可用")?;
    let value: Option<String> = db.query_row("SELECT payload FROM news_cache WHERE channel='ai'", [], |r| r.get(0)).optional().map_err(|e|e.to_string())?;
    value.map(|value| {
        let daily: Daily = serde_json::from_str(&value).map_err(|_| "新闻缓存损坏，正在尝试重新获取")?;
        if NaiveDate::parse_from_str(&daily.date,"%Y-%m-%d").is_err() || chrono::DateTime::parse_from_rfc3339(&daily.fetched_at).is_err() || daily.items.is_empty() || daily.items.iter().any(|i|i.title.trim().is_empty()) {
            return Err("新闻缓存内容无效，正在尝试重新获取".into());
        }
        Ok(daily)
    }).transpose()
}

#[tauri::command]
pub async fn refresh_news(app: tauri::AppHandle) -> Result<Daily, String> {
    // 网络请求在阻塞工作线程完成，不占用 UI 线程和内容数据库锁。
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = FETCH_LOCK.try_lock().map_err(|_| "新闻正在获取，请稍候")?;
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).connect_timeout(Duration::from_secs(8)).build().map_err(|e|e.to_string())?;
        let response = client.get(API).header("User-Agent", "LightNote/0.4 (local Windows desktop app)").header("Accept","application/json").send()
            .map_err(|e| if e.is_timeout() {"新闻获取超时，请稍后重试".into()} else {format!("无法连接新闻源，请检查网络：{e}")})?;
        if !response.status().is_success() {
            return Err(if response.status().as_u16() == 429 {"请求过于频繁（HTTP 429），请稍后刷新".into()} else {format!("新闻源返回 HTTP {}",response.status().as_u16())});
        }
        let mut bytes = Vec::new();
        response.take(2_000_001).read_to_end(&mut bytes).map_err(|e|format!("读取新闻失败：{e}"))?;
        if bytes.len() > 2_000_000 { return Err("新闻响应过大，已拒绝读取".into()); }
        let raw: Value = serde_json::from_slice(&bytes).map_err(|_| "新闻源返回了无法识别的 JSON")?;
        let daily = normalize(raw)?;
        let state = app.state::<Database>();
        let db = state.0.lock().map_err(|_| "数据库暂不可用")?;
        // 只有完整验证成功的内容才替换缓存；网络或解析失败不影响上一份成功记录。
        db.execute("INSERT INTO news_cache(channel,payload) VALUES ('ai',?1) ON CONFLICT(channel) DO UPDATE SET payload=excluded.payload", [serde_json::to_string(&daily).map_err(|e|e.to_string())?]).map_err(|e|format!("新闻缓存保存失败：{e}"))?;
        Ok(daily)
    }).await.map_err(|e|format!("新闻后台任务失败：{e}"))?
}

#[link(name="shell32")]
extern "system" { fn ShellExecuteW(hwnd: *mut std::ffi::c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize; }
#[tauri::command]
pub fn open_news_link(url: String) -> Result<(), String> {
    let url = safe_url(&url).ok_or("链接无效，只能打开 HTTP/HTTPS 原文")?;
    let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe { ShellExecuteW(std::ptr::null_mut(),std::ptr::null(),wide.as_ptr(),std::ptr::null(),std::ptr::null(),1) };
    if result <= 32 { Err(format!("无法打开浏览器，Windows 错误 {result}")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn rejects_invalid_response_without_inventing_data() {
        assert!(normalize(serde_json::json!({"date":"2026-09-26","sections":[]})).is_err());
        assert!(safe_url("javascript:alert(1)").is_none());
    }
    #[test] fn preserves_returned_date_and_missing_source() {
        let d=normalize(serde_json::json!({"date":"2026-09-25","sections":[{"items":[{"title":"真实标题"}]}]})).unwrap();
        assert_eq!(d.date,"2026-09-25"); assert!(d.items[0].source.is_empty());
    }
}
