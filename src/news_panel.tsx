import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SplitView } from './split_view';
type Article={category:string;title:string;summary:string;source:string;sourceUrl:string|null;permalink:string|null};
type Daily={date:string;fetchedAt:string;source:string;items:Article[]};
// 页面切换与 React 重挂载共享一次请求，避免重复抓取和过期结果覆盖。
let pending:Promise<Daily>|null=null;
function fetchDaily(){
  if(!pending)pending=invoke<Daily>('refresh_news').finally(()=>{pending=null;});
  return pending;
}
export function NewsPanel(){
  const [daily,setDaily]=useState<Daily|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const [cacheError,setCacheError]=useState('');
  const [selected,setSelected]=useState('');
  const key=(item:Article)=>`${item.sourceUrl||item.permalink||item.source}:${item.title}`;
  const active=daily?.items.find(item=>key(item)===selected)||daily?.items[0];
  useEffect(()=>{if(daily)setSelected(previous=>daily.items.some(item=>key(item)===previous)?previous:daily.items[0]?key(daily.items[0]):'');},[daily]);
  const running=useRef(false),mounted=useRef(true);
  const refresh=async()=>{
    if(running.current)return;running.current=true;setBusy(true);
    try {const data=await fetchDaily();if(mounted.current){setDaily(data);setError('');setCacheError('');}}
    catch(e){if(mounted.current)setError('获取失败：'+String(e));}
    finally{running.current=false;if(mounted.current)setBusy(false);}
  };
  useEffect(()=>{mounted.current=true;void (async()=>{try{const cache=await invoke<Daily|null>('get_news_cache');if(mounted.current)setDaily(cache);}catch(e){if(mounted.current)setCacheError(String(e));}if(mounted.current)void refresh();})();return()=>{mounted.current=false;};},[]);
  // 按北京时间自然日标注是否为今日；不把 08:00 业务切换规则套到接口的返回日期上。
  const today=new Intl.DateTimeFormat('sv-SE',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit'}).format(new Date());
  return <div className="daily-news">
    <div className="news-toolbar"><strong>AI 新闻</strong><button onClick={()=>void refresh()} disabled={busy}>{busy?'获取中…':'刷新'}</button></div>
    {cacheError&&<p className="news-error" role="alert">{cacheError}</p>}
    {error&&<p className="news-error" role="alert">{error}{daily?(daily.date===today?'；更新失败，显示今日上次获取内容。':'；保留上次成功内容。'):''}</p>}
    {daily&&active?<><div className="news-meta">{daily.date!==today&&<strong className="stale-news">非今日内容 · </strong>}数据：{daily.date} · {daily.source||'来源未提供'}<br/>获取：{new Date(daily.fetchedAt).toLocaleString('zh-CN')}</div>
      <SplitView storageKey="news" list={<div role="listbox" aria-label="新闻标题列表" tabIndex={0} onKeyDown={e=>{
        if(!['ArrowUp','ArrowDown'].includes(e.key))return;e.preventDefault();
        // 仅列表内处理方向键，选中后同步焦点与滚动，不影响其他输入控件。
        const index=daily.items.findIndex(i=>key(i)===key(active));const next=Math.max(0,Math.min(daily.items.length-1,index+(e.key==='ArrowDown'?1:-1)));
        setSelected(key(daily.items[next]));const card=e.currentTarget.querySelectorAll<HTMLButtonElement>('.news-title-card')[next];card?.focus({preventScroll:true});card?.scrollIntoView({block:'nearest'});
      }}>{daily.items.map((item,index)=><button role="option" tabIndex={key(active)===key(item)?0:-1} className={'news-title-card '+(key(active)===key(item)?'selected':'')} key={key(item)+index} aria-selected={key(active)===key(item)} onClick={()=>setSelected(key(item))}>{item.title}</button>)}</div>} detail={<article className="news-card"><small>{active.category} · 摘要</small><h2>{active.title}</h2><p>{active.summary||'来源未提供摘要'}</p><footer><span>{active.source||'来源未提供'}</span>{(active.sourceUrl||active.permalink)&&<button onClick={()=>void invoke('open_news_link',{url:active.sourceUrl||active.permalink}).catch(e=>setError(String(e)))}>原文</button>}</footer><small>接口未提供全文与单条发布时间。</small></article>}/></>:<p className="news-meta">{busy?'正在获取真实新闻…':'暂无可用新闻缓存。请刷新重试。'}</p>}
  </div>;
}
