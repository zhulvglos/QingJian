import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
type Settings={alwaysOnTop:boolean;edgeHide:boolean;autoStart:boolean;error:string};
export function WindowSettings(){
  const [settings,setSettings]=useState<Settings|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  useEffect(()=>{void invoke<Settings>('get_shell_settings').then(setSettings).catch(e=>setError(String(e)));},[]);
  const change=async(key:string,value:boolean)=>{setBusy(true);setError('');try{setSettings(await invoke<Settings>('set_shell_setting',{key,value}));}catch(e){setError(String(e));}finally{setBusy(false);}};
  return <div className="window-settings"><h2>窗口与启动</h2>
    {([['alwaysOnTop','轻笺置顶','主窗口和快捷标签一起置顶'],['edgeHide','贴边隐藏','支持左、右、顶部；触碰边缘恢复'],['autoStart','开机启动','登录后打开新闻页']] as const).map(([key,label,detail])=><label className="window-setting-row" key={key}><span><strong>{label}</strong><small>{detail}</small></span><input type="checkbox" aria-label={label} checked={settings?.[key]??false} disabled={!settings||busy} onChange={e=>void change(key,e.target.checked)}/></label>)}
    {(error||settings?.error)&&<p className="error-text" role="alert">{error||settings?.error}</p>}
    <small>透明度尚未接入。</small>
  </div>;
}
