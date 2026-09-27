use crate::database::Database;
use chrono::Utc;
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use scraper::{Html,Selector};
use sha2::{Digest,Sha256};
use rusqlite::OptionalExtension;
use tauri::Manager;
use std::{io::Read,time::Duration,sync::Mutex};
static FETCH:Mutex<()>=Mutex::new(());
const GROQ:&str="https://console.groq.com/docs/rate-limits";
const ALI:&str="https://help.aliyun.com/zh/model-studio/new-free-quota";
#[derive(Serialize,Deserialize,Clone)]
#[serde(rename_all="camelCase")]
struct Candidate {provider:String,model:String,endpoint:String,quota_tokens:Option<u64>,period:String,conditions:String,valid_until:Option<String>,evidence_url:String,verified_at:String,basis:String,reason:String,eligible:bool}
fn err(e:impl std::fmt::Display)->String{e.to_string()}
fn number(s:&str)->Option<u64>{let s=s.trim().replace(',',"");let (n,m)=if let Some(n)=s.strip_suffix('K'){(n,1000.0)}else if let Some(n)=s.strip_suffix('M'){(n,1_000_000.0)}else{(s.as_str(),1.0)};n.parse::<f64>().ok().map(|v|(v*m)as u64)}
pub(crate) fn fetch(url:&str)->Result<String,String>{
    let c=reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).connect_timeout(Duration::from_secs(8)).build().map_err(err)?;
    let r=c.get(url).header("User-Agent","Qingjian-source-check/1.0").send().map_err(err)?;if !r.status().is_success(){return Err(format!("HTTP {}",r.status()));}
    let mut bytes=Vec::new();r.take(2_000_001).read_to_end(&mut bytes).map_err(err)?;if bytes.len()>2_000_000{return Err("响应超过2MB".into());}String::from_utf8(bytes).map_err(err)
}
fn parse_groq(html:&str)->Result<Vec<Candidate>,String>{
    let doc=Html::parse_document(html);let table=Selector::parse("table").unwrap();let tr=Selector::parse("tr").unwrap();let td=Selector::parse("td").unwrap();
    let table=doc.select(&table).find(|t|t.text().collect::<String>().contains("Free Plan Limits")).ok_or("未找到免费方案表格，来源结构可能变化")?;
    let mut out=Vec::new();for row in table.select(&tr){let cells=row.select(&td).map(|x|x.text().collect::<String>()).collect::<Vec<_>>();if cells.len()!=7||cells[0].trim().is_empty(){continue;}
        let tokens=number(&cells[4]);let reason=if tokens.is_none(){"无 Token 额度，可能是音频等接口，不符合文本大模型免费额度条件"}else{"TPD 是组织级每日速率限制，不是单用户单月或明确有效期的免费总额度；不能累加推算"};
        out.push(Candidate{provider:"Groq".into(),model:cells[0].trim().into(),endpoint:"https://api.groq.com/openai/v1".into(),quota_tokens:tokens,period:"TPD（每日速率上限，非可领取额度）".into(),conditions:"需注册 Groq 免费账户；限额按组织计算".into(),valid_until:None,evidence_url:GROQ.into(),verified_at:Utc::now().to_rfc3339(),basis:format!("官方免费方案表：TPD={}",cells[4]),reason:reason.into(),eligible:false});
    }if out.is_empty(){return Err("官方免费方案表为空，不能认定零结果".into());}Ok(out)
}
fn parse_ali(html:&str)->Result<Vec<Candidate>,String>{
    let doc=Html::parse_document(html);let text=doc.root_element().text().collect::<String>();
    let re=regex::Regex::new(r"通常为\s*(\d+)\s*万\s*Token").unwrap();let quota=re.captures(&text).and_then(|c|c[1].parse::<u64>().ok()).map(|n|n*10000);
    if !text.contains("免费额度")||quota.is_none()||!text.contains("90"){return Err("官方免费额度规则无法可靠解析，需重新核验来源结构".into());}
    let reason=if quota.unwrap()<=100_000_000{"官方通常每模型额度不超过一亿；各模型独立且不能合并转移，未证实单用户可领取超过一亿的完整方案"}else{"页面是通常额度说明，缺少具体可调用模型与可领取总额度清单，暂不收录"};
    Ok(vec![Candidate{provider:"阿里云百炼".into(),model:"新人额度规则（具体模型待逐项核验）".into(),endpoint:"https://dashscope.aliyuncs.com/compatible-mode/v1".into(),quota_tokens:quota,period:"90 天；各模型独立额度".into(),conditions:"首次开通，指定地域模型；起算日以开通、模型发布或申请通过的较晚日期为准".into(),valid_until:None,evidence_url:ALI.into(),verified_at:Utc::now().to_rfc3339(),basis:format!("官方新人额度：通常 {} Token / 模型，90 天",quota.unwrap()),reason:reason.into(),eligible:false}])
}
#[tauri::command]
pub fn get_free_api_cache(database:tauri::State<'_,Database>)->Result<Option<Value>,String>{
    let c=database.0.lock().map_err(err)?;let raw:Option<String>=c.query_row("SELECT payload FROM news_cache WHERE channel='free-api-v1'",[],|r|r.get(0)).optional().map_err(err)?;
    raw.map(|s|{let v:Value=serde_json::from_str(&s).map_err(|_|"免费 API 缓存损坏")?;if v["format"]!="qingjian-free-api-audit-v1"||!v["candidates"].is_array(){return Err("免费 API 缓存格式无效".into());}Ok(v)}).transpose()
}
#[tauri::command]
pub async fn refresh_free_api(app:tauri::AppHandle)->Result<Value,String>{
    tauri::async_runtime::spawn_blocking(move||{
        let _lock=FETCH.try_lock().map_err(|_|"免费 API 正在核验，请稍候")?;
        // 获取公开官方页面，不带用户密钥；来源内容发生变化时拒绝猜测额度。
        let results=std::thread::scope(|s|{let a=s.spawn(||fetch(GROQ));let b=s.spawn(||fetch(ALI));vec![(GROQ,a.join().unwrap_or_else(|_|Err("获取线程失败".into()))),(ALI,b.join().unwrap_or_else(|_|Err("获取线程失败".into())))]});
        let mut candidates=Vec::new();let mut sources=Vec::new();let mut failures=Vec::new();
        // RSS 独立缓存；该来源失败时不能被其他官方页面的成功覆盖成空结果。
        let rss=fetch(crate::juya::URL).and_then(|xml|crate::juya::parse(&xml));
        let mut rss_failure=None;
        let juya=match rss {Ok((count,mut articles))=>{crate::juya::verify_candidates(&mut articles);Some(json!({"fetchedAt":Utc::now().to_rfc3339(),"url":crate::juya::URL,"issueCount":count,"articles":articles}))},Err(e)=>{rss_failure=Some(e);let db=app.state::<Database>();let raw:Option<String>=db.0.lock().map_err(err)?.query_row("SELECT payload FROM news_cache WHERE channel='free-api-v1'",[],|r|r.get(0)).optional().map_err(err)?;raw.and_then(|s|serde_json::from_str::<Value>(&s).ok()).and_then(|v|v.get("juya").cloned()).filter(|v|v.is_object())}};
        for (url,result) in results {match result.and_then(|html|{let rows=if url==GROQ{parse_groq(&html)}else{parse_ali(&html)}?;Ok((html,rows))}){
            Ok((html,rows))=>{sources.push(json!({"url":url,"fetchedAt":Utc::now().to_rfc3339(),"sha256":format!("{:x}",Sha256::digest(html.as_bytes())),"checked":rows.len()}));candidates.extend(rows);},
            Err(e)=>failures.push(format!("{url}：{e}"))
        }}
        if sources.is_empty()&&juya.is_none(){return Err(format!("来源核验失败，不能判断是否存在符合条件的 API：{}",format!("{}；橘鸦 RSS：{}",failures.join("；"),rss_failure.as_deref().unwrap_or("未知错误"))));}
        // 当前适配的官方规则没有证实合格优惠；保留逐项排除原因，绝不把失败冒充零结果。
        let eligible=candidates.iter().filter(|c|c.eligible&&c.quota_tokens.is_some_and(|n|n>100_000_000)&&c.valid_until.as_ref().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()).is_some_and(|d|d>Utc::now())).cloned().collect::<Vec<_>>();
        let value=json!({"format":"qingjian-free-api-audit-v1","fetchedAt":Utc::now().to_rfc3339(),"sources":sources,"failures":failures,"candidates":candidates,"eligible":eligible,"juya":juya,"rssFailure":rss_failure});
        let db=app.state::<Database>();db.0.lock().map_err(err)?.execute("INSERT INTO news_cache(channel,payload) VALUES('free-api-v1',?1) ON CONFLICT(channel) DO UPDATE SET payload=excluded.payload",[serde_json::to_string(&value).map_err(err)?]).map_err(err)?;Ok(value)
    }).await.map_err(err)?
}

#[cfg(test)]mod tests{use super::*;#[test]fn invalid_source_is_not_zero_results(){assert!(parse_groq("<html>Access denied</html>").is_err());assert!(parse_ali("200000000 free tokens").is_err());}#[test]fn no_dollar_or_future_month_conversion(){assert_eq!(number("200K"),Some(200000));assert_eq!(number("$200"),None);}}
