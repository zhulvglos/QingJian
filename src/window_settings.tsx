import { useEffect, useRef,useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
type Settings={transparency:number;alwaysOnTop:boolean;edgeHide:boolean;autoStart:boolean;error:string};
export function WindowSettings(){
  const [settings,setSettings]=useState<Settings|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  useEffect(()=>{void invoke<Settings>('get_shell_settings').then(setSettings).catch(e=>setError(String(e)));},[]);
  const change=async(key:string,value:boolean)=>{setBusy(true);setError('');try{setSettings(await invoke<Settings>('set_shell_setting',{key,value}));}catch(e){setError(String(e));}finally{setBusy(false);}};
  const serial=useRef(Promise.resolve());
  const transparency=(value:number)=>{setSettings(s=>s?{...s,transparency:value}:s);serial.current=serial.current.then(async()=>{try{await invoke('set_background_transparency',{value});setError('');}catch(e){setError(String(e));}});};
  return <div className="window-settings"><h2>窗口与启动</h2>
    {([['edgeHide','贴边隐藏','窗口实际贴住左、右或上边缘，松手 0.5 秒隐藏；移到原边缘位置唤出，离开窗口 0.5 秒后再次隐藏；最大化时不隐藏'],['autoStart','开机启动','登录后打开新闻页']] as const).map(([key,label,detail])=><label className="window-setting-row" key={key}><span><strong>{label}</strong><small>{detail}</small></span><input type="checkbox" aria-label={label} checked={settings?.[key]??false} disabled={!settings||busy} onChange={e=>void change(key,e.target.checked)}/></label>)}
    {(error||settings?.error)&&<p className="error-text" role="alert">{error||settings?.error}</p>}
    <div className="transparency-setting"><div className="transparency-heading"><label htmlFor="window-transparency">整窗透明度 · {settings?.transparency??0}%</label><button className="text-action" onClick={()=>transparency(0)} disabled={!settings||settings.transparency===0}>恢复默认</button></div><input id="window-transparency" aria-label="整窗透明度" type="range" min={0} max={70} step={5} value={settings?.transparency??0} disabled={!settings} onChange={e=>transparency(Number(e.target.value))}/><small className="transparency-help">主窗口和快捷标签统一透明；文字、图标同步变化，不改变点击。0% 为不透明。</small></div>
  </div>;
}
