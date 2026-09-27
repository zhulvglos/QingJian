use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::State;
use uuid::Uuid;

pub struct Database(pub Mutex<Connection>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    id: String,
    kind: String,
    title: String,
    body: String,
    body_json: Option<String>,
    created_at: String,
    updated_at: String,
    revision: i64,
    is_pinned: bool,
    sort_order: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveInput {
    id: Option<String>,
    kind: String,
    title: String,
    body: String,
    body_json: Option<String>,
    revision: Option<i64>,
}

fn row_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: row.get(0)?, kind: row.get(1)?, title: row.get(2)?, body: row.get(3)?,
        body_json: row.get(4)?, created_at: row.get(5)?, updated_at: row.get(6)?, revision: row.get(7)?,
        is_pinned: row.get(8)?, sort_order: row.get(9)?,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedItem {
    item_id: String,
    kind: String,
    title: String,
    sort_order: i64,
}

fn save_error(error: rusqlite::Error) -> String {
    if let rusqlite::Error::SqliteFailure(code, _) = &error {
        if code.code == rusqlite::ErrorCode::DatabaseBusy || code.code == rusqlite::ErrorCode::DatabaseLocked {
            return "数据库正在被占用，请稍后重试".into();
        }
    }
    format!("数据库操作失败：{error}")
}

pub(crate) fn migrate(conn: &mut Connection) -> Result<(), String> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if version > 5 { return Err(format!("数据库版本 {version} 高于当前程序支持的版本 5")); }
    if version == 0 {
        // 版本号与建表在同一事务中提交，失败时不会留下半套结构。
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK(kind IN ('sticky','note')),
                title TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL DEFAULT '',
                category_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT,
                revision INTEGER NOT NULL DEFAULT 1,
                origin TEXT NOT NULL DEFAULT 'native',
                origin_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_items_kind_updated ON items(kind, deleted_at, updated_at DESC);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_items_origin ON items(origin, origin_id) WHERE origin_id IS NOT NULL;
            INSERT INTO schema_migrations(version, applied_at) VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ','now'));
            PRAGMA user_version = 1;").map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if version == 1 {
        // 旧版正文仍保留在 body；NULL 表示尚未转换的纯文本，不在迁移时改写内容。
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("ALTER TABLE items ADD COLUMN body_json TEXT;
            CREATE TABLE pins (
                item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
                sort_order INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX idx_pins_order ON pins(sort_order, created_at);
            INSERT INTO schema_migrations(version, applied_at) VALUES (2, strftime('%Y-%m-%dT%H:%M:%fZ','now'));
            PRAGMA user_version = 2;").map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if version == 2 {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // 只新增排序、提醒表；旧正文、时间和固定关系均原样保留。
        tx.execute_batch("ALTER TABLE items ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0 CHECK(is_pinned IN (0,1));
            ALTER TABLE items ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
            CREATE INDEX idx_items_list_order ON items(kind,deleted_at,is_pinned DESC,sort_order);
            CREATE TABLE reminders (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL UNIQUE REFERENCES items(id) ON DELETE CASCADE,
                enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
                repeat_rule TEXT NOT NULL DEFAULT 'once',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE reminder_occurrences (
                id TEXT PRIMARY KEY,
                reminder_id TEXT NOT NULL REFERENCES reminders(id) ON DELETE CASCADE,
                due_at TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('pending','completed','cancelled')),
                completed_at TEXT,
                is_current INTEGER NOT NULL CHECK(is_current IN (0,1)),
                created_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX idx_reminder_current ON reminder_occurrences(reminder_id) WHERE is_current=1;
            CREATE INDEX idx_reminder_due ON reminder_occurrences(status,due_at);").map_err(|e| e.to_string())?;
        let ordered: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id,kind FROM items ORDER BY kind,updated_at DESC,id DESC").map_err(save_error)?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).map_err(save_error)?
                .collect::<Result<Vec<_>, _>>().map_err(save_error)?;
            rows
        };
        let mut last_kind = String::new();
        let mut rank = 0_i64;
        for (id, kind) in ordered {
            if kind != last_kind { rank = 0; last_kind = kind; }
            tx.execute("UPDATE items SET sort_order=?1 WHERE id=?2", params![rank, id]).map_err(save_error)?;
            rank += 1;
        }
        tx.execute_batch("INSERT INTO schema_migrations(version,applied_at) VALUES (3,strftime('%Y-%m-%dT%H:%M:%fZ','now'));
            PRAGMA user_version = 3;").map_err(save_error)?;
        tx.commit().map_err(save_error)?;
    }
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(save_error)?;
    if version == 3 {
        let tx = conn.transaction().map_err(save_error)?;
        tx.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE news_cache(channel TEXT PRIMARY KEY,payload TEXT NOT NULL);
            INSERT INTO schema_migrations(version,applied_at) VALUES (4,strftime('%Y-%m-%dT%H:%M:%fZ','now'));
            PRAGMA user_version=4;").map_err(save_error)?;
        tx.commit().map_err(save_error)?;
    }
    migrate_audio(conn)
}

fn migrate_audio(conn:&mut Connection)->Result<(),String>{
    let version:i64=conn.query_row("PRAGMA user_version",[],|r|r.get(0)).map_err(save_error)?;
    if version==4 {let tx=conn.transaction().map_err(save_error)?;
        // 会议记录独立建表，不重写现有内容。
        tx.execute_batch("CREATE TABLE audio_sessions(id TEXT PRIMARY KEY,created_at TEXT NOT NULL,payload TEXT NOT NULL); INSERT INTO schema_migrations VALUES(5,strftime('%Y-%m-%dT%H:%M:%fZ','now')); PRAGMA user_version=5;").map_err(save_error)?;tx.commit().map_err(save_error)?;}
    Ok(())
}
pub fn open() -> Result<(Database, PathBuf), String> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or("无法确定当前用户的数据目录 LOCALAPPDATA")?;
    let directory = PathBuf::from(base).join("SHUSHIN").join("Qingjian");
    std::fs::create_dir_all(&directory).map_err(|e| format!("创建数据目录失败：{e}"))?;
    let path = directory.join("qingjian-v1.sqlite3");
    let mut conn = Connection::open(&path).map_err(|e| format!("打开数据库失败：{e}"))?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys = ON;").map_err(|e| e.to_string())?;
    migrate(&mut conn)?;
    Ok((Database(Mutex::new(conn)), path))
}

#[tauri::command]
pub fn list_items(kind: String, database: State<'_, Database>) -> Result<Vec<Item>, String> {
    if kind != "sticky" && kind != "note" { return Err("内容类型无效".into()); }
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let mut stmt = conn.prepare("SELECT id,kind,title,body,body_json,created_at,updated_at,revision,is_pinned,sort_order FROM items WHERE kind=?1 AND deleted_at IS NULL ORDER BY is_pinned DESC,sort_order,id").map_err(save_error)?;
    let rows = stmt.query_map([kind], row_item).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_trashed(kind: String, database: State<'_, Database>) -> Result<Vec<Item>, String> {
    if kind != "sticky" && kind != "note" { return Err("内容类型无效".into()); }
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let mut stmt = conn.prepare("SELECT id,kind,title,body,body_json,created_at,updated_at,revision,is_pinned,sort_order FROM items WHERE kind=?1 AND deleted_at IS NOT NULL ORDER BY deleted_at DESC,id DESC").map_err(save_error)?;
    let rows = stmt.query_map([kind], row_item).map_err(save_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(save_error)
}

#[tauri::command]
pub fn trash_item(id: String, revision: i64, database: State<'_, Database>) -> Result<(), String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    // 软删除只改变 deleted_at；固定关系保留，普通列表与快捷标签查询会暂时隐藏它。
    let changed = conn.execute("UPDATE items SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),revision=revision+1 WHERE id=?1 AND revision=?2 AND deleted_at IS NULL", params![id, revision]).map_err(save_error)?;
    if changed != 1 { return Err("删除失败：条目已变化或不存在，请重新读取".into()); }
    Ok(())
}

#[tauri::command]
pub fn restore_item(id: String, database: State<'_, Database>) -> Result<(), String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let changed = conn.execute("UPDATE items SET deleted_at=NULL,revision=revision+1 WHERE id=?1 AND deleted_at IS NOT NULL", [&id]).map_err(save_error)?;
    if changed != 1 { return Err("恢复失败：条目不在回收站".into()); }
    Ok(())
}

#[tauri::command]
pub fn delete_item_forever(id: String, database: State<'_, Database>) -> Result<(), String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    // 外键级联移除快捷固定；仅允许删除已在回收站的条目。
    let changed = conn.execute("DELETE FROM items WHERE id=?1 AND deleted_at IS NOT NULL", [&id]).map_err(save_error)?;
    if changed != 1 { return Err("永久删除失败：请先将条目移入回收站".into()); }
    Ok(())
}

#[tauri::command]
pub fn save_item(input: SaveInput, database: State<'_, Database>) -> Result<Item, String> {
    if input.kind != "sticky" && input.kind != "note" { return Err("内容类型无效".into()); }
    if input.title.trim().is_empty() && input.body.trim().is_empty() { return Err("请输入标题或正文后再保存".into()); }
    if let Some(document) = &input.body_json {
        if document.len() > 2_000_000 { return Err("正文过长，请缩短后再保存".into()); }
        let value: serde_json::Value = serde_json::from_str(document).map_err(|_| "正文格式无效，草稿仍保留")?;
        let valid = value.get("format").and_then(|value| value.as_str()) == Some("qingjian-rich-v1")
            && matches!(value.get("mode").and_then(|value| value.as_str()), Some("paragraph" | "checklist"))
            && value.get("document").and_then(|value| value.get("type")).and_then(|value| value.as_str()) == Some("doc")
            && value.get("checks").and_then(|value| value.as_array()).is_some_and(|checks| checks.iter().all(|item| item.is_boolean()));
        if !valid { return Err("正文格式无效，草稿仍保留".into()); }
    }
    let mut conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(save_error)?;
    let id = if let Some(id) = input.id {
        let revision = input.revision.ok_or("缺少内容版本，请重新打开该条目")?;
        // 只更新当前版本，避免其他窗口或进程先保存时被静默覆盖。
        let changed = tx.execute("UPDATE items SET title=?1,body=?2,body_json=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),revision=revision+1 WHERE id=?4 AND kind=?5 AND revision=?6 AND deleted_at IS NULL",
            params![input.title, input.body, input.body_json, id, input.kind, revision]).map_err(save_error)?;
        if changed != 1 { return Err("保存失败：条目已变化或不存在，请保留草稿并重新检查".into()); }
        id
    } else {
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO items(id,kind,title,body,body_json,created_at,updated_at,sort_order) VALUES (?1,?2,?3,?4,?5,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),(SELECT COALESCE(MIN(sort_order),0)-1 FROM items WHERE kind=?2 AND is_pinned=0))",
            params![id, input.kind, input.title, input.body, input.body_json]).map_err(save_error)?;
        id
    };
    let item = tx.query_row("SELECT id,kind,title,body,body_json,created_at,updated_at,revision,is_pinned,sort_order FROM items WHERE id=?1", [&id], row_item).optional().map_err(save_error)?.ok_or("保存后未找到条目")?;
    tx.commit().map_err(save_error)?;
    Ok(item)
}

#[tauri::command]
pub fn get_item(id: String, database: State<'_, Database>) -> Result<Item, String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    conn.query_row("SELECT id,kind,title,body,body_json,created_at,updated_at,revision,is_pinned,sort_order FROM items WHERE id=?1 AND deleted_at IS NULL", [&id], row_item)
        .optional().map_err(save_error)?.ok_or("原内容不存在或已删除".into())
}

#[tauri::command]
pub fn list_pins(database: State<'_, Database>) -> Result<Vec<PinnedItem>, String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let mut stmt = conn.prepare("SELECT p.item_id,i.kind,i.title,p.sort_order FROM pins p JOIN items i ON i.id=p.item_id WHERE i.deleted_at IS NULL ORDER BY p.sort_order,p.created_at").map_err(save_error)?;
    let rows = stmt.query_map([], |row| Ok(PinnedItem { item_id: row.get(0)?, kind: row.get(1)?, title: row.get(2)?, sort_order: row.get(3)? })).map_err(save_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(save_error)
}

#[tauri::command]
pub fn pin_item(id: String, database: State<'_, Database>) -> Result<(), String> {
    let mut conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(save_error)?;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM items WHERE id=?1 AND deleted_at IS NULL)", [&id], |row| row.get(0)).map_err(save_error)?;
    if !exists { return Err("无法固定：原内容不存在".into()); }
    // 唯一主键保证重复拖入不会生成第二个快捷标签。
    tx.execute("INSERT OR IGNORE INTO pins(item_id,sort_order,created_at) VALUES (?1,(SELECT COALESCE(MAX(sort_order),0)+1 FROM pins),strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [&id]).map_err(save_error)?;
    tx.commit().map_err(save_error)
}

#[tauri::command]
pub fn unpin_item(id: String, database: State<'_, Database>) -> Result<(), String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    conn.execute("DELETE FROM pins WHERE item_id=?1", [&id]).map_err(save_error)?;
    Ok(())
}

#[tauri::command]
pub fn set_item_pinned(id: String, pinned: bool, database: State<'_, Database>) -> Result<(), String> {
    let mut conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(save_error)?;
    let kind: String = tx.query_row("SELECT kind FROM items WHERE id=?1 AND deleted_at IS NULL", [&id], |row| row.get(0))
        .optional().map_err(save_error)?.ok_or("内容不存在或已删除")?;
    let next: i64 = tx.query_row("SELECT COALESCE(MIN(sort_order),0)-1 FROM items WHERE kind=?1 AND is_pinned=?2 AND deleted_at IS NULL", params![kind, pinned], |row| row.get(0)).map_err(save_error)?;
    // 置顶只修改本模块列表顺序，不触碰正文更新时间和外侧快捷固定。
    tx.execute("UPDATE items SET is_pinned=?1,sort_order=?2 WHERE id=?3", params![pinned, next, id]).map_err(save_error)?;
    tx.commit().map_err(save_error)
}

#[tauri::command]
pub fn move_item(id: String, target_id: String, database: State<'_, Database>) -> Result<(), String> {
    if id == target_id { return Ok(()); }
    let mut conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(save_error)?;
    let source: (String, bool, i64) = tx.query_row("SELECT kind,is_pinned,sort_order FROM items WHERE id=?1 AND deleted_at IS NULL", [&id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional().map_err(save_error)?.ok_or("待移动条目不存在")?;
    let target: (String, bool, i64) = tx.query_row("SELECT kind,is_pinned,sort_order FROM items WHERE id=?1 AND deleted_at IS NULL", [&target_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional().map_err(save_error)?.ok_or("目标条目不存在")?;
    if source.0 != target.0 || source.1 != target.1 { return Err("只能在同模块、同置顶分组内移动".into()); }
    tx.execute("UPDATE items SET sort_order=?1 WHERE id=?2", params![target.2, id]).map_err(save_error)?;
    tx.execute("UPDATE items SET sort_order=?1 WHERE id=?2", params![source.2, target_id]).map_err(save_error)?;
    tx.commit().map_err(save_error)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderView {
    occurrence_id: String,
    item_id: String,
    kind: String,
    title: String,
    due_at: String,
    status: String,
    completed_at: Option<String>,
    is_current: bool,
}

#[tauri::command]
pub fn list_reminders(database: State<'_, Database>) -> Result<Vec<ReminderView>, String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let mut stmt = conn.prepare("SELECT o.id,i.id,i.kind,i.title,o.due_at,o.status,o.completed_at,o.is_current
        FROM reminder_occurrences o JOIN reminders r ON r.id=o.reminder_id JOIN items i ON i.id=r.item_id
        WHERE i.deleted_at IS NULL AND (o.status='completed' OR (r.enabled=1 AND o.is_current=1 AND o.status='pending'))
        ORDER BY o.due_at DESC,i.id").map_err(save_error)?;
    let rows = stmt.query_map([], |row| Ok(ReminderView { occurrence_id: row.get(0)?, item_id: row.get(1)?, kind: row.get(2)?, title: row.get(3)?, due_at: row.get(4)?, status: row.get(5)?, completed_at: row.get(6)?, is_current: row.get(7)? }))
        .map_err(save_error)?.collect::<Result<Vec<_>, _>>().map_err(save_error)?;
    Ok(rows)
}

#[tauri::command]
pub fn save_reminder(item_id: String, due_at: String, database: State<'_, Database>) -> Result<(), String> {
    if !due_at.ends_with('Z') { return Err("提醒时间必须包含明确时区".into()); }
    let mut conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(save_error)?;
    let valid: bool = tx.query_row("SELECT strftime('%s',?1) IS NOT NULL", [&due_at], |row| row.get(0)).map_err(save_error)?;
    if !valid { return Err("提醒时间无效".into()); }
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM items WHERE id=?1 AND deleted_at IS NULL)", [&item_id], |row| row.get(0)).map_err(save_error)?;
    if !exists { return Err("请先保存便签或笔记，再设置提醒".into()); }
    let previous: Option<String> = tx.query_row("SELECT id FROM reminders WHERE item_id=?1", [&item_id], |row| row.get(0)).optional().map_err(save_error)?;
    let reminder_id = if let Some(id) = previous {
        tx.execute("UPDATE reminders SET enabled=1,repeat_rule='once',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1", [&id]).map_err(save_error)?;
        // 修改提醒只结束旧待办的当前身份；已完成历史行不删除。
        tx.execute("UPDATE reminder_occurrences SET is_current=0,status=CASE WHEN status='pending' THEN 'cancelled' ELSE status END WHERE reminder_id=?1 AND is_current=1", [&id]).map_err(save_error)?;
        id
    } else {
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO reminders(id,item_id,created_at,updated_at) VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![id, item_id]).map_err(save_error)?;
        id
    };
    let occurrence_id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO reminder_occurrences(id,reminder_id,due_at,status,is_current,created_at) VALUES (?1,?2,?3,'pending',1,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![occurrence_id, reminder_id, due_at]).map_err(save_error)?;
    tx.commit().map_err(save_error)
}

#[tauri::command]
pub fn cancel_reminder(item_id: String, database: State<'_, Database>) -> Result<(), String> {
    let mut conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(save_error)?;
    tx.execute("UPDATE reminder_occurrences SET is_current=0,status=CASE WHEN status='pending' THEN 'cancelled' ELSE status END WHERE reminder_id=(SELECT id FROM reminders WHERE item_id=?1) AND is_current=1", [&item_id]).map_err(save_error)?;
    tx.execute("UPDATE reminders SET enabled=0,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE item_id=?1", [&item_id]).map_err(save_error)?;
    tx.commit().map_err(save_error)
}

#[tauri::command]
pub fn complete_reminder(occurrence_id: String, database: State<'_, Database>) -> Result<(), String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    let changed = conn.execute("UPDATE reminder_occurrences SET status='completed',completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status='pending' AND is_current=1 AND EXISTS(SELECT 1 FROM reminders r JOIN items i ON i.id=r.item_id WHERE r.id=reminder_occurrences.reminder_id AND r.enabled=1 AND i.deleted_at IS NULL)", [&occurrence_id]).map_err(save_error)?;
    if changed != 1 { return Err("本次提醒已变化，请重新读取".into()); }
    Ok(())
}

#[tauri::command]
pub fn completed_reminder_count(item_id: String, database: State<'_, Database>) -> Result<i64, String> {
    let conn = database.0.lock().map_err(|_| "数据库暂时不可用")?;
    conn.query_row("SELECT COUNT(*) FROM reminder_occurrences o JOIN reminders r ON r.id=o.reminder_id WHERE r.item_id=?1 AND o.status='completed'", [&item_id], |row| row.get(0)).map_err(save_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_creates_versioned_empty_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0)).unwrap();
        assert_eq!((version, count), (5, 0));
    }
    #[test]
    fn v1_plain_text_survives_migration() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);
            CREATE TABLE items(id TEXT PRIMARY KEY,kind TEXT NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL,category_id TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,deleted_at TEXT,revision INTEGER NOT NULL,origin TEXT NOT NULL,origin_id TEXT);
            INSERT INTO items VALUES ('old-id','sticky','旧标题','第一行\n第二行',NULL,'2026-01-01','2026-01-01',NULL,3,'native',NULL);
            INSERT INTO schema_migrations VALUES (1,'2026-01-01'); PRAGMA user_version=1;").unwrap();
        migrate(&mut conn).unwrap();
        let row: (String, String, Option<String>, i64) = conn.query_row("SELECT id,body,body_json,revision FROM items", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(row, ("old-id".into(), "第一行\n第二行".into(), None, 3));
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }
    #[test]
    fn v2_content_and_quick_pins_survive_v3_migration() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);
            CREATE TABLE items(id TEXT PRIMARY KEY,kind TEXT NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL,category_id TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,deleted_at TEXT,revision INTEGER NOT NULL,origin TEXT NOT NULL,origin_id TEXT,body_json TEXT);
            CREATE TABLE pins(item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,sort_order INTEGER NOT NULL,created_at TEXT NOT NULL);
            INSERT INTO items VALUES ('old-id','sticky','原标题','原正文',NULL,'2026-01-01','2026-01-02',NULL,3,'native',NULL,NULL);
            INSERT INTO pins VALUES ('old-id',1,'2026-01-03');
            INSERT INTO schema_migrations VALUES (2,'2026-01-01'); PRAGMA user_version=2;").unwrap();
        migrate(&mut conn).unwrap();
        let row: (String, String, i64, i64) = conn.query_row("SELECT title,body,revision,is_pinned FROM items WHERE id='old-id'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(row, ("原标题".into(), "原正文".into(), 3, 0));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pins WHERE item_id='old-id'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }
}
