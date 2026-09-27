import {useEffect,useMemo,useRef,useState} from 'react';
import type {Item} from './item_workspace';

export function QuickSearch({items,onOpen}:{items:Item[];onOpen:(item:Item)=>void}){
 const [open,setOpen]=useState(false),[input,setInput]=useState(''),[query,setQuery]=useState(''),[composing,setComposing]=useState(false),[active,setActive]=useState(0);
 const root=useRef<HTMLDivElement>(null),field=useRef<HTMLInputElement>(null),list=useRef<HTMLDivElement>(null);
 const needle=query.trim().toLowerCase();
 // 纯本地同步筛选，没有异步搜索响应覆盖后一次输入的问题；保留每条稳定 ID。
 const results=useMemo(()=>needle?items.filter(i=>i.title.toLowerCase().includes(needle)).sort((a,b)=>{
  const rank=(s:string)=>s===needle?0:s.startsWith(needle)?1:2;
  return rank(a.title.toLowerCase())-rank(b.title.toLowerCase())||b.updatedAt.localeCompare(a.updatedAt)||a.id.localeCompare(b.id);
 }):[],[items,needle]);
 const selected=Math.min(active,Math.max(0,results.length-1));
 useEffect(()=>{if(open)field.current?.focus();},[open]);
 useEffect(()=>{list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({block:'nearest'});},[selected,needle]);
 useEffect(()=>{const outside=(e:MouseEvent)=>{if(root.current&&!root.current.contains(e.target as Node))setOpen(false);};document.addEventListener('mousedown',outside);return()=>document.removeEventListener('mousedown',outside);},[]);
 useEffect(()=>{const key=(e:KeyboardEvent)=>{if(e.ctrlKey&&!e.altKey&&e.key.toLowerCase()==='f'){e.preventDefault();setOpen(true);field.current?.focus();}};document.addEventListener('keydown',key);return()=>document.removeEventListener('keydown',key);},[]);
 const choose=(item:Item)=>{setOpen(false);onOpen(item);};
 const highlight=(title:string)=>{const out=[];const lower=title.toLowerCase();let start=0,index=lower.indexOf(needle);while(index>=0){out.push(title.slice(start,index),<mark key={index}>{title.slice(index,index+needle.length)}</mark>);start=index+needle.length;index=lower.indexOf(needle,start);}out.push(title.slice(start));return out;};
 return <div ref={root} className="quick-search">
  <button className="icon-button" title="搜索标题" aria-label="搜索标题" aria-expanded={open} onClick={()=>setOpen(v=>!v)}><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/></svg></button>
  {open&&<div className="quick-search-popover"><input ref={field} type="search" role="combobox" aria-label="搜索便签和笔记标题" aria-controls="quick-search-results" aria-expanded={!!needle&&!composing} aria-autocomplete="list" aria-activedescendant={needle&&!composing&&results[selected]?'search-'+results[selected].id:undefined} placeholder="搜索便签、笔记标题" value={input}
   onCompositionStart={()=>setComposing(true)} onCompositionEnd={e=>{setComposing(false);setQuery(e.currentTarget.value);setActive(0);}}
   onChange={e=>{setInput(e.target.value);if(!composing){setQuery(e.target.value);setActive(0);}}}
   onKeyDown={e=>{if(composing||e.nativeEvent.isComposing||e.keyCode===229)return;if(e.key==='Escape'){e.preventDefault();setOpen(false);root.current?.querySelector('button')?.focus();}else if(['ArrowDown','ArrowUp'].includes(e.key)){e.preventDefault();setActive(Math.max(0,Math.min(results.length-1,selected+(e.key==='ArrowDown'?1:-1))));}else if(e.key==='Enter'&&results[selected]){e.preventDefault();choose(results[selected]);}}}/>
   {!!needle&&!composing&&<div id="quick-search-results" ref={list} className="quick-search-results" role="listbox" aria-label="标题搜索结果">{results.length?results.map((item,index)=><button type="button" id={'search-'+item.id} key={item.id} role="option" aria-selected={index===selected} onMouseDown={e=>e.preventDefault()} onClick={()=>choose(item)}><span>{highlight(item.title)}</span><small>{item.kind==='sticky'?'便签':'笔记'}</small></button>):<p role="status">未找到</p>}</div>}
  </div>}
 </div>;
}
