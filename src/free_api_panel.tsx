import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {SplitView} from './split_view';
import {useRunState} from './run_session';
type Entry={id:string;kind:'release'|'offer';title:string;model:string;platform:string;campaign:string;status:string;evidence:string;conditions:{text:string;evidence:string}[];deadline:string|null;deadlineEvidence?:string;sourceUrl:string;issueUrl:string;issueDate:string;checkedAt:string;extractedBy:{model:string}};
type News={initialized:boolean;latestDate:string;updatedAt:string|null;offers:Record<string,Entry>;releases:Entry[];errors:string[];calls:number;checkSummary?:{sourceLatestDate:string|null;lastSuccessfulCheck:string|null;sourceCheckedAt:string|null;coverageComplete:boolean;pendingIssues:number;trackingStart:string|null}};
let pending:Promise<News>|null=null;
type Task={id?:string;status:string;phase?:string;completed?:number;total?:number|null;endedAt?:string;error?:string;result?:{added:number;changed:number;removed:number;completedIssues:number;failedIssues:number}};
const fetchNews=()=>pending??(pending=invoke<News>('refresh_model_news').finally(()=>{pending=null;}));
export function FreeApiPanel(){
 const [data,setData]=useState<News|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[selected,setSelected]=useRunState('model-news.selected',''),[now,setNow]=useState(Date.now());const live=useRef(false);
 const [task,setTask]=useState<Task>({status:'idle'});
 const refresh=async()=>{if(busy||pending)return;setBusy(true);setTask({status:'running',phase:'fetch'});setError('');try{const d=await fetchNews();if(live.current)setData(d);}catch(e){if(live.current)setError(String(e));}finally{if(live.current){setBusy(false);}}};
  useEffect(()=>{
   live.current=true;
   // 页面挂载只读缓存和任务状态。网络调度归后端启动/月度计划负责，切页不触发筛选。
   const read=()=>void invoke<News>('get_model_news').then(d=>{if(live.current)setData(d);}).catch(e=>{if(live.current)setError(String(e));});
   const p=listen<string>('model-news-progress',e=>{if(live.current){read();}});
   const done=listen<Task>('model-news-updated',e=>{if(live.current)setTask(e.payload);read();});
   const poll=()=>void invoke<Task>('model_news_task_status').then(s=>{if(live.current){setTask(s);setBusy(s.status==='running'||pending!==null);if(s.status!=='running')read();}}).catch(()=>{});
   read();poll();const t=setInterval(()=>{setNow(Date.now());poll();},1000);
   return()=>{live.current=false;clearInterval(t);void p.then(f=>f());void done.then(f=>f());};
  },[]);
 // 截止日期在本地定时判断，不依赖刷新成功；期限未知的活动不会因断网消失。
 const offers=Object.values(data?.offers||{}).filter(e=>e.status!=='ended'&&(!e.deadline||Date.parse(e.deadline)>now));const rows=[...offers,...(data?.releases||[])];const active=rows.find(r=>r.id===selected)||rows[0];const failure=error||(!busy&&data?.errors?.[0]);
 // 成功提示依任务真实结束时间显示三秒，切页返回不重新开始计时；部分失败不能显示“已是最新”。
 const failed=!busy&&(!!failure||task.status==='failure');
 const partial=failed&&(task.result?.completedIssues||0)>0;
 const statusText=busy?(task.phase==='screening'?(task.total&&typeof task.completed==='number'?`正在筛选资讯 · 已完成 ${task.completed}/${task.total} 期`:'正在筛选资讯…'):'正在获取最新资讯…'):failed?`${partial?'部分更新未完成':'更新失败'}${rows.length?'，已有内容保留。':'。'}`:task.status==='cancelled'?'检查已取消。':task.status==='success'&&task.endedAt&&now-Date.parse(task.endedAt)<3000?(task.result?.added?`已更新 · 新增 ${task.result.added} 条`:task.result&&(task.result.changed||task.result.removed)?'已更新':'已是最新'):'';
 useEffect(()=>{if(task.status!=='success'||!task.endedAt)return;const timer=window.setTimeout(()=>setNow(Date.now()),Math.max(0,Date.parse(task.endedAt)+3000-Date.now()));return()=>clearTimeout(timer);},[task.id,task.status,task.endedAt]);
 const open=(url:string)=>void invoke('open_news_link',{url}).catch(e=>setError(String(e)));
 return <div className="daily-news model-news-panel"><div className="news-toolbar"><strong>模型资讯</strong><button aria-busy={busy} onClick={()=>void refresh()}>{busy?'检查中…':failed?'重试':'检查更新'}</button></div>
 {statusText&&<p className={failed?'news-error':'news-meta'} role={failed?'alert':'status'}>{statusText}</p>}
 {failed&&<details className="news-error"><summary>查看原因</summary>{error||task.error||data?.errors.join('；')||'本次检查未完成，请重试。'}</details>}
 {active?<SplitView storageKey="model-news" list={<div role="listbox" aria-label="模型资讯标题列表" tabIndex={0} onKeyDown={e=>{if(!['ArrowDown','ArrowUp'].includes(e.key))return;e.preventDefault();const i=rows.findIndex(r=>r.id===active.id),n=Math.max(0,Math.min(rows.length-1,i+(e.key==='ArrowDown'?1:-1)));setSelected(rows[n].id);const b=e.currentTarget.querySelectorAll<HTMLButtonElement>('.news-title-card')[n];b?.focus({preventScroll:true});b?.scrollIntoView({block:'nearest'});}}>{rows.map(r=><button key={r.id} role="option" aria-selected={r.id===active.id} tabIndex={r.id===active.id?0:-1} className={'news-title-card '+(r.id===active.id?'selected':'')} onClick={()=>setSelected(r.id)}>{r.kind==='offer'&&<small>限时免费 · {r.deadline?'有截止日期':'期限待确认'}</small>}{r.title}<small>{r.platform} · {r.issueDate}</small></button>)}</div>} detail={<article className="news-card"><h2>{active.title}</h2><p>{active.model} · {active.platform}</p>{active.kind==='offer'&&<><strong>限时免费</strong><p>{active.deadline?`截止：${new Date(active.deadline).toLocaleString('zh-CN')}`:'期限待确认；来源未公布明确截止时间。'}</p><h3>免费条件</h3>{active.conditions.length?active.conditions.map((c,i)=><p key={i}>{c.text}</p>):<p>来源未明确领取条件，请核对原文。</p>}</>}<p className="api-story-body">{active.evidence}</p><footer><button onClick={()=>open(active.issueUrl)}>橘鸦原文</button>{active.sourceUrl!==active.issueUrl&&<button onClick={()=>open(active.sourceUrl)}>报道中的来源</button>}</footer><details><summary>来源依据</summary>{active.conditions.map((c,i)=><p key={i}>{c.evidence}</p>)}{active.deadlineEvidence&&<p>{active.deadlineEvidence}</p>}<small>报道 {active.issueDate} · 提取模型 {active.extractedBy?.model}<br/>仅覆盖公开网站与 RSS，未确认公众号全量同步。</small></details></article>}/>:<div className="free-empty"><strong>{failure?'模型资讯暂不可用':'暂无可展示的模型资讯'}</strong></div>}
 </div>;
}
