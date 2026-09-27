use crate::audio;
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use tauri::Emitter;

fn chunks(text:&str,limit:usize)->Vec<String>{
    let mut result=Vec::new();let mut part=String::new();
    // 逐 Unicode 字符划分，只分段、不截断；所有原文恰好进入一个初始片段。
    for ch in text.chars(){if part.len()+ch.len_utf8()>limit&&!part.is_empty(){result.push(std::mem::take(&mut part));}part.push(ch);}
    if !part.is_empty(){result.push(part);}result
}
fn request(app:&tauri::AppHandle,id:&str,config:&Value,input:&str,merge:bool)->Result<String,String>{
    let key=format!("{:x}",Sha256::digest(format!("summary-v1|{}|{}|{merge}|{input}",config["revision"],config["model"]).as_bytes()));
    let state=audio::session(app,id)?;if let Some(text)=state["summaryChunks"][&key].as_str(){return Ok(text.into());}
    let result=crate::online::summary_chunk(config,input,merge)?;
    let mut state=audio::session(app,id)?;state["summaryChunks"][&key]=json!(result);audio::put_session(app,&state)?;Ok(result)
}
pub(crate) fn generate(app:&tauri::AppHandle,id:&str,text:&str,source_version:Option<String>,config:&Value)->Result<Value,String>{
    if text.trim().is_empty(){return Err("尚无可生成纪要的转写文字".into());}
    if text.len()>4_000_000{return Err("转写文字超过本轮支持的4MB范围；未发送、未截断，原稿保留".into());}
    let original=audio::session(app,id)?;
    let source=source_version.as_ref().and_then(|id|original["versions"].as_array()?.iter().find(|v|v["id"]==*id));
    let missing=source.map(|s|s["source"]["missingRanges"].clone()).unwrap_or(json!([]));
    let inputs=chunks(text,8000);let count=inputs.len();let mut nodes=Vec::new();
    for (index,part)in inputs.iter().enumerate(){let _=app.emit("recording-stage",json!({"id":id,"message":format!("纪要 · 处理 {}/{} 段",index+1,count)}));nodes.push(request(app,id,config,part,false)?);}
    let mut depth=0;
    while nodes.len()>1{
        depth+=1;if depth>12{return Err("模型无法在有限轮次内汇总全部内容，分段结果与原稿保留".into());}
        let all=nodes.iter().enumerate().map(|(i,n)|format!("【分段纪要 {}】\n{n}",i+1)).collect::<Vec<_>>().join("\n\n");
        let groups=chunks(&all,8000);let mut next=Vec::new();
        for(index,part)in groups.iter().enumerate(){let _=app.emit("recording-stage",json!({"id":id,"message":format!("纪要 · 第 {depth} 轮汇总 {}/{}",index+1,groups.len())}));next.push(request(app,id,config,part,true)?);}
        if next.len()>=nodes.len()&&next.iter().map(String::len).sum::<usize>()>=all.len(){return Err("文本模型未有效压缩分段内容，无法保证完整汇总；原稿及分段结果保留".into());}
        nodes=next;
    }
    let mut result=nodes.pop().ok_or("纪要结果为空")?;
    let missing_count=missing.as_array().map(Vec::len).unwrap_or(0);if missing_count>0{result=format!("> 尚有 {missing_count} 个转写片段未完成；本纪要只覆盖已有有效转写。\n\n{result}");}
    audio::add_version(app,id,"summary",result,json!({"kind":"summary","endpoint":config["endpoint"],"model":config["model"],"revision":config["revision"],"transcriptVersionId":source_version,"inputBytes":text.len(),"leafChunks":count,"coveredChunks":count,"missingRanges":missing}))
}

#[cfg(test)]mod tests{use super::*;
#[test]fn unicode_chunking_never_discards_or_duplicates_text(){let text="甲乙丙丁🙂\n原文不截断".repeat(4000);let pieces=chunks(&text,8000);assert!(pieces.len()>1);assert!(pieces.iter().all(|p|p.len()<=8000));assert_eq!(pieces.concat(),text);}
}
