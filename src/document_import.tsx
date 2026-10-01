import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {ItemKind} from './item_workspace';
type Entry={id:string;title:string;bodyPreview:string;previewTruncated:boolean;error:string|null};
type FilePreview={id:string;fileName:string;format:string;entries:Entry[];warnings:string[];error:string|null;fields:string[];canMap:boolean;canAsText:boolean};
type Preview={requestId:string;files:FilePreview[]};
type Choice={title:string;kind:ItemKind;selected:boolean};
type Mapping={titleField:string;bodyField:string;asText:boolean};
export type DocumentImportResult={items:{id:string;kind:ItemKind}[];sticky:number;note:number;backupPath:string};
type Props={defaultKind:ItemKind;preferences:{theme:string;fontSize:string};onClose:()=>void;onImported:()=>Promise<void>;onView:(id:string)=>void;onSaving:(busy:boolean)=>void};

export function DocumentImportDialog({defaultKind,preferences,onClose,onImported,onView,onSaving}:Props){
 const [preview,setPreview]=useState<Preview|null>(null),[choices,setChoices]=useState<Record<string,Choice>>({}),[mappings,setMappings]=useState<Record<string,Mapping>>({});
 const [target,setTarget]=useState(defaultKind),[phase,setPhase]=useState<'pick'|'choosing'|'parsing'|'mapping'|'preview'|'saving'|'done'>('pick');
 const [error,setError]=useState(''),[partialAccepted,setPartialAccepted]=useState(false),[result,setResult]=useState<DocumentImportResult|null>(null);
 const request=useRef<string|null>(null),alive=useRef(true),dialog=useRef<HTMLElement>(null),closeRef=useRef<()=>void>(()=>{});
 const busy=['choosing','parsing','mapping','saving'].includes(phase);
 const close=()=>{if(phase==='saving')return;alive.current=false;const id=request.current;request.current=null;if(id)void invoke('cancel_document_import',{requestId:id}).catch(()=>{});onClose();};closeRef.current=close;
 useEffect(()=>{
  alive.current=true;dialog.current?.querySelector<HTMLButtonElement>('button')?.focus();
  const keys=(e:KeyboardEvent)=>{
   if(e.ctrlKey&&e.key.toLowerCase()==='s'){e.preventDefault();e.stopImmediatePropagation();return;}
   if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();closeRef.current();}
   if(e.key==='Tab'){const buttons=[...(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),summary')??[])].filter(el=>el.getClientRects().length);const first=buttons[0],last=buttons.at(-1);if(e.shiftKey&&document.activeElement===first){e.preventDefault();last?.focus();}else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first?.focus();}}
  };
  document.addEventListener('keydown',keys,true);
  return()=>{alive.current=false;document.removeEventListener('keydown',keys,true);const id=request.current;if(id)void invoke('cancel_document_import',{requestId:id}).catch(()=>{});};
 },[]);
 const accept=(next:Preview,resetFile?:string)=>{
  setPreview(next);setChoices(old=>Object.fromEntries(next.files.flatMap(f=>f.entries.map(e=>[e.id,(!resetFile||resetFile===f.id||!old[e.id])?{title:e.title,kind:target,selected:!f.error&&!e.error}:old[e.id]]))));setPartialAccepted(false);
 };
 const choose=async()=>{
  setError('');setPhase('choosing');
  try{const paths=await invoke<string[]|null>('choose_document_files');if(!alive.current)return;if(!paths?.length){setPhase(preview?'preview':'pick');return;}
   const old=request.current;if(old)await invoke('cancel_document_import',{requestId:old});const id=crypto.randomUUID();request.current=id;setPhase('parsing');setPreview(null);setResult(null);setChoices({});setMappings({});
   const next=await invoke<Preview>('preview_documents',{requestId:id,paths});if(!alive.current||request.current!==id)return;accept(next);setPhase('preview');
  }catch(e){if(alive.current){setError(String(e));setPhase(preview?'preview':'pick');}}
 };
 const remap=async(f:FilePreview)=>{
  const id=request.current;if(!id)return;setError('');setPhase('mapping');const m=mappings[f.id]??{titleField:'',bodyField:'',asText:false};
  try{const next=await invoke<Preview>('remap_document_json',{requestId:id,fileId:f.id,titleField:m.titleField||null,bodyField:m.bodyField||null,asText:m.asText});if(alive.current&&request.current===id){accept(next,f.id);setPhase('preview');}}catch(e){if(alive.current){setError(String(e));setPhase('preview');}}
 };
 const setMapping=(id:string,next:Partial<Mapping>)=>setMappings(old=>({...old,[id]:{...(old[id]??{titleField:'',bodyField:'',asText:false}),...next}}));
 const valid=preview?.files.flatMap(f=>f.error?[]:f.entries.filter(e=>!e.error))??[];
 const invalid=preview?.files.flatMap(f=>f.entries.filter(e=>!!e.error)).length??0;
 const failedFiles=preview?.files.filter(f=>!!f.error).length??0;
 const selected=valid.filter(e=>choices[e.id]?.selected);
 const hasInvalid=invalid>0||failedFiles>0;
 const confirm=async()=>{
  const id=request.current;if(!id||!selected.length||busy||(hasInvalid&&!partialAccepted))return;
  setPhase('saving');onSaving(true);setError('');
  try{const next=await invoke<DocumentImportResult>('commit_documents',{requestId:id,selections:selected.map(e=>({entryId:e.id,title:choices[e.id].title,kind:choices[e.id].kind})),preferences});await onImported();if(alive.current){setResult(next);setPhase('done');}}
  catch(e){if(alive.current){setError(String(e));setPhase('preview');}}finally{onSaving(false);}
 };
 const chooseAll=(selected:boolean)=>setChoices(old=>{const next={...old};for(const e of valid)next[e.id]={...next[e.id],selected};return next;});
 return <div className="dialog-backdrop document-import-backdrop"><section ref={dialog} className="document-import-dialog" role="dialog" aria-modal="true" aria-labelledby="document-import-title">
  <header className="document-import-heading"><h2 id="document-import-title">导入文档</h2><button aria-label="关闭导入" disabled={phase==='saving'} onClick={close}>×</button></header>
  <div className="document-import-content">
   <p className="import-intro">TXT、Markdown、Word (.docx) 和 JSON · 本地处理</p>
   <details className="import-limits"><summary>文件与格式限制</summary><p>最多 20 个文件；单文件 10 MiB，总计 30 MiB；最多 200 条，每条正文 500 KiB。富文本最多 20,000 个节点、2 MB；解析后正文总量 32 MiB。DOCX 解压最多 32 MiB、2,000 个包内文件，单 XML 16 MiB。结构深度最多 64；JSON 最多 100,000 个值，每个对象 100 个字段、字段名 128 字。TXT 仅支持 UTF-8 / UTF-8 BOM。不支持 .doc、加密文件或宏文件。图片及附件不自动下载。</p></details>
   {!result&&<div className="import-toolbar"><button disabled={busy} onClick={()=>void choose()}>{preview?'重新选择文件':'选择文件'}</button><label>默认目标<select aria-label="默认导入栏目" disabled={busy} value={target} onChange={e=>{const kind=e.target.value as ItemKind;setTarget(kind);setChoices(old=>Object.fromEntries(Object.entries(old).map(([id,c])=>[id,{...c,kind}])));}}><option value="sticky">便签</option><option value="note">笔记</option></select></label></div>}
   {busy&&<p className="import-processing" role="status">{phase==='choosing'?'请选择文件…':phase==='parsing'?'正在解析本地文件…':phase==='mapping'?'正在重新生成预览…':'正在备份并保存…'}</p>}
   {error&&<p role="alert" className="error-text">{error}</p>}
   {preview&&!result&&<>
    <p className="import-counts" role="status">可导入 {valid.length} 条 · 无法导入 {invalid} 条{failedFiles?` · ${failedFiles} 个文件解析失败`:''}</p>
    <div className="import-select-actions"><button disabled={busy||!valid.length} onClick={()=>chooseAll(true)}>全选有效条目</button><button disabled={busy} onClick={()=>chooseAll(false)}>取消全选</button><small>已选 {selected.length} 条</small></div>
    {preview.files.map(f=><section className="import-file" key={f.id} data-file-name={f.fileName}><h3>{f.fileName}<small>{f.format.toUpperCase()}</small></h3>
     {f.error&&<p role="alert" className="error-text">{f.error}</p>}
     {f.warnings.length>0&&<details className="import-warnings"><summary>格式说明 · {f.warnings.length} 项</summary><ul>{f.warnings.map(w=><li key={w}>{w}</li>)}</ul></details>}
     {f.format==='json'&&(f.canMap||f.canAsText)&&<details className="import-mapping" open={!!f.error||f.entries.some(e=>!!e.error)}><summary>JSON 字段映射与文本导入</summary>
      {f.canMap&&<><label>标题字段<select aria-label={f.fileName+'标题字段'} disabled={busy||mappings[f.id]?.asText} value={mappings[f.id]?.titleField??''} onChange={e=>setMapping(f.id,{titleField:e.target.value})}><option value="">标准 title／默认标题</option>{f.fields.map(k=><option key={k} value={k}>{k}</option>)}</select></label><label>正文字段<select aria-label={f.fileName+'正文字段'} disabled={busy||mappings[f.id]?.asText} value={mappings[f.id]?.bodyField??''} onChange={e=>setMapping(f.id,{bodyField:e.target.value})}><option value="">标准 content／body</option>{f.fields.map(k=><option key={k} value={k}>{k}</option>)}</select></label></>}
      {f.canAsText&&<label className="import-check"><input type="checkbox" aria-label={f.fileName+'作为格式化文本'} disabled={busy} checked={mappings[f.id]?.asText??false} onChange={e=>setMapping(f.id,{asText:e.target.checked})}/>作为一条格式化文本导入</label>}
      <small>重新应用会重置本文件的标题与勾选。未使用字段不导入。</small><button disabled={busy} onClick={()=>void remap(f)}>应用映射</button>
     </details>}
     {f.entries.map(e=>{const c=choices[e.id];return <article className={'import-entry '+(e.error?'invalid':'')} key={e.id} data-entry-id={e.id}>
      <label className="import-check"><input type="checkbox" aria-label={'导入 '+e.title} disabled={busy||!!e.error||!!f.error} checked={c?.selected??false} onChange={v=>setChoices(old=>({...old,[e.id]:{...old[e.id],selected:v.target.checked}}))}/><span>{e.error?'无法导入':'导入此条'}</span></label>
      <label>标题<input aria-label="导入标题" maxLength={500} disabled={busy||!!e.error} value={c?.title??e.title} onChange={v=>setChoices(old=>({...old,[e.id]:{...old[e.id],title:v.target.value}}))}/></label>
      <label>目标栏目<select aria-label="条目目标栏目" disabled={busy||!!e.error} value={c?.kind??target} onChange={v=>setChoices(old=>({...old,[e.id]:{...old[e.id],kind:v.target.value as ItemKind}}))}><option value="sticky">便签</option><option value="note">笔记</option></select></label>
      <pre className="import-body-preview">{e.bodyPreview||'（无正文）'}</pre>{e.previewTruncated&&<small>仅预览前 1,200 字符，完整正文按所选格式导入。</small>}{e.error&&<p className="error-text">{e.error}</p>}
     </article>;})}
    </section>)}
    {!valid.length&&<p className="error-text">没有可导入内容，请检查上述原因或调整 JSON 映射。</p>}
    {hasInvalid&&valid.length>0&&<label className="import-check import-partial"><input type="checkbox" aria-label="仅导入有效条目" disabled={busy} checked={partialAccepted} onChange={e=>setPartialAccepted(e.target.checked)}/>仅导入已勾选的有效条目，跳过无效内容</label>}
   </>}
   {result&&<div className="import-result" role="status"><h3>导入完成</h3>{result.sticky>0&&<p>已导入 {result.sticky} 条便签</p>}{result.note>0&&<p>已导入 {result.note} 条笔记</p>}
    <div className="import-toolbar">{(['sticky','note'] as const).map(k=>{const item=result.items.find(i=>i.kind===k);return item&&<button key={k} onClick={()=>onView(item.id)}>查看{k==='sticky'?'便签':'笔记'}</button>;})}</div>
    <details><summary>导入前内容备份</summary><p>{result.backupPath}</p><small>可从设置 → 文件中的备份恢复入口读取。</small></details>
   </div>}
  </div>
  <footer className="import-footer"><button disabled={phase==='saving'} onClick={close}>{result?'完成':phase==='parsing'||phase==='mapping'?'取消解析':'取消'}</button>{!result&&<button className="save-button" disabled={busy||!selected.length||(hasInvalid&&!partialAccepted)} onClick={()=>void confirm()}>确认导入{selected.length?` ${selected.length} 条`:''}</button>}</footer>
 </section></div>;
}
