//! 资讯加入只读取本地缓存；复用导入备份和事务，不抓取外网、不沿用外部 ID。
use serde_json::{json,Value};
use tauri::{Emitter,Manager};
use crate::{database::Database,document_formats as formats,document_import::{Selection,insert_batch}};
fn text(v:&Value,k:&str)->String{v[k].as_str().unwrap_or("").to_owned()}
pub(crate) fn prepare(cache:&Value,source:&str,ids:&[String],kind:&str)->Result<Vec<(formats::Entry,Selection)>,String>{
 if !["sticky","note"].contains(&kind)||ids.is_empty()||ids.len()>200{return Err("请选择 1–200 条资讯和有效目标栏目".into());}
 let rows=if source=="news"{cache["items"].as_array().cloned().unwrap_or_default()}else if source=="model"{cache["offers"].as_object().map(|m|m.values().cloned().collect::<Vec<_>>()).unwrap_or_default().into_iter().chain(cache["releases"].as_array().cloned().unwrap_or_default()).collect()}else{return Err("资讯来源无效".into());};
 let mut seen=std::collections::HashSet::new();let mut out=vec![];
 for id in ids{if !seen.insert(id){return Err("同一批次不能重复选择资讯".into());}let row=rows.iter().find(|r|r["id"].as_str()==Some(id)).ok_or("资讯已更新或移除，请重新选择")?;
  let title=text(row,"title");if title.trim().is_empty()||title.chars().count()>500{return Err("资讯标题无效".into());}
  let body=if source=="news"{format!("{}\n\n来源：{}\n新闻所属日期：{}\n原文发布时间：{}\n{}\n{}",text(row,"summary"),text(row,"source"),cache["date"].as_str().unwrap_or("来源未提供"),row["publishedAt"].as_str().unwrap_or("来源未提供"),text(row,"permalink"),text(row,"sourceUrl"))}else{
   let conditions=row["conditions"].as_array().map(|a|a.iter().map(|c|format!("{}\n依据：{}",text(c,"text"),text(c,"evidence"))).collect::<Vec<_>>().join("\n")).unwrap_or_default();
   format!("模型：{}\n平台：{}\n期限：{}\n报道日期：{}\n\n{}\n\n免费条件：\n{}\n截止依据：{}\n来源：{}\n{}",text(row,"model"),text(row,"platform"),row["deadline"].as_str().unwrap_or("期限待确认"),text(row,"issueDate"),text(row,"evidence"),conditions,text(row,"deadlineEvidence"),text(row,"issueUrl"),text(row,"sourceUrl"))
  };
  if body.len()>formats::MAX_BODY{return Err("资讯正文超过保存上限".into());}let entry=formats::entry(id.clone(),title.clone(),body.clone(),formats::plain_document(&body));
  out.push((entry,Selection{entry_id:id.clone(),title,kind:kind.to_owned()}));
 }Ok(out)
}
#[tauri::command]
pub async fn collect_news(source:String,ids:Vec<String>,kind:String,preferences:Value,app:tauri::AppHandle)->Result<Value,String>{
 let cache=if source=="news"{serde_json::to_value(crate::news::get_news_cache(app.state::<Database>())?.ok_or("暂无新闻缓存")?).map_err(|_|"新闻缓存无效")?}else if source=="model"{crate::model_news::get_model_news(app.clone())?}else{return Err("资讯来源无效".into());};
 let selected=prepare(&cache,&source,&ids,&kind)?;
 let guard=crate::document_import::begin_external_save()?;
 let result=tauri::async_runtime::spawn_blocking(move||{let _guard=guard;let db=app.state::<Database>();let mut conn=db.0.lock().map_err(|_|"数据库不可用")?;let backup=crate::backup::before_document_import(&conn,preferences,&app.state::<crate::data_root::DataRoot>().0)?;let items=insert_batch(&mut conn,&selected)?;drop(conn);let _=app.emit("content-collected",true);Ok::<_,String>(json!({"items":items,"backupPath":backup}))}).await.map_err(|_|"资讯保存任务异常，请检查结果")?;
 result
}
#[cfg(test)]mod tests{use super::*;#[test]fn missing_date_and_duplicate(){let c=json!({"items":[{"id":"a","title":"资讯","summary":"摘要","publishedAt":null}]});let s=prepare(&c,"news",&["a".into()],"sticky").unwrap();assert!(s[0].0.body.contains("来源未提供"));assert!(prepare(&c,"news",&["a".into(),"a".into()],"note").is_err());assert!(prepare(&c,"news",&["bad".into()],"note").is_err());}}
