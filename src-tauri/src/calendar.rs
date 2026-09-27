use crate::database::Database;
use chrono::{NaiveDate,Duration,Utc};
use regex::Regex;
use scraper::{Html,Selector};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use tauri::Manager;
use rusqlite::OptionalExtension;
const SEED:&str=include_str!("../data/holidays-2026.json");
fn err(e:impl std::fmt::Display)->String{e.to_string()}
pub(crate) fn parse(year:i32,html:&str,source:&str)->Result<Value,String>{
    let doc=Html::parse_document(html);let selector=Selector::parse("p").unwrap();
    let text=doc.root_element().text().collect::<String>();let compact=text.split_whitespace().collect::<String>();
    if !compact.contains(&format!("国务院办公厅关于{year}年"))||!compact.contains("部分节假日安排"){return Err("页面不是指定年度国务院放假安排通知".into());}
    let parens=Regex::new(r"（[^）]*）").unwrap();let dates=Regex::new(r"(?:(\d{1,2})月)?(\d{1,2})日").unwrap();
    let mut days=serde_json::Map::new();let mut sections=0;
    for p in doc.select(&selector){let raw=p.text().collect::<String>();let t=parens.replace_all(&raw,"");
        if !t.contains('：')||!t.contains("放假")||!t.contains('、'){continue;}
        let (heading,body)=t.split_once('：').unwrap();let name=heading.split('、').last().unwrap_or(heading);
        if !["元旦","春节","清明节","劳动节","端午节","中秋节","国庆节"].contains(&name.trim()){continue;}
        let rest=body.split("放假").next().unwrap_or("");let mut month=0;let mut ds=Vec::new();
        for c in dates.captures_iter(rest){if let Some(m)=c.get(1){month=m.as_str().parse::<u32>().map_err(err)?;}let d=c[2].parse::<u32>().map_err(err)?;ds.push((month,d));}
        if ds.is_empty()||ds.len()>2{return Err("放假日期区间无法可靠解析，保留旧版数据".into());}
        let end=ds[ds.len()-1];let start=ds[0];let sy=if start.0>end.0{year-1}else{year};
        let mut day=NaiveDate::from_ymd_opt(sy,start.0,start.1).ok_or("放假开始日期无效")?;let last=NaiveDate::from_ymd_opt(year,end.0,end.1).ok_or("放假结束日期无效")?;
        if (last-day).num_days()<0||(last-day).num_days()>15{return Err("放假区间异常".into());}
        while day<=last{days.insert(day.to_string(),json!({"type":"rest","name":name}));day+=Duration::days(1);}
        for sentence in body.split('。').filter(|s|s.contains("上班")){let mut month=0;for c in dates.captures_iter(sentence){if let Some(m)=c.get(1){month=m.as_str().parse().map_err(err)?;}let day=NaiveDate::from_ymd_opt(year,month,c[2].parse().map_err(err)?).ok_or("调休日期无效")?;if days.contains_key(&day.to_string()){return Err("休班日期冲突".into());}days.insert(day.to_string(),json!({"type":"work","name":format!("{name}调休")}));}}
        sections+=1;
    }
    if sections<6||days.len()<20{return Err("年度通知解析不完整，未替换已核验数据".into());}
    let version=Regex::new(r"国办发明电〔\d{4}〕\d+号").unwrap().find(&compact).map(|m|m.as_str().to_string()).ok_or("缺少通知版本文号")?;
    Ok(json!({"year":year,"source":source,"version":version,"verifiedAt":Utc::now().to_rfc3339(),"sourceHash":format!("{:x}",Sha256::digest(html.as_bytes())),"days":days}))
}
#[tauri::command]
pub fn get_holidays(year:i32,database:tauri::State<'_,Database>)->Result<Option<Value>,String>{
    let conn=database.0.lock().map_err(err)?;let raw:Option<String>=conn.query_row("SELECT payload FROM news_cache WHERE channel=?1",[format!("calendar-{year}")],|r|r.get(0)).optional().map_err(err)?;
    if let Some(raw)=raw {let v:Value=serde_json::from_str(&raw).map_err(|_|"休班缓存损坏，请更新官方安排")?;if v["year"]!=year||!v["days"].is_object(){return Err("休班缓存结构无效".into());}return Ok(Some(v));}
    if year==2026 {Ok(Some(serde_json::from_str(SEED).map_err(err)?))}else{Ok(None)}
}
#[tauri::command]
pub async fn refresh_holidays(year:i32,source:Option<String>,app:tauri::AppHandle)->Result<Value,String>{
    tauri::async_runtime::spawn_blocking(move||{
        if !(1900..=2100).contains(&year){return Err("年份超出支持范围".into());}
        let source=source.filter(|s|!s.trim().is_empty()).or_else(||if year==2026{Some("https://www.gov.cn/zhengce/zhengceku/202511/content_7047091.htm".into())}else{None}).ok_or("本年尚无已核验来源；可在下方输入中国政府网年度通知地址")?;
        let url=reqwest::Url::parse(&source).map_err(err)?;if url.scheme()!="https"||url.host_str()!=Some("www.gov.cn"){return Err("年度来源仅接受 https://www.gov.cn 官方通知链接".into());}
        let client=reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(err)?;
        let response=client.get(&source).header("User-Agent","Mozilla/5.0 Qingjian/0.7").send().map_err(|e|format!("官方安排更新失败：{e}"))?.error_for_status().map_err(err)?;
        if response.url().host_str()!=Some("www.gov.cn"){return Err("官方来源跳转至未知站点，停止更新".into());}
        use std::io::Read;let mut bytes=Vec::new();response.take(2_000_001).read_to_end(&mut bytes).map_err(err)?;if bytes.len()>2_000_000{return Err("官方响应过大".into());}
        let html=String::from_utf8(bytes).map_err(err)?;let value=parse(year,&html,&source)?;
        let db=app.state::<Database>();db.0.lock().map_err(err)?.execute("INSERT INTO news_cache(channel,payload) VALUES(?1,?2) ON CONFLICT(channel) DO UPDATE SET payload=excluded.payload",[format!("calendar-{year}"),serde_json::to_string(&value).map_err(err)?]).map_err(err)?;Ok(value)
    }).await.map_err(err)?
}
