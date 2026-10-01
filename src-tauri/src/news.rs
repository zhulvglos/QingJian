use crate::database::Database;
use chrono::{DateTime,FixedOffset,NaiveDate,Utc};
use rusqlite::{Connection,OptionalExtension};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::{collections::HashSet,sync::Mutex,time::Duration};
use tauri::{Emitter,Manager};
use tokio::sync::oneshot;
const API:&str="https://aihot.news/api/v1/items";
const CHANNEL:&str="ai-selected-v1";
#[derive(Clone,Serialize,Deserialize,PartialEq)]
#[serde(rename_all="camelCase")]
pub struct Article{id:String,category:String,title:String,summary:String,source:String,source_url:Option<String>,permalink:Option<String>,published_at:Option<String>,#[serde(default)]timeline_at:Option<String>}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct NewsFeed{fetched_at:String,source:String,window:String,items:Vec<Article>,#[serde(default)]date:Option<String>}
#[derive(Clone,Default,Serialize)]
#[serde(rename_all="camelCase")]
pub struct UpdateStatus{initialized:bool,running:bool,error:String}
struct Runtime{id:u64,status:UpdateStatus,cancel:Option<oneshot::Sender<()>>}
static RUNTIME:Mutex<Runtime>=Mutex::new(Runtime{id:0,status:UpdateStatus{initialized:false,running:false,error:String::new()},cancel:None});
fn text(v:&Value,key:&str)->String{v[key].as_str().unwrap_or("").trim().to_string()}
fn safe_url(s:&str)->Option<String>{reqwest::Url::parse(s).ok().filter(|u|matches!(u.scheme(),"http"|"https")&&u.host_str().is_some()).map(|u|u.to_string())}
fn valid_date(s:&str)->Option<String>{NaiveDate::parse_from_str(s,"%Y-%m-%d").ok().filter(|d|d.format("%Y-%m-%d").to_string()==s).map(|_|s.to_owned())}
fn beijing_day()->String{
 // 可控日期仅用于隔离测试，不调整 Windows 时间，不影响正常用户。
 if std::env::var_os("QINGJIAN_TEST_MODE").is_some(){if let Some(day)=std::env::var("QINGJIAN_TEST_NEWS_DAY").ok().and_then(|s|valid_date(&s)){return day;}}
 Utc::now().with_timezone(&FixedOffset::east_opt(28800).unwrap()).format("%Y-%m-%d").to_string()
}
fn init_runtime_table(db:&Connection)->Result<(),String>{db.execute_batch("CREATE TABLE IF NOT EXISTS news_runtime(key TEXT PRIMARY KEY,value TEXT NOT NULL)").map_err(|_|"无法保存新闻更新状态".into())}
fn claim_auto_day(db:&Connection,day:&str)->Result<bool,String>{
 init_runtime_table(db)?;
 // 原子写入先提交再联网，多连接只有一方获准；失败和异常退出不退还当天机会。
 db.execute("INSERT INTO news_runtime(key,value) VALUES('auto_day',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value<>excluded.value",[day]).map(|n|n==1).map_err(|_|"无法保存本日新闻获取状态".into())
}
fn load_cache(db:&Connection)->Result<Option<NewsFeed>,String>{
 let raw:Option<String>=db.query_row("SELECT payload FROM news_cache WHERE channel=?1",[CHANNEL],|r|r.get(0)).optional().map_err(|_|"无法读取新闻缓存")?;
 if let Some(raw)=raw{let mut feed:NewsFeed=serde_json::from_str(&raw).map_err(|_|"上次新闻内容无法读取")?;feed.items.retain(|i|!i.id.trim().is_empty()&&!i.title.trim().is_empty());if feed.date.is_none(){feed.date=valid_date(&feed.window);}return Ok(Some(feed));}
 let legacy:Option<String>=db.query_row("SELECT payload FROM news_cache WHERE channel='ai'",[],|r|r.get(0)).optional().map_err(|_|"无法读取旧新闻缓存")?;
 let Some(raw)=legacy.and_then(|s|serde_json::from_str::<Value>(&s).ok())else{return Ok(None)};
 let rows=raw["items"].as_array().ok_or("旧新闻内容无法读取")?;
 let items=rows.iter().enumerate().filter_map(|(index,row)|{let title=text(row,"title");if title.is_empty(){return None;}Some(Article{id:format!("legacy-{index}"),category:text(row,"category"),title,summary:text(row,"summary"),source:text(row,"source"),source_url:safe_url(&text(row,"sourceUrl")),permalink:safe_url(&text(row,"permalink")),published_at:None,timeline_at:None})}).collect::<Vec<_>>();
 Ok(Some(NewsFeed{fetched_at:text(&raw,"fetchedAt"),source:"AIHOT".into(),window:text(&raw,"date"),date:valid_date(&text(&raw,"date")),items}))
}
fn timestamp(row:&Value,key:&str)->Option<DateTime<FixedOffset>>{DateTime::parse_from_rfc3339(&text(row,key)).ok()}
fn timeline(row:&Value)->Option<DateTime<FixedOffset>>{
 // 按官方 by=timeline 合同复现网页分组：延迟超过 72 小时的补录使用原文时间，其余使用收录时间。
 let published=timestamp(row,"publishedAt");let discovered=timestamp(row,"discoveredAt");
 match (published,discovered){(Some(p),Some(d)) if d.signed_duration_since(p)>chrono::Duration::hours(72)=>Some(p),(_,Some(d))=>Some(d),(p,None)=>p}
}
fn item_day(item:&Article)->Option<String>{item.timeline_at.as_deref().and_then(|s|DateTime::parse_from_rfc3339(s).ok()).map(|d|d.with_timezone(&FixedOffset::east_opt(28800).unwrap()).format("%Y-%m-%d").to_string())}
fn normalize_item(row:&Value)->Option<Article>{
 let id=text(row,"id");let title=text(row,"title");if id.is_empty()||title.is_empty()||row["selected"]==false{return None;}
 let category=match text(row,"category").as_str(){"ai-models"=>"模型","ai-products"=>"产品","industry"=>"行业","paper"=>"论文","tip"=>"技巧",_=>"精选"}.to_owned();
 Some(Article{id,category,title,summary:text(row,"summary"),source:text(&row["source"],"name"),source_url:safe_url(&text(&row["links"],"original")),permalink:safe_url(&text(&row["links"],"aihot")),published_at:timestamp(row,"publishedAt").map(|d|d.to_rfc3339()),timeline_at:timeline(row).map(|d|d.to_rfc3339())})
}
fn normalize_selected(rows:Vec<Article>)->Result<NewsFeed,String>{
 let date=rows.iter().filter_map(item_day).max().ok_or("新闻来源未提供有效精选日期，当前显示上次内容。")?;
 let mut seen=HashSet::new();let items=rows.into_iter().filter(|r|(item_day(r).is_none()||item_day(r).as_deref()==Some(&date))&&seen.insert(r.id.clone())).collect::<Vec<_>>();
 if items.is_empty(){return Err("新闻来源未提供有效内容，当前显示上次内容。".into());}
 Ok(NewsFeed{fetched_at:Utc::now().to_rfc3339(),source:"AIHOT 精选".into(),window:date.clone(),date:Some(date),items})
}
fn save_feed(db:&mut Connection,feed:NewsFeed)->Result<NewsFeed,String>{
 if feed.items.is_empty(){return Err("新闻来源未提供有效内容，当前显示上次内容。".into());}
 if let Ok(Some(old))=load_cache(db){
  // 来源未更新或返回更旧日报，保留内容与获取时间，不发生日期倒退。
  if old.source==feed.source&&(old.date>feed.date||(old.date==feed.date&&old.items==feed.items)){return Ok(old);}
 }
 let tx=db.transaction().map_err(|_|"新闻内容保存失败，当前显示上次内容。")?;
 tx.execute("INSERT INTO news_cache(channel,payload) VALUES(?1,?2) ON CONFLICT(channel) DO UPDATE SET payload=excluded.payload",rusqlite::params![CHANNEL,serde_json::to_string(&feed).map_err(|_|"新闻内容无法保存")?]).map_err(|_|"新闻内容保存失败，当前显示上次内容。")?;
 // 仅替换旧新闻通道，不动模型资讯；不建日报归档，安装前保存可恢复备份。
 tx.execute("DELETE FROM news_cache WHERE channel='ai'",[]).map_err(|_|"新闻缓存整理失败")?;tx.commit().map_err(|_|"新闻内容保存失败，当前显示上次内容。")?;Ok(feed)
}
fn network_error(e:reqwest::Error)->String{if e.is_timeout(){return "更新超时，当前显示上次内容。".into();}let reason=e.to_string().to_lowercase();if reason.contains("certificate")||reason.contains("tls"){return "新闻来源安全连接异常，当前显示上次内容。".into();}if e.is_connect(){"网络不可用，无法更新，当前显示上次内容。".into()}else{"新闻来源连接中断，当前显示上次内容。".into()}}
async fn fetch_selected()->Result<NewsFeed,String>{
 let mock=std::env::var("QINGJIAN_TEST_NEWS_SOURCE").ok().filter(|s|std::env::var_os("QINGJIAN_TEST_MODE").is_some()&&reqwest::Url::parse(s).ok().is_some_and(|u|u.scheme()=="http"&&matches!(u.host_str(),Some("127.0.0.1"|"localhost"))));
 let timeout=if std::env::var_os("QINGJIAN_TEST_MODE").is_some(){std::env::var("QINGJIAN_TEST_NEWS_TIMEOUT_MS").ok().and_then(|s|s.parse::<u64>().ok()).filter(|n|*n>0&&*n<=15000).unwrap_or(15000)}else{15000};
 let client=reqwest::Client::builder().timeout(Duration::from_millis(timeout)).connect_timeout(Duration::from_secs(8)).build().map_err(|_|"新闻连接初始化失败")?;
 let request=async{
 let mut cursor=None::<String>;let mut cursors=HashSet::new();let mut items=Vec::new();
 // 最新一个自然日完整翻页后才提交缓存；任何后续页失败均不写入半份结果。
 for _ in 0..100{
 let mut request=client.get(mock.as_deref().unwrap_or(API)).query(&[("mode","selected"),("window","7d"),("by","timeline"),("limit","100")]).header("User-Agent",concat!("Qingjian/",env!("CARGO_PKG_VERSION"))).header("Accept","application/json").header("Cache-Control","no-cache");
 if let Some(ref cursor)=cursor{request=request.query(&[("cursor",cursor)]);}
 let response=request.send().await.map_err(network_error)?;
 if !response.status().is_success(){return Err(format!("新闻来源异常（HTTP {}），当前显示上次内容。",response.status().as_u16()));}
 let bytes=response.bytes().await.map_err(network_error)?;if bytes.len()>2_000_000{return Err("新闻来源内容过大，当前显示上次内容。".into());}let raw:Value=serde_json::from_slice(&bytes).map_err(|_|"新闻来源数据无法解析，当前显示上次内容。")?;
 if raw["query"]["mode"]!="selected"||raw["query"]["by"]!="timeline"{return Err("新闻来源精选口径异常，当前显示上次内容。".into());}
 let rows=raw["items"].as_array().ok_or("新闻来源数据结构异常，当前显示上次内容。")?;
 items.extend(rows.iter().filter_map(normalize_item));
 let latest=items.iter().filter_map(item_day).max();
 // 接口按 timeline 倒序，当前页末已进入更早日期时，当日内容已经齐全。
 let older=latest.as_ref().is_some_and(|day|rows.iter().filter_map(timeline).any(|d|d.with_timezone(&FixedOffset::east_opt(28800).unwrap()).format("%Y-%m-%d").to_string()<*day));
 let has_more=raw["page"]["hasMore"].as_bool().ok_or("新闻来源分页信息异常，当前显示上次内容。")?;
 if !has_more||older{return normalize_selected(items);}
 let next=text(&raw["page"],"nextCursor");if next.is_empty()||!cursors.insert(next.clone()){return Err("新闻来源分页标识异常，当前显示上次内容。".into());}cursor=Some(next);
 }
 Err("新闻来源分页超过安全范围，当前显示上次内容。".into())};
 tokio::time::timeout(Duration::from_millis(timeout),request).await.map_err(|_|"更新超时，当前显示上次内容。".to_string())?
}
#[tauri::command]pub fn get_news_cache(database:tauri::State<'_,Database>)->Result<Option<NewsFeed>,String>{let db=database.0.lock().map_err(|_|"新闻缓存暂不可用")?;load_cache(&db)}
#[tauri::command]pub fn get_news_update_status()->UpdateStatus{RUNTIME.lock().map(|r|r.status.clone()).unwrap_or_default()}
pub fn cancel(){if let Ok(mut runtime)=RUNTIME.lock(){if let Some(tx)=runtime.cancel.take(){let _=tx.send(());}runtime.id+=1;runtime.status.running=false;}}
pub fn start_background(app:tauri::AppHandle){
 let claimed=(||{let state=app.state::<Database>();let db=state.0.lock().map_err(|_|"新闻缓存暂不可用")?;let claimed=claim_auto_day(&db,&beijing_day())?;let error=db.query_row("SELECT value FROM news_runtime WHERE key='last_error'",[],|r|r.get::<_,String>(0)).optional().map_err(|_|"无法读取新闻更新状态")?.unwrap_or_default();Ok::<_,String>((claimed,error))})();
 // 先释放数据库锁，统一按任务锁、数据库锁的顺序，避免与完成回调交叉死锁。
 match claimed{Ok((claimed,error))=>{if let Ok(mut runtime)=RUNTIME.lock(){runtime.status.error=error;if !claimed{runtime.status.initialized=true;}}if claimed{if let Ok((id,rx))=begin_update(){tauri::async_runtime::spawn(async move{let _=run_update(app,id,rx).await;});}}},Err(error)=>{if let Ok(mut runtime)=RUNTIME.lock(){runtime.status.error=error;runtime.status.initialized=true;}}}
}
fn begin_update()->Result<(u64,oneshot::Receiver<()>),String>{
 let(id,rx)={let mut runtime=RUNTIME.lock().map_err(|_|"新闻更新状态暂不可用")?;if runtime.status.running{return Err("新闻正在更新，请稍后刷新。".into());}runtime.id+=1;runtime.status=UpdateStatus{initialized:true,running:true,error:String::new()};let(tx,rx)=oneshot::channel();runtime.cancel=Some(tx);(runtime.id,rx)};
 Ok((id,rx))
}
#[tauri::command]pub async fn refresh_news(app:tauri::AppHandle)->Result<NewsFeed,String>{let(id,rx)=begin_update()?;run_update(app,id,rx).await}
async fn run_update(app:tauri::AppHandle,id:u64,rx:oneshot::Receiver<()>)->Result<NewsFeed,String>{
 let result=tokio::select!{_ = rx=>Err("新闻更新已取消".into()),result=fetch_selected()=>result};
 // 持任务锁再写缓存，取消或退出后的迟到响应不能写入结果。
 let mut runtime=RUNTIME.lock().map_err(|_|"新闻更新状态暂不可用")?;if runtime.id!=id{return Err("新闻更新已取消".into());}
 let result=result.and_then(|feed|{let state=app.state::<Database>();let mut db=state.0.lock().map_err(|_|"新闻缓存暂不可用")?;save_feed(&mut db,feed)});
 runtime.cancel=None;runtime.status=UpdateStatus{initialized:true,running:false,error:result.as_ref().err().cloned().unwrap_or_default()};
 let state=app.state::<Database>();if let Ok(db)=state.0.lock(){let _=init_runtime_table(&db);let _=db.execute("INSERT INTO news_runtime(key,value) VALUES('last_error',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&runtime.status.error]);}
 let _=app.emit("ai-news-updated",&runtime.status);result
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
 fn row(date:&str,title:&str)->Value{json!({"id":"a","title":title,"summary":"来源内容","links":{"aihot":"https://aihot.news/items/a"},"discoveredAt":format!("{date}T12:00:00+08:00"),"publishedAt":null,"selected":true})}
 fn feed(date:&str,title:&str)->NewsFeed{normalize_selected(vec![normalize_item(&row(date,title)).unwrap()]).unwrap()}
 #[test]fn selected_date_and_missing_publication_time_are_not_invented(){let f=feed("2026-09-30","前一日精选");assert_eq!(f.date.as_deref(),Some("2026-09-30"));assert_eq!(f.items.len(),1);assert!(f.items[0].published_at.is_none());assert!(normalize_item(&row("2026-10-01"," ")).is_none());let mut invalid=row("2026-10-01","a");invalid["id"]=Value::Null;assert!(normalize_item(&invalid).is_none());assert!(normalize_selected(vec![]).is_err());}
 #[test]fn timeline_matches_website_slow_push_and_dedup(){let mut r=row("2026-10-01","a");r["publishedAt"]=json!("2026-09-30T03:00:00+08:00");assert_eq!(item_day(&normalize_item(&r).unwrap()).as_deref(),Some("2026-10-01"));r["publishedAt"]=json!("2026-09-20T03:00:00+08:00");assert_eq!(item_day(&normalize_item(&r).unwrap()).as_deref(),Some("2026-09-20"));let a=normalize_item(&row("2026-10-01","a")).unwrap();let f=normalize_selected(vec![a.clone(),a,normalize_item(&r).unwrap()]).unwrap();assert_eq!(f.items.len(),1);}
 #[test]fn single_cache_replaces_only_on_valid_changed_source(){let mut db=Connection::open_in_memory().unwrap();db.execute_batch("CREATE TABLE news_cache(channel TEXT PRIMARY KEY,payload TEXT NOT NULL)").unwrap();let first=save_feed(&mut db,feed("2026-09-30","旧内容")).unwrap();let same=save_feed(&mut db,feed("2026-09-30","旧内容")).unwrap();assert_eq!(first.fetched_at,same.fetched_at);save_feed(&mut db,feed("2026-10-01","新内容")).unwrap();let old=save_feed(&mut db,feed("2026-09-29","来源倒退")).unwrap();assert_eq!(old.date.as_deref(),Some("2026-10-01"));assert_eq!(db.query_row("SELECT count(*) FROM news_cache",[],|r|r.get::<_,u32>(0)).unwrap(),1);}
 #[test]fn beijing_natural_day_crosses_at_utc_16(){let t=DateTime::parse_from_rfc3339("2026-10-01T15:59:59Z").unwrap();let z=FixedOffset::east_opt(28800).unwrap();assert_eq!(t.with_timezone(&z).format("%Y-%m-%d").to_string(),"2026-10-01");assert_eq!((t+chrono::Duration::seconds(1)).with_timezone(&z).format("%Y-%m-%d").to_string(),"2026-10-02");}
 #[test]fn atomic_claim_survives_reopen_and_multiple_connections(){
  let path=std::env::temp_dir().join(format!("qingjian-news-{}.sqlite3",uuid::Uuid::new_v4()));let db=Connection::open(&path).unwrap();init_runtime_table(&db).unwrap();drop(db);
  let barrier=std::sync::Arc::new(std::sync::Barrier::new(4));let mut threads=Vec::new();
  for _ in 0..4{let path=path.clone();let barrier=barrier.clone();threads.push(std::thread::spawn(move||{let db=Connection::open(path).unwrap();db.busy_timeout(Duration::from_secs(5)).unwrap();barrier.wait();claim_auto_day(&db,"2026-10-01").unwrap()}));}
  assert_eq!(threads.into_iter().filter_map(|t|t.join().unwrap().then_some(1)).sum::<u32>(),1);let db=Connection::open(path).unwrap();assert!(!claim_auto_day(&db,"2026-10-01").unwrap());assert!(claim_auto_day(&db,"2026-10-02").unwrap());assert!(!claim_auto_day(&db,"2026-10-02").unwrap());
 }
}
