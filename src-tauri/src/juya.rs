use chrono::{DateTime,Utc};
use quick_xml::{Reader,events::Event};
use scraper::{Html,Selector,ElementRef};
use serde::{Serialize,Deserialize};
use sha2::{Sha256,Digest};
use std::collections::{HashMap,HashSet};

pub const URL:&str="https://daily.juya.uk/rss.xml";
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct Story {pub id:String,pub title:String,pub issue_title:String,pub published_at:String,pub source:String,pub url:String,pub body:String,pub links:Vec<String>,pub reason:String,pub eligible:bool, pub evidence_url:Option<String>,pub quota_tokens:Option<u64>,pub period:Option<String>}
fn plain(s:&str)->String{Html::parse_fragment(s).root_element().text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")}
fn public_link(s:&str)->bool{reqwest::Url::parse(s).ok().is_some_and(|u|u.scheme()=="https"&&u.host_str().is_some_and(|h|h.contains('.')&&!h.ends_with(".local")&&h.parse::<std::net::IpAddr>().is_err()))}
// 只把免费 API 线索列入筛选依据；新闻中的上下文长度、美元试用金不能当作免费权益。
fn reason(text:&str)->String{
    let quota=regex::Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(亿|万|million|billion)?\s*(?:免费\s*)?tokens?").unwrap();
    let mut max=0f64;
    for sentence in text.split(['。','；','\n']){if !sentence.contains("免费"){continue;}for c in quota.captures_iter(sentence){let n=c[1].parse::<f64>().unwrap_or(0.);let unit=c.get(2).map(|m|m.as_str().to_lowercase()).unwrap_or_default();let n=n*match unit.as_str(){"亿"=>1e8,"万"=>1e4,"million"=>1e6,"billion"=>1e9,_=>1.};max=max.max(n);}}
    if max>0.&&max<=100_000_000.{return "来源给出的免费 Token 数未超过一亿；不累加其他模型或未来月份".into();}
    if max==0.{return "未取得可核实的单用户超过一亿 Token 免费额度；免费调用、金额或速率限制不能代替额度证明".into();}
    "来源提到较大 Token 数，但尚未核实单用户额度、适用周期及官方领取条件，不列为达标结果".into()
}
pub fn parse(xml:&str)->Result<(usize,Vec<Story>),String>{
    let mut r=Reader::from_str(xml);let mut fields=HashMap::<String,String>::new();let mut field=String::new();let mut in_item=false;let mut entries=Vec::new();let mut rss=false;
    loop{match r.read_event().map_err(|e|format!("RSS 格式错误：{e}"))?{
        Event::Start(e)=>{let n=String::from_utf8_lossy(e.name().as_ref()).to_string();if n=="rss"{rss=true;}if n=="item"{in_item=true;fields.clear();}else if in_item{field=n;fields.entry(field.clone()).or_default();}},
        Event::Text(e) if in_item=>{let s=e.decode().map_err(|e|e.to_string())?;fields.entry(field.clone()).or_default().push_str(&s);},
        Event::CData(e) if in_item=>{let s=e.decode().map_err(|e|e.to_string())?;fields.entry(field.clone()).or_default().push_str(&s);},
        Event::GeneralRef(e) if in_item=>{let s=format!("&{};",String::from_utf8_lossy(e.as_ref()));let s=quick_xml::escape::unescape(&s).map_err(|e|e.to_string())?;fields.entry(field.clone()).or_default().push_str(&s);},
        Event::End(e)=>{if e.name().as_ref()==b"item"{in_item=false;entries.push(fields.clone());}field.clear();},
        Event::DocType(_)=>return Err("RSS 不支持文档实体声明".into()),Event::Eof=>break,_=>{}
    }}
    if !rss||entries.is_empty(){return Err("RSS 未返回可解析的文章，不能判定为零结果".into());}
    let count=entries.len();let mut out=Vec::new();let mut seen=HashSet::new();
    let heading=Selector::parse("h3").unwrap();let anchors=Selector::parse("a[href]").unwrap();
    for e in entries{
        let url=e.get("link").filter(|s|public_link(s)).ok_or("RSS 原文链接缺失或无效")?;
        let date=e.get("pubDate").ok_or("RSS 发布日期缺失")?;let date=DateTime::parse_from_rfc2822(date).map_err(|_|"RSS 发布日期无效")?.with_timezone(&Utc).to_rfc3339();
        let issue=e.get("title").cloned().unwrap_or_default();let body=e.get("content:encoded").or_else(||e.get("description")).ok_or("RSS 正文和摘要均缺失")?;
        let doc=Html::parse_fragment(body);let mut blocks=Vec::new();
        for h in doc.select(&heading){if h.select(&anchors).next().is_none(){continue;}let title=h.select(&anchors).next().map(|a|a.text().collect::<String>()).unwrap_or_default();let mut lines=Vec::new();let mut links=h.select(&anchors).filter_map(|a|a.value().attr("href")).filter(|s|public_link(s)).map(str::to_string).collect::<Vec<_>>();
            for n in h.next_siblings(){if let Some(el)=ElementRef::wrap(n){if matches!(el.value().name(),"h2"|"h3"){break;}if matches!(el.value().name(),"script"|"style"){continue;}let t=plain(&el.html());if !t.is_empty(){lines.push(t);}links.extend(el.select(&anchors).filter_map(|a|a.value().attr("href")).filter(|s|public_link(s)).map(str::to_string));}}
            blocks.push((title,lines.join("\n\n"),links));
        }
        if blocks.is_empty(){blocks.push((issue.clone(),plain(body),Vec::new()));}
        for (title,body,mut links) in blocks{
            let all=format!("{title}\n{body}");let lower=all.to_lowercase();
            if !(all.contains("免费")||all.contains("赠送"))||!(lower.contains("api")||all.contains("模型接口")){continue;}
            links.sort();links.dedup();let id=format!("{:x}",Sha256::digest(format!("{url}\n{title}").as_bytes()));if !seen.insert(id.clone()){continue;}
            out.push(Story{id,title,issue_title:issue.clone(),published_at:date.clone(),source:"橘鸦 AI 早报".into(),url:url.clone(),reason:reason(&all),body,links,eligible:false,evidence_url:None,quota_tokens:None,period:None});
        }
    }
    out.sort_by(|a,b|b.published_at.cmp(&a.published_at));Ok((count,out))
}
// 仅自动确认措辞明确的「每用户每月免费额度」。限时、组织额度等复杂规则保留待核验，不能推算。
fn monthly_claim(text:&str)->Option<u64>{
    let re=regex::Regex::new(r"(?i)([0-9]+(?:\.[0-9]+)?)\s*(亿|万|million|billion)?\s*(?:免费\s*)?tokens?").ok()?;
    for sentence in text.split(['。','；','\n']){
        if !sentence.contains("免费")||!sentence.contains("每月")||!["每位用户","每个用户","单用户","每人","每个账号"].iter().any(|s|sentence.contains(s))||["上下文","速率","付费","共享","最高","最多"].iter().any(|s|sentence.contains(s)){continue;}
        for c in re.captures_iter(sentence){let n=c[1].parse::<f64>().ok()?;let scale=match c.get(2).map(|x|x.as_str()).unwrap_or(""){"亿"=>1e8,"万"=>1e4,"million"=>1e6,"billion"=>1e9,_=>1.};let n=(n*scale)as u64;if n>100_000_000{return Some(n);}}
    }None
}
pub fn verify_candidates(stories:&mut[Story]){
    for s in stories {let Some(quota)=monthly_claim(&s.body)else{continue;};
        // 官方证据域名使用显式适配清单，RSS 中任意网址不能成为内部网络抓取入口。
        for link in s.links.iter().take(8){let Ok(url)=reqwest::Url::parse(link)else{continue;};let host=url.host_str().unwrap_or("");
            if !["longcat.chat","siliconflow.cn","docs.siliconflow.cn","help.aliyun.com","console.groq.com"].contains(&host){continue;}
            if let Ok(html)=crate::free_api::fetch(link){let text=plain(&html);if monthly_claim(&text)==Some(quota)&&text.to_lowercase().contains("api"){
                s.eligible=true;s.evidence_url=Some(link.clone());s.quota_tokens=Some(quota);s.period=Some("单用户每月；截止时间以原文为准".into());s.reason="橘鸦内容与适配的官方来源均明确同一单用户月免费额度；领取条件见原文".into();break;
            }}
        }
    }
}
#[cfg(test)]mod tests{use super::*;
#[test]fn quota_is_not_context_or_money(){assert!(reason("每人500万免费token").contains("未超过"));assert!(reason("免费API提供$200试用金").contains("未取得"));}
#[test]fn invalid_is_not_empty(){assert!(parse("<html>Access denied</html>").is_err());}
#[test]fn feed_duplicates_and_irrelevant_are_filtered(){let item=r#"<item><title>日报</title><link>https://daily.juya.uk/issues/test/</link><pubDate>Sat, 26 Sep 2026 02:08:01 GMT</pubDate><description><![CDATA[<h3><a href="https://example.org/api">免费API试用</a></h3><p>每人500万免费token</p><h3><a href="https://example.org/news">其他新闻</a></h3><p>新闻正文</p>]]></description></item>"#;let(n,s)=parse(&format!("<rss><channel>{item}{item}</channel></rss>")).unwrap();assert_eq!(n,2);assert_eq!(s.len(),1);assert!(!s[0].body.contains("新闻正文"));assert!(!s[0].eligible);}
}
