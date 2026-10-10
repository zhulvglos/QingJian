//! 邮箱认证由 Supabase 执行；令牌仅在 Rust 内存与 Windows 凭据存储中存在。
use crate::{database::Database, sync_store::{self, Remote}};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::{Path, PathBuf}, sync::{Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
use tauri::{Emitter, Manager};

fn err(e:impl std::fmt::Display)->String{e.to_string()}
fn hash(s:&str)->String{format!("{:x}",Sha256::digest(s.as_bytes()))}
#[derive(Clone,Default,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Config { url:String, public_key:String }
#[derive(Clone,Serialize,Deserialize)]
struct Account { id:String, email:String, project:String }
struct Session { access:String, refresh:String, expires:i64 }
#[derive(Default)]
struct Progress { phase:String, error:String }
pub struct Cloud {
    root:PathBuf, config:Mutex<Config>, account:Option<Account>,
    gate:Mutex<Option<Session>>, progress:Mutex<Progress>, stopped:AtomicBool, cancel_sync:AtomicBool,
}
fn read_json<T:serde::de::DeserializeOwned>(p:&Path)->Result<T,String>{serde_json::from_slice(&std::fs::read(p).map_err(err)?).map_err(|_|"云同步配置文件无效".into())}
fn write_json(p:&Path,v:&impl Serialize)->Result<(),String>{
    use std::io::Write;
    let temp=p.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(err)?;
    f.write_all(&serde_json::to_vec_pretty(v).map_err(err)?).map_err(err)?;f.sync_all().map_err(err)?;drop(f);
    std::fs::rename(&temp,p).map_err(err)
}
fn validate_config(c:&Config)->Result<(),String>{
    let url=reqwest::Url::parse(&c.url).map_err(|_|"请输入 Supabase 项目 HTTPS 地址")?;
    if url.scheme()!="https"||!url.host_str().is_some_and(|s|s.ends_with(".supabase.co"))||url.port().is_some()||url.path()!="/"||url.query().is_some()||url.fragment().is_some()||!url.username().is_empty()||url.password().is_some(){return Err("仅支持官方 Supabase 项目地址，例如 https://项目标识.supabase.co".into());}
    if c.public_key.starts_with("sb_publishable_"){return Ok(());}
    // 旧版 anon JWT 仅用于验证配置类型，不参与认证或签名；绝不接收管理密钥。
    use base64::Engine;
    let role=c.public_key.split('.').nth(1).and_then(|s|base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()).and_then(|b|serde_json::from_slice::<Value>(&b).ok()).and_then(|v|v["role"].as_str().map(str::to_string));
    if role.as_deref()!=Some("anon"){return Err("只能填写 publishable key 或 anon key，禁止填写 secret / service_role 管理密钥".into());}Ok(())
}
fn account_path(root:&Path,a:&Account)->Result<PathBuf,String>{
    if !root.to_string_lossy().to_ascii_lowercase().starts_with("d:"){return Err("云同步工作区必须位于 D 盘，请先配置 QINGJIAN_DATA_ROOT 后重启".into());}
    uuid::Uuid::parse_str(&a.id).map_err(|_|"本地账号标识无效")?;
    if a.project.len()!=64||!a.project.bytes().all(|b|b.is_ascii_hexdigit()){return Err("本地项目标识无效".into());}
    Ok(root.join("accounts").join(&a.project).join(&a.id).join("data"))
}
fn credential(cloud:&Cloud,a:&Account)->Result<keyring::Entry,String>{
    let scope=format!("Qingjian.Sync.{}",hash(&cloud.root.to_string_lossy()));
    keyring::Entry::new(&scope,&format!("{}.{}",a.project,a.id)).map_err(|_|"Windows 凭据存储不可用".into())
}
pub fn initialize(root:&Path)->Result<(Cloud,PathBuf),String>{
    let config=if root.join("cloud-config.json").exists(){read_json(&root.join("cloud-config.json"))?}else{Config::default()};
    let account:Option<Account>=if root.join("active-account.json").exists(){Some(read_json(&root.join("active-account.json"))?)}else{None};
    let effective=if let Some(a)=&account{account_path(root,a)?}else{root.to_path_buf()};
    let cloud=Cloud{root:root.to_path_buf(),config:Mutex::new(config),account,gate:Mutex::new(None),progress:Mutex::new(Progress::default()),stopped:AtomicBool::new(false),cancel_sync:AtomicBool::new(false)};
    Ok((cloud,effective))
}
// 只向本机资源检测提供基础目录，不暴露会话或其他账号数据。
pub fn local_base_root(cloud:&Cloud)->&Path { &cloud.root }
fn email(value:&str)->Result<String,String>{
    let s=value.trim();
    if s.len()>254||!regex::Regex::new(r"^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$").unwrap().is_match(s){return Err("请输入有效邮箱地址，例如 123456@qq.com".into());}Ok(s.into())
}
fn client()->Result<reqwest::blocking::Client,String>{reqwest::blocking::Client::builder().timeout(Duration::from_secs(25)).connect_timeout(Duration::from_secs(10)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_|"无法建立安全网络连接".into())}
fn response(r:reqwest::blocking::Response)->Result<Value,String>{
    let status=r.status();
    // 不把服务端原始响应、URL 查询、令牌或笔记正文写入日志或错误报告。
    if status.as_u16()==429{return Err("请求过于频繁，请稍后重试（服务端限流）".into());}
    if status.as_u16()==401{return Err("会话已过期，请重新获取邮箱验证码登录".into());}
    let bytes=r.bytes().map_err(|_|"读取响应失败，请检查网络")?;
    if bytes.len()>8_000_000{return Err("云端响应过大，已停止同步".into());}
    let v:Value=serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success(){return Err(match v["error_code"].as_str().or(v["code"].as_str()).unwrap_or(""){
        "otp_expired"=>"验证码错误或已过期，请重新获取",
        "over_email_send_rate_limit"|"over_request_rate_limit"=>"请求过于频繁，请稍后重试（服务端限流）",
        "email_address_not_authorized"=>"发信服务尚未配置为支持此邮箱，请联系项目管理员",
        "refresh_token_not_found"|"refresh_token_already_used"|"session_not_found"=>"会话已过期，请重新获取邮箱验证码登录",
        _=>"云端请求失败，请核对认证、发信或数据库配置后重试",
    }.into());}Ok(v)
}
fn request(c:&Config,path:&str,token:Option<&str>,body:Option<&Value>)->Result<Value,String>{
    validate_config(c)?;let client=client()?;let url=format!("{}{}",c.url.trim_end_matches('/'),path);
    let mut req=if let Some(body)=body{client.post(url).json(body)}else{client.get(url)};
    req=req.header("apikey",&c.public_key);if let Some(t)=token{req=req.bearer_auth(t);}
    response(req.send().map_err(|_|"网络连接失败，本地内容已保留，可稍后重试")?)
}
fn parse_session(v:&Value)->Result<(Account,Session),String>{
    let id=v["user"]["id"].as_str().ok_or("认证返回缺少账号")?.to_string();uuid::Uuid::parse_str(&id).map_err(|_|"认证返回的账号无效")?;
    let a=Account{id,email:email(v["user"]["email"].as_str().ok_or("认证返回缺少邮箱")?)?,project:String::new()};
    let s=Session{access:v["access_token"].as_str().filter(|s|!s.is_empty()).ok_or("认证未返回会话")?.into(),refresh:v["refresh_token"].as_str().filter(|s|!s.is_empty()).ok_or("认证未返回刷新凭据")?.into(),expires:chrono::Utc::now().timestamp()+v["expires_in"].as_i64().filter(|n|*n>0).ok_or("认证未返回会话有效期")?};Ok((a,s))
}
fn can_switch()->Result<(),String>{
    if crate::audio::active()||crate::audio::processing()||crate::online::processing()||crate::document_import::saving(){return Err("请先完成录音、导入或处理任务，再变更账号".into());}
    Ok(())
}
fn token(cloud:&Cloud,slot:&mut Option<Session>,c:&Config)->Result<String,String>{
    let a=cloud.account.as_ref().ok_or("请先登录邮箱账号")?;
    if a.project!=hash(c.url.trim_end_matches('/')){return Err("云端项目与当前账号不一致，请退出后重新配置".into());}
    if slot.as_ref().is_some_and(|s|s.expires>chrono::Utc::now().timestamp()+60){return Ok(slot.as_ref().unwrap().access.clone());}
    let refresh=if let Some(s)=slot.as_ref(){s.refresh.clone()}else{credential(cloud,a)?.get_password().map_err(|_|"会话已过期，请重新获取邮箱验证码登录")?};
    let v=request(c,"/auth/v1/token?grant_type=refresh_token",None,Some(&json!({"refresh_token":refresh})))?;
    let (remote,s)=parse_session(&v)?;
    if remote.id!=a.id{return Err("会话账号不一致，已停止同步".into());}
    credential(cloud,a)?.set_password(&s.refresh).map_err(|_|"无法安全保存会话，请重新登录")?;
    let access=s.access.clone();*slot=Some(s);Ok(access)
}
#[tauri::command]
pub fn cloud_status(app:tauri::AppHandle)->Result<Value,String>{
    let cloud=app.state::<Cloud>();let config=cloud.config.lock().map_err(err)?.clone();let p=cloud.progress.lock().map_err(err)?;
    let mut v=json!({"configured":validate_config(&config).is_ok(),"url":config.url,"publicKey":config.public_key,"email":cloud.account.as_ref().map(|a|&a.email),"phase":p.phase,"error":p.error,"restartRequired":cloud.stopped.load(Ordering::SeqCst)});
    drop(p);if cloud.account.is_some(){let db=app.state::<Database>();v["sync"]=sync_store::status(&*db.0.lock().map_err(err)?)?;}
    Ok(v)
}
#[tauri::command]
pub fn cloud_configure(url:String,public_key:String,app:tauri::AppHandle)->Result<(),String>{
    let cloud=app.state::<Cloud>();let _gate=cloud.gate.lock().map_err(err)?;
    if cloud.account.is_some()||cloud.stopped.load(Ordering::SeqCst){return Err("请先退出账号并重启后配置项目".into());}
    let c=Config{url:url.trim().trim_end_matches('/').into(),public_key:public_key.trim().into()};validate_config(&c)?;
    if !cloud.root.to_string_lossy().to_ascii_lowercase().starts_with("d:"){return Err("云同步工作区必须位于 D 盘，请先配置 QINGJIAN_DATA_ROOT 后重启".into());}
    write_json(&cloud.root.join("cloud-config.json"),&c)?;*cloud.config.lock().map_err(err)?=c;Ok(())
}
fn auth_send(app:&tauri::AppHandle,input:&str)->Result<(),String>{
    let cloud=app.state::<Cloud>();let _gate=cloud.gate.lock().map_err(err)?;
    if cloud.stopped.load(Ordering::SeqCst){return Err("请先重启完成账号切换".into());}
    let e=email(input)?;
    if cloud.account.as_ref().is_some_and(|a|!a.email.eq_ignore_ascii_case(&e)){return Err("请先退出当前账号并重启，再登录其他邮箱".into());}
    let c=cloud.config.lock().map_err(err)?.clone();request(&c,"/auth/v1/otp",None,Some(&json!({"email":e,"create_user":true})))?;Ok(())
}
#[tauri::command]
pub async fn cloud_send_code(email:String,app:tauri::AppHandle)->Result<(),String>{tauri::async_runtime::spawn_blocking(move||auth_send(&app,&email)).await.map_err(err)?}
#[tauri::command]
pub async fn cloud_verify(email:String,code:String,app:tauri::AppHandle)->Result<bool,String>{
    tauri::async_runtime::spawn_blocking(move||{
        let cloud=app.state::<Cloud>();let mut slot=cloud.gate.lock().map_err(err)?;
        if cloud.stopped.load(Ordering::SeqCst){return Err("请先重启完成账号切换".into());}
        let e=self::email(&email)?;
        if cloud.account.as_ref().is_some_and(|a|!a.email.eq_ignore_ascii_case(&e)){return Err("请先退出当前账号并重启".into());}
        if !(6..=10).contains(&code.len())||!code.bytes().all(|b|b.is_ascii_digit()){return Err("请输入邮件中的数字验证码".into());}
        let c=cloud.config.lock().map_err(err)?.clone();let v=request(&c,"/auth/v1/verify",None,Some(&json!({"email":e,"token":code,"type":"email"})))?;
        let(mut a,s)=parse_session(&v)?;a.project=hash(c.url.trim_end_matches('/'));
        // 网络等待期间可能有先前发起的任务开始运行；提交账号变更前再次核对原生状态。
        can_switch()?;
        if cloud.account.as_ref().is_some_and(|old|old.id!=a.id||old.project!=a.project){return Err("会话账号不一致，请先退出后重新登录".into());}
        credential(&cloud,&a)?.set_password(&s.refresh).map_err(|_|"无法安全保存会话，登录未完成")?;
        let restart=cloud.account.is_none();
        if restart {
            let dir=account_path(&cloud.root,&a)?;std::fs::create_dir_all(&dir).map_err(err)?;
            write_json(&cloud.root.join("active-account.json"),&a)?;
            cloud.stopped.store(true,Ordering::SeqCst);
        }
        *slot=Some(s);cloud.cancel_sync.store(false,Ordering::SeqCst);cloud.progress.lock().map_err(err)?.error.clear();Ok(restart)
    }).await.map_err(err)?
}
#[tauri::command]
pub async fn cloud_logout(app:tauri::AppHandle)->Result<(),String>{
    tauri::async_runtime::spawn_blocking(move||{
        can_switch()?;
        let cloud=app.state::<Cloud>();
        // 先停止新的同步请求，再等待已发出的请求结束，避免退出排队期间继续上传。
        cloud.cancel_sync.store(true,Ordering::SeqCst);
        let mut slot=cloud.gate.lock().map_err(err)?;
        let a=cloud.account.as_ref().ok_or("当前没有登录账号")?;
        // 本机退出不依赖联网。先删除安全凭据与选择标记，再停止本进程同步。
        match credential(&cloud,a)?.delete_credential(){Ok(())|Err(keyring::Error::NoEntry)=>{},Err(_)=>return Err("无法删除 Windows 会话凭据，退出未完成".into())}
        match std::fs::remove_file(cloud.root.join("active-account.json")){Ok(())=>{},Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(err(e))}
        cloud.stopped.store(true,Ordering::SeqCst);
        if let Some(s)=slot.take(){let c=cloud.config.lock().map_err(err)?.clone();let _=request(&c,"/auth/v1/logout?scope=local",Some(&s.access),Some(&json!({})));}
        Ok(())
    }).await.map_err(err)?
}
#[tauri::command]
pub fn cloud_restart(app:tauri::AppHandle)->Result<(),String>{
    if !app.state::<Cloud>().stopped.load(Ordering::SeqCst){return Err("没有待完成的账号切换".into());}
    if crate::audio::active()||crate::audio::processing()||crate::online::processing()||crate::document_import::saving(){return Err("请先完成录音、导入或处理任务".into());}
    app.restart();
}
#[tauri::command]
pub fn cloud_link_local(include:bool,app:tauri::AppHandle)->Result<Value,String>{
    let cloud=app.state::<Cloud>();let _gate=cloud.gate.lock().map_err(err)?;
    if cloud.account.is_none()||cloud.stopped.load(Ordering::SeqCst){return Err("请先登录并完成重启".into());}
    let db=app.state::<Database>();let mut c=db.0.lock().map_err(err)?;
    link_existing(&cloud.root,&mut c,include)
}
// 独立于认证与网络，便于验证备份失败时不关联、重试不重复、原库不修改。
fn link_existing(root:&Path,c:&mut Connection,include:bool)->Result<Value,String>{
    if c.query_row("SELECT linked FROM sync_control WHERE id=1",[],|r|r.get::<_,bool>(0)).map_err(err)?{return Ok(json!({"alreadyLinked":true}));}
    let mut backup=None;let mut imported=0;
    if include&&root.join("qingjian-v1.sqlite3").exists(){
        let source=Connection::open_with_flags(root.join("qingjian-v1.sqlite3"),OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        let before=root.join("backups").join(format!("首次账号关联前-{}.sqlite3",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(before.parent().unwrap()).map_err(err)?;
        // 只读打开原库，以 SQLite 一致快照保护 WAL 中已提交内容，校验后才允许关联。
        source.execute("VACUUM INTO ?1",[before.to_string_lossy().as_ref()]).map_err(err)?;
        let frozen=Connection::open_with_flags(&before,OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        let integrity:String=frozen.query_row("PRAGMA integrity_check",[],|r|r.get(0)).map_err(err)?;
        if integrity!="ok"{return Err("关联前备份校验失败，未上传内容".into());}
        c.execute("ATTACH DATABASE ?1 AS local_source",[before.to_string_lossy().as_ref()]).map_err(err)?;
        let result=(||{let tx=c.transaction().map_err(err)?;
            imported=tx.execute("INSERT OR IGNORE INTO items(id,kind,title,body,body_json,created_at,updated_at,deleted_at,is_pinned,sort_order) SELECT id,kind,title,body,body_json,created_at,updated_at,deleted_at,is_pinned,sort_order FROM local_source.items",[]).map_err(err)?;
            tx.execute("UPDATE sync_control SET linked=1 WHERE id=1",[]).map_err(err)?;tx.commit().map_err(err)
        })();let detached=c.execute_batch("DETACH DATABASE local_source").map_err(err);result?;detached?;backup=Some(before.to_string_lossy().into_owned());
    }else{c.execute("UPDATE sync_control SET linked=1 WHERE id=1",[]).map_err(err)?;}
    Ok(json!({"imported":imported,"backup":backup}))
}
fn synchronize(app:&tauri::AppHandle,cloud:&Cloud,slot:&mut Option<Session>)->Result<(),String>{
    if cloud.cancel_sync.load(Ordering::SeqCst){return Err("同步已暂停，请完成退出或重新验证邮箱".into());}
    if cloud.stopped.load(Ordering::SeqCst)||cloud.account.is_none(){return Err("请先登录邮箱账号".into());}
    let db=app.state::<Database>();
    if !sync_store::status(&*db.0.lock().map_err(err)?)?["linked"].as_bool().unwrap_or(false){return Err("请先选择是否关联本机已有内容".into());}
    let c=cloud.config.lock().map_err(err)?.clone();let access=token(cloud,slot,&c)?;
    // 上传优先。单轮限量，持久化队列在断网、退出和崩溃后仍可继续重试。
    for _ in 0..200 {
        if cloud.cancel_sync.load(Ordering::SeqCst){return Err("同步已暂停，请完成退出或重新验证邮箱".into());}
        let op=sync_store::next(&mut *db.0.lock().map_err(err)?)?;let Some(op)=op else{break};
        let result=request(&c,"/rest/v1/rpc/qj_push",Some(&access),Some(&json!({"p_op":op.op,"p_base":op.base,"p_document":op.document})))?;
        let ok=result["ok"].as_bool().ok_or("云端同步返回格式无效")?;
        let remote:Remote=serde_json::from_value(result["item"].clone()).map_err(|_|"云端同步返回格式无效")?;
        sync_store::accept(&mut *db.0.lock().map_err(err)?,&op,ok,&remote)?;
    }
    // 先取版本索引，仅下载变化的正文，避免个人免费额度被重复全文拉取消耗。
    // UUID 分页不把自增序号当作事务提交游标；扫描中发生的变化在下一轮收敛。
    let mut cursor=String::new();
    loop {
        if cloud.cancel_sync.load(Ordering::SeqCst){return Err("同步已暂停，请完成退出或重新验证邮箱".into());}
        let path=format!("/rest/v1/qj_documents?select=id,version&order=id.asc&limit=100{}",if cursor.is_empty(){String::new()}else{format!("&id=gt.{cursor}")});
        let result=request(&c,&path,Some(&access),None)?;
        let rows=result.as_array().ok_or("云端同步返回格式无效")?;
        let mut changed=Vec::new();
        for row in rows {
            let id=row["id"].as_str().ok_or("云端内容标识无效")?;
            let version=row["version"].as_i64().ok_or("云端版本无效")?;
            if sync_store::needs_pull(&*db.0.lock().map_err(err)?,id,version)?{changed.push(id);}
            cursor=id.to_string();
        }
        if !changed.is_empty(){
            let path=format!("/rest/v1/qj_documents?select=version,document&id=in.({})&order=id.asc&limit=100",changed.join(","));
            let documents=request(&c,&path,Some(&access),None)?;
            for row in documents.as_array().ok_or("云端内容返回格式无效")?{
                let remote:Remote=serde_json::from_value(row.clone()).map_err(|_|"云端内容格式无效")?;
                if !changed.contains(&remote.document.id.as_str()){return Err("云端内容标识不一致".into());}
                sync_store::pull(&mut *db.0.lock().map_err(err)?,&remote)?;
            }
        }
        if rows.len()<100{break;}
    }
    db.0.lock().map_err(err)?.execute("UPDATE sync_control SET last_success=?1 WHERE id=1",[chrono::Utc::now().to_rfc3339()]).map_err(err)?;
    Ok(())
}
fn run(app:&tauri::AppHandle,cloud:&Cloud,slot:&mut Option<Session>)->Result<(),String>{
    {let mut p=cloud.progress.lock().map_err(err)?;p.phase="正在同步".into();p.error.clear();}
    let result=synchronize(app,cloud,slot);
    {let mut p=cloud.progress.lock().map_err(err)?;p.phase=if result.is_ok(){"已同步"}else{"同步失败，可重试"}.into();p.error=result.as_ref().err().cloned().unwrap_or_default();}
    // 无论中途失败还是成功，已拉取的内容都通知列表；前端保留未保存草稿。
    let _=app.emit("cloud-content-changed",());let _=app.emit_to("quick","quick-items-changed",true);
    result
}
#[tauri::command]
pub async fn cloud_sync(app:tauri::AppHandle)->Result<(),String>{tauri::async_runtime::spawn_blocking(move||{let cloud=app.state::<Cloud>();let mut slot=cloud.gate.lock().map_err(err)?;run(&app,&cloud,&mut slot)}).await.map_err(err)?}
#[tauri::command]
pub fn cloud_resolve_conflict(copy_id:String,app:tauri::AppHandle)->Result<(),String>{
    let db=app.state::<Database>();db.0.lock().map_err(err)?.execute("UPDATE sync_conflicts SET resolved=1 WHERE copy_id=?1",[copy_id]).map_err(err)?;Ok(())
}
pub fn start(app:tauri::AppHandle){
    std::thread::Builder::new().name("qingjian-cloud-sync".into()).spawn(move||{
        let mut failures=0u32;
        loop {
            std::thread::sleep(Duration::from_secs(if failures==0{60}else{(30u64<<failures.min(5)).min(900)}));
            let cloud=app.state::<Cloud>();if cloud.account.is_none()||cloud.stopped.load(Ordering::SeqCst)||cloud.cancel_sync.load(Ordering::SeqCst){continue;}
            let Ok(mut slot)=cloud.gate.try_lock() else{continue};
            let linked={let db=app.state::<Database>();let c=db.0.lock();c.ok().and_then(|c|sync_store::status(&c).ok()).is_some_and(|v|v["linked"]==true)};
            if !linked{continue;}
            if run(&app,&cloud,&mut slot).is_ok(){failures=0}else{failures=failures.saturating_add(1);}
        }
    }).ok();
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn accepts_qq_and_other_email(){assert!(email("123456@qq.com").is_ok());assert!(email("name+tag@example.org").is_ok());assert!(email("bad @qq.com").is_err());}
    #[test] fn rejects_admin_keys_and_external_endpoints(){assert!(validate_config(&Config{url:"https://demo.supabase.co".into(),public_key:"sb_secret_hidden".into()}).is_err());assert!(validate_config(&Config{url:"https://evil.example".into(),public_key:"sb_publishable_test".into()}).is_err());}
    #[test] fn cache_paths_isolate_accounts_and_projects(){let root=Path::new("D:/isolated-test");let a=Account{id:uuid::Uuid::new_v4().to_string(),email:"a@qq.com".into(),project:hash("one")};let mut b=a.clone();b.id=uuid::Uuid::new_v4().to_string();assert_ne!(account_path(root,&a).unwrap(),account_path(root,&b).unwrap());b=a.clone();b.project=hash("two");assert_ne!(account_path(root,&a).unwrap(),account_path(root,&b).unwrap());b.id="../../bad".into();assert!(account_path(root,&b).is_err());}
    fn fixture()->(PathBuf,Connection,String){
        let root=PathBuf::from("D:/SOFTWARE/轻笺/isolated-test/cloud-link-rust").join(uuid::Uuid::new_v4().to_string());
        let (local,_)=crate::database::open(&root).unwrap();let id=crate::database::insert_new_item(&local.0.lock().unwrap(),"sticky","首次关联测试","原始内容",None).unwrap();drop(local);
        let mut account=Connection::open_in_memory().unwrap();crate::database::migrate(&mut account).unwrap();sync_store::install(&account).unwrap();(root,account,id)
    }
    #[test] fn first_link_backs_up_and_retries_do_not_duplicate_or_change_source(){
        let(root,mut account,id)=fixture();let result=link_existing(&root,&mut account,true).unwrap();assert_eq!(result["imported"],1);
        let backup=Connection::open(result["backup"].as_str().unwrap()).unwrap();assert_eq!(sync_store::document(&backup,&id).unwrap().body,"原始内容");
        account.execute("UPDATE items SET body='账号内独立修改' WHERE id=?1",[&id]).unwrap();assert_eq!(link_existing(&root,&mut account,true).unwrap()["alreadyLinked"],true);
        let source=Connection::open(root.join("qingjian-v1.sqlite3")).unwrap();assert_eq!(sync_store::document(&source,&id).unwrap().body,"原始内容");assert_eq!(sync_store::document(&account,&id).unwrap().body,"账号内独立修改");
        assert_eq!(account.query_row("SELECT COUNT(*) FROM items",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        // 测试夹具保留在 D 盘，可直接检查快照与原库。
    }
    #[test] fn backup_failure_never_links_or_imports(){
        let(root,mut account,_)=fixture();std::fs::write(root.join("backups"),b"fixture blocks directory").unwrap();assert!(link_existing(&root,&mut account,true).is_err());
        assert_eq!(sync_store::status(&account).unwrap()["linked"],false);assert_eq!(account.query_row("SELECT COUNT(*) FROM items",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    #[test] fn decline_first_link_keeps_original_private(){
        let(root,mut account,id)=fixture();link_existing(&root,&mut account,false).unwrap();assert_eq!(sync_store::status(&account).unwrap()["pending"],0);assert!(sync_store::document(&account,&id).is_err());assert!(!root.join("backups").exists());
    }
    #[test] fn auth_errors_are_classified_without_exposing_response_secrets(){
        use std::io::{Read,Write};
        for(status,body,expected)in [(400,r#"{"error_code":"otp_expired","secret":"DO_NOT_ECHO"}"#,"验证码错误或已过期"),(429,r#"{"secret":"DO_NOT_ECHO"}"#,"服务端限流"),(401,r#"{"secret":"DO_NOT_ECHO"}"#,"会话已过期")]{
            let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
            let worker=std::thread::spawn(move||{let(mut stream,_)=listener.accept().unwrap();let mut b=[0;2048];let _=stream.read(&mut b);write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();});
            let result=response(client().unwrap().get(format!("http://{address}")).send().unwrap()).unwrap_err();worker.join().unwrap();assert!(result.contains(expected));assert!(!result.contains("DO_NOT_ECHO"));
        }
    }
}
