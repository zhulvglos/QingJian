//! 同步账本与业务内容一起提交。云端版本只来自服务器，本地 revision 仅保护编辑器。
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

fn err(e: impl std::fmt::Display) -> String { e.to_string() }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Document {
    pub id: String, pub kind: String, pub title: String, pub body: String,
    pub body_json: Option<String>, pub created_at: String, pub updated_at: String,
    pub deleted_at: Option<String>, pub is_pinned: bool, pub sort_order: i64,
    #[serde(default)] pub purged: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Remote { pub version: i64, pub document: Document }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation { pub op: String, pub seq: i64, pub base: i64, pub document: Document }

pub fn install(c: &Connection) -> Result<(), String> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS sync_control(id INTEGER PRIMARY KEY CHECK(id=1), applying INTEGER NOT NULL DEFAULT 0, linked INTEGER NOT NULL DEFAULT 0, last_success TEXT);
      INSERT OR IGNORE INTO sync_control(id) VALUES(1);
      CREATE TABLE IF NOT EXISTS sync_meta(id TEXT PRIMARY KEY,base INTEGER NOT NULL DEFAULT 0,seq INTEGER NOT NULL DEFAULT 1,ack INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS sync_outbox(id TEXT PRIMARY KEY,operation TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS sync_purged(id TEXT PRIMARY KEY,document TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS sync_conflicts(copy_id TEXT PRIMARY KEY,original_id TEXT NOT NULL,remote_version INTEGER NOT NULL,resolved INTEGER NOT NULL DEFAULT 0);
      CREATE TRIGGER IF NOT EXISTS sync_insert AFTER INSERT ON items WHEN (SELECT applying FROM sync_control WHERE id=1)=0 BEGIN
        INSERT INTO sync_meta(id) VALUES(NEW.id) ON CONFLICT(id) DO UPDATE SET seq=seq+1;
        DELETE FROM sync_purged WHERE id=NEW.id;
      END;
      CREATE TRIGGER IF NOT EXISTS sync_update AFTER UPDATE OF title,body,body_json,deleted_at,is_pinned,sort_order,created_at,updated_at ON items
      WHEN (SELECT applying FROM sync_control WHERE id=1)=0 BEGIN
        INSERT INTO sync_meta(id) VALUES(NEW.id) ON CONFLICT(id) DO UPDATE SET seq=seq+1;
      END;
      CREATE TRIGGER IF NOT EXISTS sync_delete BEFORE DELETE ON items WHEN (SELECT applying FROM sync_control WHERE id=1)=0 BEGIN
        INSERT OR REPLACE INTO sync_purged VALUES(OLD.id,json_object('id',OLD.id,'kind',OLD.kind,'title','','body','','body_json',NULL,'created_at',OLD.created_at,'updated_at',OLD.updated_at,'deleted_at',COALESCE(OLD.deleted_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),'is_pinned',json(CASE WHEN OLD.is_pinned=1 THEN 'true' ELSE 'false' END),'sort_order',OLD.sort_order,'purged',json('true')));
        INSERT INTO sync_meta(id) VALUES(OLD.id) ON CONFLICT(id) DO UPDATE SET seq=seq+1;
      END;
      INSERT OR IGNORE INTO sync_meta(id) SELECT id FROM items;").map_err(err)
}
pub fn document(c: &Connection, id: &str) -> Result<Document, String> {
    let row = c.query_row("SELECT id,kind,title,body,body_json,created_at,updated_at,deleted_at,is_pinned,sort_order FROM items WHERE id=?1", [id], |r| Ok(Document {
        id:r.get(0)?,kind:r.get(1)?,title:r.get(2)?,body:r.get(3)?,body_json:r.get(4)?,created_at:r.get(5)?,updated_at:r.get(6)?,deleted_at:r.get(7)?,is_pinned:r.get(8)?,sort_order:r.get(9)?,purged:false
    })).optional().map_err(err)?;
    if let Some(d)=row {return Ok(d);}
    let raw:String=c.query_row("SELECT document FROM sync_purged WHERE id=?1",[id],|r|r.get(0)).map_err(err)?;
    serde_json::from_str(&raw).map_err(err)
}
pub fn next(c: &mut Connection) -> Result<Option<Operation>, String> {
    let tx=c.transaction().map_err(err)?;
    let raw:Option<String>=tx.query_row("SELECT operation FROM sync_outbox ORDER BY id LIMIT 1",[],|r|r.get(0)).optional().map_err(err)?;
    let op=if let Some(raw)=raw {Some(serde_json::from_str(&raw).map_err(err)?)} else {
        let m:Option<(String,i64,i64)>=tx.query_row("SELECT id,seq,base FROM sync_meta WHERE seq>ack ORDER BY id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        if let Some((id,seq,base))=m {
            let operation=Operation{op:Uuid::new_v4().to_string(),seq,base,document:document(&tx,&id)?};
            tx.execute("INSERT INTO sync_outbox VALUES(?1,?2)",params![id,serde_json::to_string(&operation).map_err(err)?]).map_err(err)?;
            Some(operation)
        }else{None}
    };
    tx.commit().map_err(err)?;Ok(op)
}
fn validate(d:&Document)->Result<(),String>{
    Uuid::parse_str(&d.id).map_err(|_|"云端内容标识无效")?;
    if d.title.len()>100_000||d.body.len()>2_000_000{return Err("云端内容超过支持大小".into());}
    if !d.purged {crate::database::validate_item_fields(&d.kind,&d.title,&d.body,d.body_json.as_deref())?;}
    for t in [&d.created_at,&d.updated_at] {chrono::DateTime::parse_from_rfc3339(t).map_err(|_|"云端时间格式无效")?;}
    if let Some(t)=&d.deleted_at{chrono::DateTime::parse_from_rfc3339(t).map_err(|_|"云端删除时间无效")?;}
    Ok(())
}
fn apply(c:&Connection,r:&Remote)->Result<(),String>{
    validate(&r.document)?;let d=&r.document;
    // 拉取不生成新的上传任务；本地修订号递增，使旧草稿保存明确失败，而非覆盖远端。
    c.execute("UPDATE sync_control SET applying=1 WHERE id=1",[]).map_err(err)?;
    if d.purged {
        c.execute("DELETE FROM items WHERE id=?1",[&d.id]).map_err(err)?;
        c.execute("INSERT OR REPLACE INTO sync_purged VALUES(?1,?2)",params![d.id,serde_json::to_string(d).map_err(err)?]).map_err(err)?;
    }else{
        c.execute("INSERT INTO items(id,kind,title,body,body_json,created_at,updated_at,deleted_at,is_pinned,sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
          ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,title=excluded.title,body=excluded.body,body_json=excluded.body_json,created_at=excluded.created_at,updated_at=excluded.updated_at,deleted_at=excluded.deleted_at,is_pinned=excluded.is_pinned,sort_order=excluded.sort_order,revision=items.revision+1",
          params![d.id,d.kind,d.title,d.body,d.body_json,d.created_at,d.updated_at,d.deleted_at,d.is_pinned,d.sort_order]).map_err(err)?;
        c.execute("DELETE FROM sync_purged WHERE id=?1",[&d.id]).map_err(err)?;
    }
    c.execute("UPDATE sync_control SET applying=0 WHERE id=1",[]).map_err(err)?;
    c.execute("INSERT INTO sync_meta(id,base,seq,ack) VALUES(?1,?2,0,0) ON CONFLICT(id) DO UPDATE SET base=excluded.base,ack=seq",params![d.id,r.version]).map_err(err)?;
    Ok(())
}
fn conflict(c:&Connection,id:&str,version:i64)->Result<(),String>{
    let d=document(c,id)?;
    // 墓碑也保留为可读副本，防止离线删除与另一设备修改互相吞掉。
    let copy=Uuid::new_v4().to_string();
    let title=format!("{}（冲突副本{}）",d.title,if d.deleted_at.is_some(){"·本机已删除"}else{""});
    c.execute("INSERT INTO items(id,kind,title,body,body_json,created_at,updated_at,is_pinned,sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![copy,d.kind,title,d.body,d.body_json,d.created_at,d.updated_at,d.is_pinned,d.sort_order]).map_err(err)?;
    c.execute("INSERT INTO sync_conflicts(copy_id,original_id,remote_version) VALUES(?1,?2,?3)",params![copy,id,version]).map_err(err)?;
    Ok(())
}
pub fn accept(c:&mut Connection,op:&Operation,ok:bool,remote:&Remote)->Result<(),String>{
    if remote.document.id!=op.document.id||remote.version<1{return Err("云端同步响应无效".into());}
    let tx=c.transaction().map_err(err)?;
    if ok {
        // 网络请求期间仍可本地保存；只确认已发送的 seq，后续修改继续排队。
        tx.execute("UPDATE sync_meta SET base=MAX(base,?1),ack=MAX(ack,?2) WHERE id=?3",params![remote.version,op.seq,op.document.id]).map_err(err)?;
    }else{
        conflict(&tx,&op.document.id,remote.version)?;
        apply(&tx,remote)?;
    }
    tx.execute("DELETE FROM sync_outbox WHERE id=?1",[&op.document.id]).map_err(err)?;
    tx.commit().map_err(err)
}
pub fn pull(c:&mut Connection,remote:&Remote)->Result<bool,String>{
    if remote.version<1{return Err("云端版本无效".into());}
    let tx=c.transaction().map_err(err)?;
    let m:Option<(i64,bool)>=tx.query_row("SELECT base,seq>ack FROM sync_meta WHERE id=?1",[&remote.document.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
    if m.is_some_and(|m|m.0>=remote.version){return Ok(false);}
    if m.is_some_and(|m|m.1){conflict(&tx,&remote.document.id,remote.version)?;}
    apply(&tx,remote)?;tx.commit().map_err(err)?;Ok(true)
}
pub fn needs_pull(c:&Connection,id:&str,version:i64)->Result<bool,String>{
    Uuid::parse_str(id).map_err(|_|"云端内容标识无效")?;
    if version<1{return Err("云端版本无效".into());}
    let base:Option<i64>=c.query_row("SELECT base FROM sync_meta WHERE id=?1",[id],|r|r.get(0)).optional().map_err(err)?;
    Ok(base.is_none_or(|base|base<version))
}
pub fn status(c:&Connection)->Result<Value,String>{
    let pending:i64=c.query_row("SELECT COUNT(*) FROM sync_meta WHERE seq>ack",[],|r|r.get(0)).map_err(err)?;
    let linked:bool=c.query_row("SELECT linked FROM sync_control WHERE id=1",[],|r|r.get(0)).map_err(err)?;
    let last:Option<String>=c.query_row("SELECT last_success FROM sync_control WHERE id=1",[],|r|r.get(0)).map_err(err)?;
    let mut s=c.prepare("SELECT copy_id,original_id FROM sync_conflicts WHERE resolved=0").map_err(err)?;
    let conflicts=s.query_map([],|r|Ok(json!({"copyId":r.get::<_,String>(0)?,"originalId":r.get::<_,String>(1)?}))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    Ok(json!({"pending":pending,"linked":linked,"lastSuccess":last,"conflicts":conflicts}))
}

#[cfg(test)] mod tests {
    use super::*;
    use std::collections::HashMap;
    fn db()->Connection{let mut c=Connection::open_in_memory().unwrap();c.execute_batch("PRAGMA foreign_keys=ON").unwrap();crate::database::migrate(&mut c).unwrap();install(&c).unwrap();c}
    fn add(c:&Connection)->String{crate::database::insert_new_item(c,"note","原始标题","原始正文",None).unwrap()}
    fn edit(c:&Connection,id:&str,body:&str){c.execute("UPDATE items SET body=?1,revision=revision+1 WHERE id=?2",params![body,id]).unwrap();}
    // 这是隔离传输夹具，不代表已部署 PostgreSQL RLS / RPC 或真实跨电脑验收。
    #[derive(Default)] struct Server{rows:HashMap<String,Remote>,receipts:HashMap<String,(bool,Remote)>}
    impl Server{
        fn push(&mut self,o:&Operation)->(bool,Remote){
            if let Some(r)=self.receipts.get(&o.op){return r.clone();}
            let old=self.rows.get(&o.document.id);
            let ok=old.map_or(o.base==0,|r|r.version==o.base&&!r.document.purged);
            let r=if ok{Remote{version:old.map_or(1,|r|r.version+1),document:o.document.clone()}}else{old.unwrap().clone()};
            if ok{self.rows.insert(o.document.id.clone(),r.clone());}
            self.receipts.insert(o.op.clone(),(ok,r.clone()));(ok,r)
        }
        fn exchange(&mut self,c:&mut Connection){while let Some(o)=next(c).unwrap(){let(ok,r)=self.push(&o);accept(c,&o,ok,&r).unwrap();}for r in self.rows.values(){pull(c,r).unwrap();}}
    }
    #[test] fn two_clients_create_edit_delete_restore_rich_pin_and_order(){
        let(mut a,mut b,mut s)=(db(),db(),Server::default());let id=add(&a);s.exchange(&mut a);s.exchange(&mut b);
        assert_eq!(document(&a,&id).unwrap(),document(&b,&id).unwrap());
        let rich=r#"{"format":"qingjian-rich-v1","mode":"paragraph","document":{"type":"doc","content":[]},"checks":[]}"#;
        b.execute("UPDATE items SET title='新标题',body='修改正文',body_json=?1,is_pinned=1,sort_order=-10 WHERE id=?2",params![rich,id]).unwrap();s.exchange(&mut b);s.exchange(&mut a);
        assert_eq!(document(&a,&id).unwrap().body_json.as_deref(),Some(rich));assert!(document(&a,&id).unwrap().is_pinned);
        a.execute("UPDATE items SET deleted_at='2026-10-10T00:00:00Z' WHERE id=?1",[&id]).unwrap();s.exchange(&mut a);s.exchange(&mut b);assert!(document(&b,&id).unwrap().deleted_at.is_some());
        b.execute("UPDATE items SET deleted_at=NULL WHERE id=?1",[&id]).unwrap();s.exchange(&mut b);s.exchange(&mut a);assert!(document(&a,&id).unwrap().deleted_at.is_none());
        assert_eq!(document(&a,&id).unwrap(),document(&b,&id).unwrap());
    }
    #[test] fn simultaneous_edit_keeps_both_versions_and_resync_deduplicates(){
        let(mut a,mut b,mut s)=(db(),db(),Server::default());let id=add(&a);s.exchange(&mut a);s.exchange(&mut b);
        edit(&a,&id,"甲修改");edit(&b,&id,"乙修改");s.exchange(&mut a);s.exchange(&mut b);s.exchange(&mut a);
        let bodies:Vec<String>={let mut stmt=b.prepare("SELECT body FROM items ORDER BY body").unwrap();stmt.query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap()};
        assert!(bodies.contains(&"甲修改".into())&&bodies.contains(&"乙修改".into()));assert_eq!(bodies.len(),2);
        for _ in 0..3{s.exchange(&mut a);s.exchange(&mut b);}assert_eq!(a.query_row("SELECT COUNT(*) FROM items",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    }
    #[test] fn lost_ack_retries_same_operation_without_duplicate_versions(){
        let mut a=db();let id=add(&a);let mut s=Server::default();let o=next(&mut a).unwrap().unwrap();let first=s.push(&o);
        let retry=next(&mut a).unwrap().unwrap();assert_eq!(o.op,retry.op);let second=s.push(&retry);assert_eq!(first.1.version,second.1.version);
        accept(&mut a,&retry,second.0,&second.1).unwrap();assert!(next(&mut a).unwrap().is_none());assert_eq!(s.rows[&id].version,1);
    }
    #[test] fn editing_while_uploading_is_not_acknowledged_early(){
        let mut a=db();let id=add(&a);let mut s=Server::default();let o=next(&mut a).unwrap().unwrap();edit(&a,&id,"发送途中保存的新正文");
        let(ok,r)=s.push(&o);accept(&mut a,&o,ok,&r).unwrap();assert_eq!(status(&a).unwrap()["pending"],1);
        s.exchange(&mut a);assert_eq!(s.rows[&id].document.body,"发送途中保存的新正文");assert_eq!(s.rows[&id].version,2);
    }
    #[test] fn purged_tombstone_defeats_stale_device_resurrection(){
        let(mut a,mut b,mut s)=(db(),db(),Server::default());let id=add(&a);s.exchange(&mut a);s.exchange(&mut b);
        a.execute("DELETE FROM items WHERE id=?1",[&id]).unwrap();s.exchange(&mut a);edit(&b,&id,"离线设备旧版本修改");s.exchange(&mut b);
        assert!(s.rows[&id].document.purged);assert!(document(&b,&id).unwrap().purged);
        assert_eq!(b.query_row("SELECT COUNT(*) FROM items WHERE body='离线设备旧版本修改'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test] fn pull_conflict_is_atomic_and_preserves_local_edit(){
        let(mut a,mut b,mut s)=(db(),db(),Server::default());let id=add(&a);s.exchange(&mut a);s.exchange(&mut b);
        edit(&a,&id,"已上传版本");s.exchange(&mut a);edit(&b,&id,"本机未上传版本");let r=s.rows[&id].clone();pull(&mut b,&r).unwrap();assert!(!pull(&mut b,&r).unwrap());
        assert_eq!(document(&b,&id).unwrap().body,"已上传版本");assert_eq!(status(&b).unwrap()["conflicts"].as_array().unwrap().len(),1);
        assert_eq!(b.query_row("SELECT COUNT(*) FROM items WHERE body='本机未上传版本'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test] fn remote_update_invalidates_local_revision_for_unsaved_draft(){
        let(mut a,mut b,mut s)=(db(),db(),Server::default());let id=add(&a);s.exchange(&mut a);s.exchange(&mut b);
        let draft_revision:i64=b.query_row("SELECT revision FROM items WHERE id=?1",[&id],|r|r.get(0)).unwrap();
        edit(&a,&id,"另一电脑修改");s.exchange(&mut a);s.exchange(&mut b);
        let changed=b.execute("UPDATE items SET body='草稿' WHERE id=?1 AND revision=?2",params![id,draft_revision]).unwrap();assert_eq!(changed,0);
    }
    #[test] fn invalid_remote_rolls_back_content_and_queue(){
        let mut a=db();let id=add(&a);let o=next(&mut a).unwrap().unwrap();let mut d=o.document.clone();d.body_json=Some("invalid".into());let r=Remote{version:2,document:d};
        assert!(accept(&mut a,&o,false,&r).is_err());assert_eq!(document(&a,&id).unwrap().body,"原始正文");assert_eq!(status(&a).unwrap()["conflicts"].as_array().unwrap().len(),0);assert_eq!(next(&mut a).unwrap().unwrap().op,o.op);
    }
    #[test] fn offline_queue_survives_sqlite_reopen(){
        let root=std::path::Path::new("D:/SOFTWARE/轻笺/isolated-test/cloud-sync-rust");std::fs::create_dir_all(root).unwrap();let p=root.join(format!("{}.sqlite3",Uuid::new_v4()));
        let id;let op;
        {let mut a=Connection::open(&p).unwrap();crate::database::migrate(&mut a).unwrap();install(&a).unwrap();id=add(&a);op=next(&mut a).unwrap().unwrap().op;edit(&a,&id,"断网继续编辑");}
        {let mut a=Connection::open(&p).unwrap();install(&a).unwrap();assert_eq!(next(&mut a).unwrap().unwrap().op,op);let mut s=Server::default();s.exchange(&mut a);assert_eq!(s.rows[&id].document.body,"断网继续编辑");assert_eq!(status(&a).unwrap()["pending"],0);}
        std::fs::remove_file(p).unwrap();
    }
    #[test] fn list_reorder_restore_and_bulk_import_all_enqueue(){
        let mut a=db();let id=add(&a);let mut s=Server::default();s.exchange(&mut a);
        for sql in ["UPDATE items SET sort_order=42","UPDATE items SET is_pinned=1","UPDATE items SET deleted_at='2026-10-10T00:00:00Z'","UPDATE items SET deleted_at=NULL"]{a.execute(sql,[]).unwrap();assert_eq!(status(&a).unwrap()["pending"],1);s.exchange(&mut a);}
        assert_eq!(s.rows[&id].version,5);assert_eq!(s.rows[&id].document.sort_order,42);
    }
    #[test] fn version_index_only_requests_changed_documents(){
        let(mut a,mut s)=(db(),Server::default());let id=add(&a);
        assert!(needs_pull(&a,&id,1).unwrap());s.exchange(&mut a);
        assert!(!needs_pull(&a,&id,1).unwrap());assert!(needs_pull(&a,&id,2).unwrap());
        assert!(needs_pull(&a,&Uuid::new_v4().to_string(),1).unwrap());
        assert!(needs_pull(&a,"bad-id",1).is_err());assert!(needs_pull(&a,&id,0).is_err());
    }
}
