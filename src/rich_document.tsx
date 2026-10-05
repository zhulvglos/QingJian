import { useLayoutEffect, useRef, useState } from 'react';
import { EditorContent, useEditor } from '@tiptap/react';
import type { JSONContent } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { TaskItem, TaskList } from '@tiptap/extension-list';
import {TaskIdentity,identifyDocument} from './task_identity';
import {NavIcon} from './ui_icons';
import {formatParagraphs} from './list_format';
import {BodyViewFilter,bodyViewKey,type BodyView} from './body_view';

type Mode = 'paragraph' | 'checklist';
type StoredDocument = { format: 'qingjian-rich-v1'; mode: Mode; document: JSONContent; checks: boolean[] };

// 四个模式由工作区统一控制；提醒时仅保留内存编辑器，正文 DOM 不渲染。
export function BodyViewTools({mode,onMode}: {mode:BodyView;onMode:(mode:BodyView)=>void}) {
  return <div className="detail-tools view-tools" role="toolbar" aria-label="正文查看" onMouseDown={e=>e.preventDefault()}>{(['all','checked','unchecked','reminder'] as const).map((value,index)=><button key={value} type="button" className={'tool-icon '+(mode===value?'active':'')} title={['全部内容','已勾选','未勾选','提醒'][index]} aria-label={['全部内容','已勾选','未勾选','提醒'][index]} aria-pressed={mode===value} onClick={()=>onMode(value)}>{value==='all'?<NavIcon name="all"/>:value==='reminder'?<NavIcon name="bell"/>:checkIcon(value==='checked')}</button>)}</div>;
}
export function ReadOnlyBody({body,bodyJson,mode}: {body:string;bodyJson:string|null;mode:BodyView}) {
  const editor=useEditor({extensions:[StarterKit,TaskIdentity,BodyViewFilter,TaskList,TaskItem.configure({nested:true})],content:identifyDocument(readDocument(body,bodyJson).document),editable:false});
  useLayoutEffect(()=>{if(editor)editor.view.dispatch(editor.state.tr.setMeta(bodyViewKey,mode));},[editor,mode]);
  const count=taskEntries(body,bodyJson).filter(t=>t.checked===(mode==='checked')).length;
  if(mode==='reminder')return null;
  return <div className="rich-body readonly-body">{mode!=='all'&&!count&&<p className="body-view-empty">暂无{mode==='checked'?'已勾选':'未勾选'}内容</p>}<EditorContent className="rich-editor" editor={editor} aria-label="已保存正文，只读" /></div>;
}

function paragraph(content?: JSONContent[]): JSONContent {
  return { type: 'paragraph', ...(content?.length ? { content } : {}) };
}

export function readDocument(body: string, bodyJson?: string | null): StoredDocument {
  if (bodyJson) {
    try {
      const stored = JSON.parse(bodyJson) as StoredDocument;
      if (stored.format === 'qingjian-rich-v1' && stored.document?.type === 'doc' && Array.isArray(stored.document.content)) return stored;
    } catch { /* 损坏时使用保留的纯文本副本，避免编辑器空白。 */ }
  }
  // 旧纯文本只在内存中解析；行首方框转为真实勾选项，原数据库正文不改写。
  const blocks: JSONContent[] = [];
  let tasks: JSONContent[] = [];
  const flush = () => { if (tasks.length) { blocks.push({ type: 'taskList', content: tasks }); tasks = []; } };
  for (const line of body.split('\n')) {
    if (/^[☐☑]/.test(line)) {
      const text = line.slice(1).trimStart();
      tasks.push(taskItem(paragraph(text ? [{ type: 'text', text }] : undefined), line[0] === '☑'));
    } else {
      flush();
      blocks.push(paragraph(line ? [{ type: 'text', text: line }] : undefined));
    }
  }
  flush();
  return { format: 'qingjian-rich-v1', mode: blocks.some((node) => node.type === 'taskList') ? 'checklist' : 'paragraph',
    document: { type: 'doc', content: blocks }, checks: [] };
}

function inlineText(node: JSONContent): string {
  if (node.type === 'text') return node.text ?? '';
  if (node.type === 'hardBreak') return '\n';
  return (node.content ?? []).map(inlineText).join('');
}

export function documentText(document: JSONContent): string {
  // 导入的普通列表与嵌套段落同样保留行分隔，编辑保存后仍可读、可搜索。
  const blockText = (node: JSONContent): string => {
    if (['bulletList','orderedList','taskList','listItem','taskItem','blockquote','doc'].includes(node.type ?? '')) return (node.content ?? []).map(blockText).join('\n');
    return inlineText(node);
  };
  const lines: string[] = [];
  for (const node of document.content ?? []) {
    if (node.type === 'taskList') {
      for (const item of node.content ?? []) lines.push((item.content ?? []).map(inlineText).join('\n'));
    } else {
      lines.push(blockText(node));
    }
  }
  return lines.join('\n');
}

export function taskEntries(body: string, bodyJson?: string | null): { text: string; checked: boolean; index: number }[] {
  const stored = readDocument(body, bodyJson);
  const entries:{text:string;checked:boolean;index:number}[]=[];
  const walk=(node:JSONContent)=>{if(node.type==='taskItem'){const text=(node.content||[]).filter(n=>n.type!=='taskList').map(inlineText).join('\n').trim();if(text)entries.push({text,checked:node.attrs?.checked===true,index:entries.length});}for(const child of node.content||[])walk(child);};walk(stored.document);
  if(entries.length)return entries;
  // 兼容旧纯文本中已有的行首待办标记；普通段落不会被误当成任务。
  return body.split('\n').flatMap((line, index) => /^[☐☑]/.test(line) ? [{ text: line.slice(1).trim(), checked: line[0] === '☑', index }] : []);
}

function taskItem(block: JSONContent, checked: boolean): JSONContent {
  const content = block.type === 'paragraph' ? [block] : [paragraph(block.content)];
  return { type: 'taskItem', attrs: { checked }, content };
}

function listIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="4" cy="5" r="1.2" fill="currentColor"/><circle cx="4" cy="12" r="1.2" fill="currentColor"/><circle cx="4" cy="19" r="1.2" fill="currentColor"/><path d="M8 5h13M8 12h13M8 19h13" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round"/></svg>;
}

function checkIcon(checked: boolean) {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2" fill={checked ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="1.6"/>{checked && <path d="m7 12 3.2 3.2L17 8" fill="none" stroke="var(--surface)" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"/>}</svg>;
}

export function RichBodyEditor({ body, bodyJson, onChange, mode:filter }: { body: string; bodyJson?: string | null; onChange: (body: string, bodyJson: string) => void; mode:BodyView }) {
  const initial = useRef(readDocument(body, bodyJson)).current;
  const checksRef = useRef(initial.checks);
  const [editorFocused,setEditorFocused]=useState(false);
  const callbackRef = useRef(onChange);
  callbackRef.current = onChange;

  const editor = useEditor({
    // 避免编辑器在列表尾部自动补段落，反复转换时累积空白勾选项。
    extensions: [StarterKit.configure({ trailingNode: false }), TaskIdentity, BodyViewFilter, TaskList, TaskItem.configure({
      a11y: { checkboxLabel: (_node, checked) => checked ? '标记为未完成' : '标记为完成' },
    })],
    content: identifyDocument(initial.document),
    onFocus:()=>setEditorFocused(true),onBlur:()=>setEditorFocused(false),
    onUpdate: ({ editor: current }) => {
      const document = current.getJSON();
      checksRef.current = (document.content ?? []).flatMap((node) => node.type === 'taskList' ? (node.content ?? []).map((item) => (item as JSONContent).attrs?.checked === true) : []);
      const mode: Mode = (document.content ?? []).some((node) => node.type === 'taskList') ? 'checklist' : 'paragraph';
      callbackRef.current(documentText(document), JSON.stringify({ format: 'qingjian-rich-v1', mode, document, checks: checksRef.current } satisfies StoredDocument));
    },
  });

  const [notice,setNotice]=useState('');
  useLayoutEffect(()=>{if(editor)editor.view.dispatch(editor.state.tr.setMeta(bodyViewKey,filter));},[editor,filter]);
  const prepare=()=>setNotice('');
  const convertAll=()=>{if(!editor)return;prepare();const document=editor.getJSON() as JSONContent;let skipped=false;
    const content:JSONContent[]=[];
    for(const block of document.content||[]){if(block.type==='paragraph'){content.push({type:'taskList',content:[taskItem(block,block.attrs?.qjChecked===true)]});}
      else if(block.type==='taskList')content.push(block);
      else if(['bulletList','orderedList'].includes(block.type||'')&&block.content?.every(i=>i.content?.length===1&&i.content[0].type==='paragraph'))content.push({type:'taskList',content:block.content.map(i=>taskItem(i.content![0],i.content![0].attrs?.qjChecked===true))});
      else{content.push(block);skipped=true;}}
    if(JSON.stringify(content)!==JSON.stringify(document.content))editor.commands.setContent({...document,content},{emitUpdate:true});
    if(skipped)setNotice('标题和复杂结构保持原样。');editor.commands.focus();
  };
  const applySelected=(target:'taskList'|'orderedList'|'bulletList',single=false)=>{if(!editor)return;prepare();const {from,to,$head}=editor.state.selection;const ids=new Set<string>();if(single){if($head.parent.type.name==='paragraph')ids.add($head.parent.attrs.qjId);}else{editor.state.doc.nodesBetween(from,to,(n)=>{if(n.type.name==='paragraph')ids.add(n.attrs.qjId);});if(from===to&&$head.parent.type.name==='paragraph')ids.add($head.parent.attrs.qjId);}if(!ids.size){setNotice('请在普通段落中设置格式，复杂结构保持原样。');return;}const next=formatParagraphs(editor.getJSON() as JSONContent,ids,target);if(JSON.stringify(next.document)!==JSON.stringify(editor.getJSON()))editor.commands.setContent(next.document,{emitUpdate:true});if(next.skipped)setNotice('复杂结构保持原样。');editor.chain().focus().setTextSelection(Math.min(from,editor.state.doc.content.size-1)).run();};
  const convertCurrent=()=>applySelected('taskList',true);
  const format=(kind:'ordered'|'bullet'|'bold')=>{if(!editor)return;if(kind==='bold'){prepare();editor.chain().focus().toggleBold().run();}else applySelected(kind==='ordered'?'orderedList':'bulletList');};
  const count=taskEntries(body,bodyJson).filter(t=>t.checked===(filter==='checked')).length;
  if(filter==='reminder')return null;
  return <div className="rich-body">
    {filter!=='all'&&!count&&<p className="body-view-empty">暂无{filter==='checked'?'已勾选':'未勾选'}内容</p>}
    <div className="body-content-card"><div className="body-format-tools" role="toolbar" aria-label="正文格式" onMouseDown={e=>e.preventDefault()}>
      <button className="tool-icon" title="全文添加勾选框" aria-label="全文添加勾选框" disabled={filter!=='all'} onClick={convertAll}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 4h4v4H3zM3 14h4v4H3zM11 6h10M11 16h10"/></svg></button>
      <button className="tool-icon" title="当前段落添加勾选框" aria-label="当前段落添加勾选框" disabled={!editorFocused} onClick={convertCurrent}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 9h5v5H3zM12 11h9"/></svg></button>
      <button className="tool-icon" title="自动序号列表" aria-label="自动序号列表" disabled={!editorFocused} aria-pressed={editor?.isActive('orderedList')} onClick={()=>format('ordered')}><svg viewBox="0 0 24 24" aria-hidden="true"><text x="1" y="9">1</text><text x="1" y="20">2</text><path d="M10 6h11M10 17h11"/></svg></button>
      <button className="tool-icon" title="自动无序列表" aria-label="自动无序列表" disabled={!editorFocused} aria-pressed={editor?.isActive('bulletList')} onClick={()=>format('bullet')}>{listIcon()}</button>
      <button className="tool-icon" title="加粗" aria-label="加粗" disabled={!editorFocused} aria-pressed={editor?.isActive('bold')} onClick={()=>format('bold')}><strong>B</strong></button>
    </div>{notice&&<small role="status">{notice}</small>}
    <EditorContent className="rich-editor" editor={editor} aria-label="正文" /></div>
  </div>;
}
