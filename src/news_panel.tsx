import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { SplitView } from './split_view';
import {useRunState} from './run_session';
type Article={id:string;category:string;title:string;summary:string;source:string;sourceUrl:string|null;permalink:string|null;publishedAt:string|null;timelineAt?:string|null};
type NewsFeed={fetchedAt:string;source:string;window:string;date:string|null;items:Article[]};
// 页面切换与 React 重挂载共享一次请求，避免重复抓取和过期结果覆盖。
let pending:Promise<NewsFeed>|null=null;
// 手动刷新反馈随切页保留；日报日期与文章发布时间分别展示，不能互相替代。
let refreshFeedback='';
function fetchDaily(){
  if(!pending)pending=(async()=>{
    const before=await invoke<NewsFeed|null>('get_news_cache');
    const after=await invoke<NewsFeed>('refresh_news');
    refreshFeedback=before?.fetchedAt===after.fetchedAt?'暂无新内容，当前已是来源提供的最新精选。':'新闻已更新。';
    return after;
  })().finally(()=>{pending=null;});
  return pending;
}
export function NewsPanel(){
  const [daily,setDaily]=useState<NewsFeed|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const [cacheError,setCacheError]=useState('');
  const [feedback,setFeedback]=useState(refreshFeedback);
  const [selected,setSelected]=useRunState('news.selected','');
  const active=daily?.items.find(item=>item.id===selected)||daily?.items[0];
  useEffect(()=>{if(daily)setSelected(previous=>daily.items.some(item=>item.id===previous)?previous:daily.items[0]?.id||'');},[daily]);
  const running=useRef(false),mounted=useRef(true);
  const refresh=async()=>{
    if(running.current)return;running.current=true;setBusy(true);refreshFeedback='';setFeedback('');
    try {const data=await fetchDaily();if(mounted.current){setDaily(data);setError('');setCacheError('');setFeedback(refreshFeedback);}}
    catch(e){refreshFeedback='';if(mounted.current){setError(String(e));setFeedback('');}}
    finally{running.current=false;if(mounted.current)setBusy(false);}
  };
  useEffect(()=>{
    mounted.current=true;
    // 页面只读取缓存与更新状态，自动获取归后端启动负责；切页和窗口恢复不发网络请求。
    const read=async()=>{try{const [cache,state]=await Promise.all([invoke<NewsFeed|null>('get_news_cache'),invoke<{running:boolean;error:string}>('get_news_update_status')]);if(mounted.current){setDaily(cache);setBusy(state.running||pending!==null);setError(state.error);setFeedback(state.error?'':refreshFeedback);setCacheError('');}}catch(e){if(mounted.current)setCacheError(String(e));}};
    const event=listen('ai-news-updated',()=>void read());void read();
    const timer=window.setInterval(()=>void read(),1000);
    return()=>{mounted.current=false;window.clearInterval(timer);void event.then(f=>f());};
  },[]);
  return <div className="daily-news">
    <div className="news-toolbar"><strong>AI 新闻</strong><button onClick={()=>void refresh()} disabled={busy} aria-busy={busy}>{busy?'刷新中…':'刷新'}</button></div>
    {cacheError&&<p className="news-error" role="alert">{cacheError}</p>}
    {error&&<p className="news-error" role="alert">{error}</p>}
    {!error&&feedback&&<p className="news-meta" role="status">{feedback}</p>}
    {daily&&active?<><div className="news-meta">{daily.source==='AIHOT 精选'?'AIHOT 精选':'AIHOT 旧缓存'} · {daily.date||'来源未提供日期'}</div>
      {daily.source!=='AIHOT 精选'&&<p className="news-meta">点击刷新，获取最新精选。</p>}
      <SplitView storageKey="news" list={<div role="listbox" aria-label="新闻标题列表" tabIndex={0} onKeyDown={e=>{
        if(!['ArrowUp','ArrowDown'].includes(e.key))return;e.preventDefault();
        // 仅列表内处理方向键，选中后同步焦点与滚动，不影响其他输入控件。
        const index=daily.items.findIndex(i=>i.id===active.id);const next=Math.max(0,Math.min(daily.items.length-1,index+(e.key==='ArrowDown'?1:-1)));
        setSelected(daily.items[next].id);const card=e.currentTarget.querySelectorAll<HTMLButtonElement>('.news-title-card')[next];card?.focus({preventScroll:true});card?.scrollIntoView({block:'nearest'});
      }}>{daily.items.map(item=><button role="option" tabIndex={active.id===item.id?0:-1} className={'news-title-card '+(active.id===item.id?'selected':'')} key={item.id} aria-selected={active.id===item.id} onClick={()=>setSelected(item.id)}>{item.title}{(item.timelineAt||item.publishedAt)&&<small>{new Date(item.timelineAt||item.publishedAt!).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai'})}</small>}</button>)}</div>} detail={<article className="news-card"><small>{active.category}{active.publishedAt&&<> · 原文发布时间 {new Date(active.publishedAt).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai'})}</>}</small><h2>{active.title}</h2><p>{active.summary||'来源未提供摘要'}</p><footer><span>{active.source||'来源未提供'}</span>{active.permalink&&<button onClick={()=>void invoke('open_news_link',{url:active.permalink}).catch(e=>setError(String(e)))}>站内阅读</button>}{active.sourceUrl&&<button onClick={()=>void invoke('open_news_link',{url:active.sourceUrl}).catch(e=>setError(String(e)))}>原文</button>}</footer></article>}/></>:<p className="news-meta">暂无新闻内容。可点击刷新。</p>}
  </div>;
}
