import {useEffect,useMemo,useRef,useState} from 'react';
import {createPortal} from 'react-dom';
import {Solar} from 'lunar-typescript';
import {invoke} from '@tauri-apps/api/core';
import {TimeSelect} from './time_select';
type Holidays={year:number;version:string;source:string;verifiedAt:string;days:Record<string,{type:'rest'|'work';name:string}>};
const iso=(d:Date)=>`${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;
export function CalendarPicker({value,onChange,reminderDates}:{value:string;onChange:(v:string)=>void;reminderDates:string[]}){
  const [open,setOpen]=useState(false),[cursor,setCursor]=useState(()=>new Date()),[holidays,setHolidays]=useState<Holidays|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false),[source,setSource]=useState('');
  const date=value.slice(0,10),time=value.slice(11,16)||'09:00';const year=cursor.getFullYear(),month=cursor.getMonth();
  // 更新请求可能晚于用户切换年份返回，只允许当前年份更新当前视图。
  const currentYear=useRef(year);currentYear.current=year;
  useEffect(()=>{if(!open)return;let active=true;setHolidays(null);setError('');setSource('');void invoke<Holidays|null>('get_holidays',{year}).then(h=>{if(active)setHolidays(h);}).catch(e=>{if(active)setError(String(e));});return()=>{active=false;};},[year,open]);
  const cells=useMemo(()=>{const first=new Date(year,month,1);const offset=(first.getDay()+6)%7;return Array.from({length:42},(_,i)=>{const d=new Date(year,month,i-offset+1);const lunar=Solar.fromYmd(d.getFullYear(),d.getMonth()+1,d.getDate()).getLunar();const term=lunar.getJieQi(),fest=lunar.getFestivals().join('·');return {d,key:iso(d),label:term||fest||(lunar.getDay()===1?lunar.getMonthInChinese()+'月':lunar.getDayInChinese()),full:lunar.getMonthInChinese()+'月'+lunar.getDayInChinese(),term,fest};});},[year,month]);
  // 按月份整体限制范围，避免十二月/一月切换越过可选年份边界。
  const move=(delta:number)=>setCursor(d=>{const n=Math.max(1901*12,Math.min(2099*12+11,d.getFullYear()*12+d.getMonth()+delta));return new Date(Math.floor(n/12),n%12,1);});
  const choose=(d:string)=>{onChange(`${d}T${time}`);setOpen(false);};
  return <div className="calendar-field"><button type="button" aria-label="选择提醒日期" onClick={()=>{setCursor(date?new Date(date+'T12:00:00'):new Date());setOpen(true);}}>{date||'选择日期'} ▾</button>
    <div className="calendar-time"><TimeSelect label="提醒小时" value={time.slice(0,2)} count={24} onChange={v=>onChange(`${date||iso(new Date())}T${v}:${time.slice(3)}`)}/><span>:</span><TimeSelect label="提醒分钟" value={time.slice(3)} count={60} onChange={v=>onChange(`${date||iso(new Date())}T${time.slice(0,2)}:${v}`)}/></div>
    {open&&createPortal(<div className="calendar-backdrop" onMouseDown={e=>{if(e.target===e.currentTarget)setOpen(false);}}><section className="calendar-dialog" role="dialog" aria-modal="true" aria-label="农历与休班日历" onKeyDown={e=>{if(e.key==='Escape'){e.stopPropagation();setOpen(false);}}}>
      <div className="calendar-heading"><strong>选择提醒日期</strong><button onClick={()=>setOpen(false)} aria-label="关闭日历">×</button></div>
      <div className="calendar-controls"><button aria-label="上个月" onClick={()=>move(-1)}>‹</button><select aria-label="年份" value={year} onChange={e=>setCursor(new Date(+e.target.value,month,1))}>{Array.from({length:199},(_,i)=>1901+i).map(y=><option key={y}>{y}</option>)}</select><select aria-label="月份" value={month} onChange={e=>setCursor(new Date(year,+e.target.value,1))}>{Array.from({length:12},(_,i)=><option value={i} key={i}>{i+1}月</option>)}</select><button aria-label="下个月" onClick={()=>move(1)}>›</button><button onClick={()=>setCursor(new Date())}>今天</button></div>
      <div className="calendar-grid">{['一','二','三','四','五','六','日'].map(d=><span className="calendar-week" key={d}>{d}</span>)}{cells.map(c=>{const h=holidays?.days[c.key];const reminded=reminderDates.includes(c.key);return <button key={c.key} data-date={c.key} aria-label={`${c.key} ${c.full} ${c.term} ${c.fest} ${h?h.name+(h.type==='rest'?'休':'班'):''}${reminded?' 有提醒':''}`} aria-pressed={c.key===date} className={'calendar-day '+(c.d.getMonth()!==month?'outside ':'')+(c.key===date?'selected ':'')+(c.key===iso(new Date())?'today ':'')} onClick={()=>choose(c.key)}><b>{c.d.getDate()}</b><small>{c.label}</small>{h&&<i className={h.type}>{h.type==='rest'?'休':'班'}</i>}{reminded&&<em title="有提醒"/>}</button>;})}</div>
      <div className="calendar-legend">休：官方放假　班：调休上班　●：已有提醒</div>
      {holidays?<p className="calendar-source">{year} 年 · {holidays.version}<br/>核验：{new Date(holidays.verifiedAt).toLocaleDateString()} <button onClick={()=>void invoke('open_news_link',{url:holidays.source})}>官方来源</button></p>:<p className="calendar-source">{year} 年暂无已核验休班数据，不推测放假或调休。农历与节气仍可查看。</p>}
      {error&&<p role="alert" className="error-text">{error}{holidays?'；保留已核验版本。':''}</p>}
      <details><summary>更新官方安排</summary><input aria-label="中国政府网年度通知地址" placeholder="https://www.gov.cn/…" value={source} onChange={e=>setSource(e.target.value)}/><button disabled={busy} onClick={()=>{setBusy(true);setError('');void invoke<Holidays>('refresh_holidays',{year,source:source||null}).then(h=>{if(currentYear.current===h.year)setHolidays(h);}).catch(e=>{if(currentYear.current===year)setError(String(e));}).finally(()=>setBusy(false));}}>{busy?'更新中…':'核验并更新'}</button></details>
      <small>传统节日与法定放假分别标注；日期选择后仍需保存提醒。</small>
    </section></div>,document.body)}
  </div>;
}
