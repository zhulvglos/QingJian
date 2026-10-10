import React,{useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';

export type CloudStatus={configured:boolean;url:string;publicKey:string;email:string|null;phase:string;error:string;restartRequired:boolean;sync?:{pending:number;linked:boolean;lastSuccess:string|null;conflicts:{copyId:string;originalId:string}[]}};
export function syncLabel(s:CloudStatus|null){
 if(!s?.email)return '仅保存在本机';
 if(!s.sync?.linked)return '等待首次关联确认';
 if(s.phase==='正在同步')return s.phase;
 if(s.error)return '同步失败，可重试';
 if(s.sync?.conflicts.length)return '发现冲突，已保留两个版本';
 if(s.sync?.pending)return '已保存到本机，等待同步';
 return s.sync?.lastSuccess?'已同步':'等待首次同步';
}
export function CloudSettings({status,onRefresh,canChange,onOpen}:{status:CloudStatus|null;onRefresh:()=>Promise<void>;canChange:()=>string;onOpen:(id:string)=>void}){
 const [email,setEmail]=useState(status?.email||''),[code,setCode]=useState(''),[url,setUrl]=useState(status?.url||''),[key,setKey]=useState(status?.publicKey||'');
 const [busy,setBusy]=useState(''),[error,setError]=useState(''),[message,setMessage]=useState(''),[remaining,setRemaining]=useState(0),[confirmLink,setConfirmLink]=useState(false),[confirmLogout,setConfirmLogout]=useState(false);
 useEffect(()=>{if(status){setUrl(status.url);setKey(status.publicKey);if(status.email)setEmail(status.email);}},[status?.url,status?.publicKey,status?.email]);
 useEffect(()=>{if(!remaining)return;const timer=setTimeout(()=>setRemaining(n=>Math.max(0,n-1)),1000);return()=>clearTimeout(timer);},[remaining]);
 const run=async(name:string,fn:()=>Promise<void>)=>{if(busy)return;setBusy(name);if(['verify','logout','restart'].includes(name))window.dispatchEvent(new CustomEvent('cloud-transition-busy',{detail:true}));setError('');setMessage('');try{await fn();}catch(e){setError(String(e));}finally{await onRefresh().catch(()=>{});setBusy('');window.dispatchEvent(new CustomEvent('cloud-transition-busy',{detail:false}));}};
 const guard=()=>{const reason=canChange();if(reason)throw new Error(reason);};
 const validEmail=/^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/.test(email.trim())&&email.trim().length<=254;
 return <div className="cloud-settings">
  <h2>账号与同步</h2><p>便签与笔记先保存在本机，再后台同步。无需登录也能使用。</p>
  {status?.restartRequired?<section role="alert"><strong>账号已变更，需要重启轻笺</strong><p>重启后进入独立工作区，旧账号的同步已停止。</p><button disabled={!!busy} onClick={()=>void run('restart',async()=>{guard();await invoke('cloud_restart');})}>重启轻笺</button></section>:<>
   {!status?.email&&<details open={!status?.configured}><summary>云端连接配置</summary><p>仅填写项目公开地址与公开 Key。发信授权码和管理密钥只能保存在云端控制台。</p><label>项目地址<input value={url} onChange={e=>setUrl(e.target.value)} placeholder="https://项目标识.supabase.co" autoComplete="off"/></label><label>Publishable / anon key<input value={key} onChange={e=>setKey(e.target.value)} autoComplete="off"/></label><button disabled={!!busy} onClick={()=>void run('config',async()=>{await invoke('cloud_configure',{url,publicKey:key});setMessage('公开连接配置已保存。尚未验证发信与云端数据库。');})}>保存连接配置</button></details>}
   {!status?.configured&&<p role="status">尚未配置云端，登录和同步不可用，本地保存正常。</p>}
   {status?.email&&<section><strong>{status.email}</strong><p role="status">{syncLabel(status)}</p><p>最近成功同步：{status.sync?.lastSuccess?new Date(status.sync.lastSuccess).toLocaleString('zh-CN'):'尚无记录'}</p><p>待同步 {status.sync?.pending||0} 条</p><button disabled={!!busy||!status.sync?.linked} onClick={()=>void run('sync',async()=>{await invoke('cloud_sync');})}>{busy==='sync'?'正在同步…':'立即同步 / 重试'}</button><button disabled={!!busy} onClick={()=>{setError('');const reason=canChange();if(reason){setError(reason);return;}setConfirmLogout(true);}}>退出登录…</button></section>}
   {(!status?.email||status.error.includes('会话')||status.error.includes('登录'))&&<section><h3>{status?.email?'重新验证邮箱':'邮箱验证码登录'}</h3><label>邮箱<input type="email" value={email} readOnly={!!status?.email} onChange={e=>{setEmail(e.target.value);setCode('');}} placeholder="123456@qq.com" autoComplete="email"/></label><button disabled={!!busy||remaining>0||!validEmail||!status?.configured} onClick={()=>void run('send',async()=>{await invoke('cloud_send_code',{email:email.trim()});setRemaining(60);setMessage('验证码请求已提交，请检查收件箱及垃圾邮件；请求成功不代表邮件已送达。');})}>{busy==='send'?'发送中…':remaining?`${remaining} 秒后可重发`:'获取 / 重新发送验证码'}</button><label>验证码<input value={code} onChange={e=>setCode(e.target.value.replace(/\D/g,'').slice(0,10))} inputMode="numeric" autoComplete="one-time-code"/></label><p>由认证服务生成并校验。验证码错误、过期或服务端限流会显示原因。</p><button disabled={!!busy||!validEmail||code.length<6||!status?.configured} onClick={()=>void run('verify',async()=>{guard();const restart=await invoke<boolean>('cloud_verify',{email:email.trim(),code});setCode('');setMessage(restart?'邮箱验证通过，请重启进入账号工作区。':'邮箱重新验证成功，可重试同步。');})}>{busy==='verify'?'验证中…':status?.email?'验证并恢复会话':'验证登录，进入账号工作区'}</button><p>首次登录需重启。请先保存草稿、结束录音或导入任务。</p></section>}
   {status?.email&&!status.sync?.linked&&<section><h3>首次关联本机内容</h3><p>是否把未登录工作区已保存的便签和笔记复制到 {status.email}？包括回收站内容及置顶排序。确认前不上传任何内容。</p><p>选择加入后，先创建并校验本地完整 SQLite 快照，再加入账号队列。原本地工作区仍保留独立副本；后续修改分别归属各工作区。</p><p>提醒、快捷标签、录音、模型密钥和设备设置不随内容加入或同步。新电脑需重新设置提醒。</p><button disabled={!!busy} onClick={()=>setConfirmLink(true)}>备份并加入此账号…</button><button disabled={!!busy} onClick={()=>void run('link',async()=>{await invoke('cloud_link_local',{include:false});setMessage('本机原有内容继续留在未登录工作区。可以同步此账号的云端内容。');})}>仅使用账号内容</button></section>}
   {!!status?.sync?.conflicts.length&&<section><h3>发现冲突，已保留两个版本</h3><p>原条目保留云端版本，冲突副本保留本机版本。可分别打开比较、复制合并，保存后再确认已处理；不自动删除任何版本。</p>{status.sync.conflicts.map(c=><div key={c.copyId}><button onClick={()=>onOpen(c.originalId)}>打开云端版本</button><button onClick={()=>onOpen(c.copyId)}>打开本机冲突副本</button><button disabled={!!busy} onClick={()=>void run('resolve',async()=>{await invoke('cloud_resolve_conflict',{copyId:c.copyId});})}>已检查并处理</button></div>)}</section>}
  </>}
  {(error||status?.error)&&<p className="error-text" role="alert">{error||status?.error}</p>}{message&&<p role="status">{message}</p>}
  {confirmLink&&<div className="dialog-backdrop"><section className="leave-dialog" role="dialog" aria-label="确认首次上传"><h2>加入此邮箱账号？</h2><p>确认后，本机未登录工作区已保存内容将加入该账号并自动上传至所配置的 Supabase 项目。备份失败则停止。</p><div className="dialog-actions"><button disabled={!!busy} onClick={()=>void run('link',async()=>{const result=await invoke<{backup:string;imported:number}>('cloud_link_local',{include:true});setConfirmLink(false);setMessage(`已关联 ${result.imported} 条。备份：${result.backup||'本机无已有数据库'}。等待后台同步。`);})}>确认备份并加入</button><button disabled={!!busy} onClick={()=>setConfirmLink(false)}>取消</button></div></section></div>}
  {confirmLogout&&<div className="dialog-backdrop"><section className="leave-dialog" role="dialog" aria-label="确认退出账号"><h2>退出此邮箱账号？</h2><p>待同步内容保留在此账号的本机缓存，下次登录继续上传。退出后需重启进入未登录工作区。云端内容不会被删除。</p><div className="dialog-actions"><button disabled={!!busy} onClick={()=>void run('logout',async()=>{guard();await invoke('cloud_logout');setConfirmLogout(false);setMessage('已停止同步并清除本机会话，请重启。');})}>退出并停止同步</button><button disabled={!!busy} onClick={()=>setConfirmLogout(false)}>取消</button></div></section></div>}
 </div>;
}
