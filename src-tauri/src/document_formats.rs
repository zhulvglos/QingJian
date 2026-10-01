//! 本地文档转换：只生成编辑器允许的节点，HTML、链接和图片不执行或下载。
use pulldown_cmark::{Event,Options,Parser,Tag};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::collections::BTreeSet;

pub const MAX_BODY:usize=500*1024;
pub const MAX_ENTRIES:usize=200;
pub const MAX_NODES:usize=20_000;
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct Entry {
 pub id:String,pub title:String,pub body_preview:String,pub preview_truncated:bool,pub error:Option<String>,
 #[serde(skip_serializing)]pub body:String,#[serde(skip_serializing)]pub body_json:Option<String>,
}
#[derive(Clone,Serialize)]
#[serde(rename_all="camelCase")]
pub struct FilePreview{
 pub id:String,pub file_name:String,pub format:String,pub entries:Vec<Entry>,pub warnings:Vec<String>,pub error:Option<String>,
 pub fields:Vec<String>,pub can_map:bool,pub can_as_text:bool,
 #[serde(skip)]pub raw_json:Option<Value>,
}
pub fn paragraph(text:&str)->Value{if text.is_empty(){json!({"type":"paragraph"})}else{json!({"type":"paragraph","content":[{"type":"text","text":text}]})}}
pub fn plain_document(body:&str)->Value{json!({"type":"doc","content":body.split('\n').map(paragraph).collect::<Vec<_>>()})}
fn normalize_blocks(content:Vec<Value>)->Vec<Value>{
 let mut result=vec![];let mut inline=vec![];
 for n in content{if matches!(n["type"].as_str(),Some("text"|"hardBreak")){inline.push(n);}else{if !inline.is_empty(){result.push(json!({"type":"paragraph","content":std::mem::take(&mut inline)}));}result.push(n);}}
 if !inline.is_empty(){result.push(json!({"type":"paragraph","content":inline}));}result
}
pub fn text_of(node:&Value)->String{
 match node["type"].as_str().unwrap_or(""){
  "text"=>node["text"].as_str().unwrap_or("").into(),"hardBreak"=>"\n".into(),
  kind=>node["content"].as_array().map(|v|v.iter().map(text_of).collect::<Vec<_>>().join(if matches!(kind,"doc"|"bulletList"|"orderedList"|"taskList"|"listItem"|"taskItem"|"blockquote"){"\n"}else{""})).unwrap_or_default()
 }
}
fn count_nodes(n:&Value)->usize{1+n["content"].as_array().map(|v|v.iter().map(count_nodes).sum::<usize>()).unwrap_or(0)}
pub fn entry(id:String,title:String,body:String,document:Value)->Entry{
 let mode=if document["content"].as_array().is_some_and(|v|v.iter().any(|n|n["type"]=="taskList")){"checklist"}else{"paragraph"};
 let count=count_nodes(&document);
 let stored=json!({"format":"qingjian-rich-v1","mode":mode,"checks":[],"document":document});
 let body_json=if count>MAX_NODES||body.len()>MAX_BODY{String::new()}else{stored.to_string()};
 let error=if body.trim().is_empty(){Some("正文为空，不能导入".into())}else if body.len()>MAX_BODY{Some("正文超过 500 KiB 上限".into())}else if body_json.len()>2_000_000||count>MAX_NODES{Some("正文结构超过编辑器资源上限".into())}else if title.chars().count()>500{Some("标题超过 500 字上限".into())}else{None};
 let body_preview=body.chars().take(1200).collect::<String>();let preview_truncated=body_preview.len()<body.len();
 Entry{id,title,body,body_preview,preview_truncated,body_json:if error.is_none(){Some(body_json)}else{None},error}
}
fn plain_entry(id:String,title:String,body:&str)->Entry{
 let document=if body.len()>MAX_BODY||body.bytes().filter(|b|*b==b'\n').count()>MAX_NODES{json!({"type":"doc","content":[]})}else{plain_document(body)};
 let mut e=entry(id,title,body.into(),document);
 if body.bytes().filter(|b|*b==b'\n').count()>MAX_NODES{e.error=Some("正文行数超过结构上限".into());e.body_json=None;}e
}
pub fn json_entries(raw:&Value,file_id:&str,stem:&str,title_field:Option<&str>,body_field:Option<&str>,as_text:bool)->Result<(Vec<Entry>,Vec<String>,Vec<String>,bool),String>{
 if as_text{let body=serde_json::to_string_pretty(raw).map_err(|_|"JSON 无法格式化")?;return Ok((vec![plain_entry(format!("{file_id}:text"),stem.into(),&body)],vec!["作为一条格式化文本导入，不解释任何外部状态或 ID。".into()],vec![],false));}
 let rows:Vec<&Value>=match raw{Value::Array(v)=>v.iter().collect(),Value::Object(o)=>match o.get("items"){Some(Value::Array(v))=>v.iter().collect(),Some(_)=>return Err("items 必须是数组；可主动选择作为格式化文本导入".into()),None=>vec![raw]},_=>return Err("复杂或非条目 JSON；可主动选择作为一条格式化文本导入".into())};
 if rows.len()>MAX_ENTRIES{return Err("JSON 超过 200 条上限，请拆分原文件后导入".into());}
 if rows.is_empty(){return Err("JSON 条目数组为空".into());}
 let fields:BTreeSet<String>=rows.iter().filter_map(|r|r.as_object()).flat_map(|r|r.keys().cloned()).collect();
 let mut unused=BTreeSet::new();let mut entries=vec![];let mut complex=false;
 for (i,row) in rows.iter().enumerate(){
  let default=if rows.len()==1{stem.to_string()}else{format!("{stem} · {}",i+1)};
  let title_key=title_field.unwrap_or("title");let body_key=body_field.unwrap_or(if row.get("content").is_some(){"content"}else{"body"});
  let body=row.get(body_key).and_then(Value::as_str);
  let title=row.get(title_key).and_then(Value::as_str).filter(|s|!s.trim().is_empty()).unwrap_or(&default).to_string();
  let mut e=plain_entry(format!("{file_id}:{i}"),title,body.unwrap_or(""));
  if !row.is_object(){e.error=Some("条目必须是对象；可主动作为一条格式化文本导入".into());complex=true;}
  else if row.get(body_key).is_none(){e.error=Some("缺少正文，请选择实际存在的正文字段".into());}
  else if body.is_none(){e.error=Some("正文必须是字符串，不能自动转换其他类型".into());complex|=row.get(body_key).is_some_and(|v|v.is_object()||v.is_array());}
  else if row.get(title_key).is_some_and(|v|!v.is_string()&&!v.is_null()){e.error=Some("标题字段不是字符串，请调整映射".into());}
  if let Some(o)=row.as_object(){for k in o.keys(){if k!=title_key&&k!=body_key{unused.insert(k.clone());}}}
  entries.push(e);
 }
 let warnings=if unused.is_empty(){vec![]}else{vec![format!("未使用字段不会导入：{}。原文件保持不变。",unused.into_iter().collect::<Vec<_>>().join("、"))]};
 Ok((entries,warnings,fields.into_iter().collect(),complex))
}
struct Frame{kind:&'static str,attrs:Value,content:Vec<Value>,suffix:Option<String>,mark:Option<&'static str>}
pub fn markdown(body:&str)->Result<(Value,Vec<String>),String>{
 let mut frames=vec![Frame{kind:"doc",attrs:Value::Null,content:vec![],suffix:None,mark:None}];let mut warnings=BTreeSet::new();let mut nodes=0usize;
 let options=Options::ENABLE_TASKLISTS|Options::ENABLE_TABLES|Options::ENABLE_STRIKETHROUGH;
 for event in Parser::new_ext(body,options){
  nodes+=1;if nodes>MAX_NODES||frames.len()>64{return Err("Markdown 结构超过解析上限".into());}
  match event{
   Event::Start(tag)=>{
    let mut f=Frame{kind:"",attrs:Value::Null,content:vec![],suffix:None,mark:None};
    match tag{
     Tag::Paragraph=>f.kind="paragraph",Tag::Heading{level,..}=>{f.kind="heading";f.attrs=json!({"level":level as u8});},
     Tag::List(start)=>{f.kind=if start.is_some(){"orderedList"}else{"bulletList"};if let Some(n)=start{f.attrs=json!({"start":n});}},Tag::Item=>f.kind="listItem",
     Tag::BlockQuote(_)=>f.kind="blockquote",Tag::CodeBlock(_)=>f.kind="codeBlock",
     Tag::Emphasis=>f.mark=Some("italic"),Tag::Strong=>f.mark=Some("bold"),Tag::Strikethrough=>f.mark=Some("strike"),
     Tag::Link{dest_url,..}=>{f.suffix=Some(format!(" ({dest_url})"));warnings.insert("链接保留为文字与地址，不自动访问。".to_string());},
     Tag::Image{dest_url,..}=>{f.content.push(json!({"type":"text","text":"[图片："}));f.suffix=Some(format!(" · {dest_url}]"));warnings.insert("图片仅保留说明和地址，不下载图片。".to_string());},
     Tag::Table(_)=>{f.kind="blockquote";warnings.insert("Markdown 表格转换为逐行文字。".to_string());},Tag::TableHead|Tag::TableRow=>f.kind="paragraph",Tag::TableCell=>f.suffix=Some(" | ".into()),
     Tag::HtmlBlock=>{f.kind="paragraph";warnings.insert("嵌入 HTML 按原文字显示，不执行。".to_string());},
     _=>{warnings.insert("不支持的 Markdown 结构已降级为可读文字。".to_string());}
    }frames.push(f);
   }
   Event::End(_)=>{
    let mut f=frames.pop().ok_or("Markdown 结构无效")?;
    if let Some(s)=f.suffix{f.content.push(json!({"type":"text","text":s}));}
    // Markdown 普通项目和待办可能属于同一个源列表；分组保留，不能给普通项新增方框。
    if matches!(f.kind,"bulletList"|"orderedList")&&f.content.iter().any(|n|n["type"]=="taskItem"){
     let mut groups:Vec<Value>=vec![];
     for item in f.content{let kind=if item["type"]=="taskItem"{"taskList"}else{f.kind};if groups.last().is_some_and(|n|n["type"]==kind){groups.last_mut().unwrap()["content"].as_array_mut().unwrap().push(item);}else{let mut group=json!({"type":kind,"content":[item]});if kind=="orderedList"&&!f.attrs.is_null(){group["attrs"]=f.attrs.clone();}groups.push(group);}}
     frames.last_mut().ok_or("Markdown 容器无效")?.content.extend(groups);continue;
    }
    if matches!(f.kind,"listItem"|"taskItem"){f.content=normalize_blocks(f.content);if f.content.is_empty()||f.content[0]["type"]!="paragraph"{f.content.insert(0,paragraph(""));}}
    let parent=frames.last_mut().ok_or("Markdown 容器无效")?;
    if f.kind.is_empty(){parent.content.extend(f.content);}else{let mut n=json!({"type":f.kind,"content":f.content});if !f.attrs.is_null(){n["attrs"]=f.attrs;}parent.content.push(n);}
   }
   Event::TaskListMarker(checked)=>{if let Some(item)=frames.iter_mut().rev().find(|f|f.kind=="listItem"){item.kind="taskItem";item.attrs=json!({"checked":checked});}}
   Event::Text(t)|Event::Html(t)|Event::InlineHtml(t)=>{
    let marks=frames.iter().filter_map(|f|f.mark.map(|m|json!({"type":m}))).collect::<Vec<_>>();
    if !t.is_empty(){let mut n=json!({"type":"text","text":t.as_ref()});if !marks.is_empty(){n["marks"]=json!(marks);}frames.last_mut().unwrap().content.push(n);}
   }
   Event::Code(t)=>frames.last_mut().unwrap().content.push(json!({"type":"text","text":t.as_ref(),"marks":[{"type":"code"}]})),
   Event::SoftBreak|Event::HardBreak=>{let code=frames.iter().any(|f|f.kind=="codeBlock");frames.last_mut().unwrap().content.push(if code{json!({"type":"text","text":"\n"})}else{json!({"type":"hardBreak"})});},
   Event::Rule=>frames.last_mut().unwrap().content.push(json!({"type":"horizontalRule"})),
   _=>{warnings.insert("不支持的 Markdown 标记已按可读文本处理。".to_string());}
  }
 }
 if body.contains('<'){warnings.insert("HTML 标签按文字保留，不执行嵌入代码。".into());}
 let f=frames.pop().ok_or("Markdown 无正文")?;let mut content=f.content;if content.is_empty(){content.push(paragraph(""));}
 Ok((json!({"type":"doc","content":content}),warnings.into_iter().collect()))
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn markdown_preserves_supported_nodes_and_safe_fallback(){let(doc,w)=markdown("# 标题\n\n**粗体** *强调*\n\n- 第一\n- 第二\n\n- [x] 完成\n- [ ] 未完成\n\n|甲|乙|\n|-|-|\n|一|二|\n\n![图](https://invalid/image)\n<script>alert(1)</script>").unwrap();let s=doc.to_string();assert!(s.contains("heading")&&s.contains("bold")&&s.contains("italic")&&s.contains("bulletList")&&s.contains("taskList"));assert!(!s.contains("\"type\":\"image\""));assert!(text_of(&doc).contains("alert(1)"));assert!(w.len()>=2);}
 #[test]fn json_shapes_types_mapping_and_external_ids(){for raw in [json!({"content":"中文"}),json!([{"body":"一"},{"title":"二","content":"二"}]),json!({"items":[{"title":"标题","body":"正文","id":"existing"}]})]{let(e,w,_,_)=json_entries(&raw,"local","文件",None,None,false).unwrap();assert!(e.iter().all(|r|r.error.is_none()));assert!(e.iter().all(|r|r.id.starts_with("local:")));if raw.get("items").is_some(){assert!(!w.is_empty());}}
 let raw=json!([{"name":"映射","text":"正文","flag":true},{"name":"非法","text":9},{"name":"空","text":" "}]);let(e,_,fields,_)=json_entries(&raw,"x","文件",Some("name"),Some("text"),false).unwrap();assert_eq!(e.iter().filter(|x|x.error.is_none()).count(),1);assert_eq!(fields,vec!["flag","name","text"]);
 let raw=json!({"deep":{"inside":[1,2]}});assert!(json_entries(&raw,"x","复杂",None,None,false).unwrap().0[0].error.is_some());assert!(json_entries(&raw,"x","复杂",None,None,true).unwrap().0[0].error.is_none());}
}
