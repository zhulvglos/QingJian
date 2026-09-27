import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
type Config={root:string;engine?:'sensevoice';sensePython?:string;senseRuntime?:string;senseModels?:string;available:boolean;processing?:boolean;installation?:{state?:string;error?:string};validation?:{transcribed?:boolean;integrity?:boolean}};
export function VoiceSettings(){return <section className="model-settings"><h1>语音</h1><LocalVoiceSettings/><OnlineEditor kind="audio"/></section>;}
function LocalVoiceSettings(){
 const [config,setConfig]=useState<Config>({root:'',available:false}),[busy,setBusy]=useState(false),[dirty,setDirty]=useState(false),[error,setError]=useState(''),[progress,setProgress]=useState('');
 useEffect(()=>{
  let active=true;
  let polling=false;const load=()=>invoke<Config>('audio_config').then(c=>{if(active){setConfig(c);polling=!!c.processing;}});
  void load().catch(e=>{if(active)setError(String(e));});
  const timer=setInterval(()=>{if(active&&polling)void load();},2000);
  const stage=listen<string>('local-model-stage',e=>{if(active){setProgress(e.payload);polling=true;}});
  return()=>{active=false;clearInterval(timer);void stage.then(f=>f());};
 },[]);
 const run=async(fn:()=>Promise<void>)=>{setBusy(true);setError('');setProgress('处理中…');try{await fn();}catch(e){setError(String(e));setProgress('');const c=await invoke<Config>('audio_config').catch(()=>null);if(c)setConfig(c);}finally{setBusy(false);}};
 const change=(key:keyof Config,value:string)=>{setConfig(c=>({...c,[key]:value,available:false,validation:undefined}));setDirty(true);setError('');setProgress('');};
 const fields:('root'|'sensePython'|'senseRuntime'|'senseModels')[]=['root','senseModels','senseRuntime','sensePython'];
 const names={root:'语音工作目录（非 C 盘）',senseModels:'模型目录',senseRuntime:'已有运行环境',sensePython:'兼容运行程序'};
 return <section className="local-voice-settings"><h2>本地转写 · SenseVoice</h2><fieldset disabled={busy||config.processing}>
  <details className="local-paths"><summary>模型与保存位置</summary>{fields.map(k=><label key={k}>{names[k]}<input value={config[k]||''} onChange={e=>change(k,e.target.value)}/><button onClick={()=>void run(async()=>{const path=await invoke<string|null>('choose_audio_path',{kind:k});if(path)change(k,path);else setProgress('');})}>选择</button></label>)}</details>
  <div className="file-actions"><button onClick={()=>void run(async()=>{let c=config;if(!c.root){const path=await invoke<string|null>('choose_audio_path',{kind:'root'});if(!path){setProgress('');return;}c={...c,root:path};}await invoke('save_audio_config',{config:c});setProgress('安装中…');setConfig(await invoke<Config>('install_sensevoice'));setDirty(false);setProgress('');})}>一键检测并安装</button><button onClick={()=>void run(async()=>{setConfig(await invoke<Config>('save_audio_config',{config}));setDirty(false);setProgress('配置已保存');})}>保存</button><button disabled={dirty||!config.available} onClick={()=>void run(async()=>{setProgress('检查并转写样例…');await invoke<string>('check_local_model');setConfig(await invoke<Config>('audio_config'));setProgress('');})}>检查可用性</button></div>
 </fieldset>
 <p role="status">{busy||config.processing?progress||(config.installation?.state==='installing'?'安装中…':'检查中…'):dirty?'配置已修改，请先保存':config.validation?.transcribed?'可用 · 实际转写通过':error||config.installation?.state==='repair'?'需修复':config.available?'需检查':'未安装'}</p>
 {!busy&&progress&&<p>{progress}</p>}{error&&<><p role="alert" className="error-text">检查或配置失败</p><details className="model-error"><summary>查看原因</summary><p>{error}</p></details></>}
 </section>;
}

type OnlineConfig={endpoint?:string;model?:string;hasKey?:boolean;tested?:boolean;testedAt?:string;revision?:string;audioProtocol?:string;testPreview?:string};
export function OnlineSettings(){return <section className="model-settings"><h1>文字模型</h1><p className="news-meta">用于转写纪要与模型资讯筛选。</p><OnlineEditor kind="text"/></section>;}

function conciseError(error:string):string {
 const code=error.match(/HTTP\s*(\d{3})/)?.[1];
 if(code) return ({'401':'密钥无效','403':'没有访问权限','404':'地址或模型不支持此功能','429':'请求限流或额度不足'} as Record<string,string>)[code] || `服务返回 HTTP ${code}`;
 if(error.includes('连接失败或超时')) return '连接失败或超时';
 if(/超时|timeout/i.test(error)) return '连接超时';
 if(/连接|connect/i.test(error)) return '无法连接服务';
 return error.split(/[\n；;]/)[0].slice(0,64);
}
function OnlineEditor({kind}:{kind:'audio'|'text'}){
 const [config,setConfig]=useState<OnlineConfig>({}),[secret,setSecret]=useState(''),[dirty,setDirty]=useState(false),[busy,setBusy]=useState(false),[testing,setTesting]=useState(false),[operation,setOperation]=useState('测试'),[error,setError]=useState('');
 useEffect(()=>{let active=true;void invoke<OnlineConfig>('online_config',{kind}).then(c=>{if(active)setConfig(c);}).catch(e=>{if(active)setError(String(e));});return()=>{active=false;};},[kind]);
 const run=async(fn:()=>Promise<void>)=>{setBusy(true);setError('');try{await fn();}catch(e){setError(String(e));}finally{setBusy(false);setTesting(false);}};
 // 任意字段变更立即撤销旧测试的展示，凭据仍只交由原有系统存储处理。
 const change=()=>{setDirty(true);setError('');setConfig(c=>({...c,tested:false,testPreview:undefined}));};
 return <fieldset disabled={busy}><legend>{kind==='audio'?'在线转写':'在线文本模型'}</legend>
  <label>服务地址（Base URL）<input placeholder="https://服务地址/v1" value={config.endpoint||''} onChange={e=>{change();setConfig(c=>({...c,endpoint:e.target.value}));}}/></label>
  <label>模型 ID<input value={config.model||''} onChange={e=>{change();setConfig(c=>({...c,model:e.target.value}));}}/></label>
  <label>API Key<input type="password" autoComplete="off" placeholder={config.hasKey?'已保存，留空保持':'尚未配置'} value={secret} onChange={e=>{change();setSecret(e.target.value);}}/></label>
  <div className="file-actions">
   <button onClick={()=>{setOperation('保存');void run(async()=>{setConfig(await invoke<OnlineConfig>('save_online_config',{kind,endpoint:config.endpoint||'',model:config.model||'',apiKey:secret||null,clearKey:false}));setSecret('');setDirty(false);});}}>保存</button>
   <button disabled={!config.hasKey} onClick={()=>{setOperation('移除密钥');void run(async()=>{setConfig(await invoke<OnlineConfig>('save_online_config',{kind,endpoint:config.endpoint||'',model:config.model||'',apiKey:null,clearKey:true}));setSecret('');setDirty(false);});}}>移除密钥</button>
   <button disabled={dirty||!config.hasKey} onClick={()=>{setOperation('测试');setTesting(true);setConfig(c=>({...c,tested:false,testPreview:undefined}));void run(async()=>{setConfig(await invoke<OnlineConfig>('test_online_model',{kind,confirmed:true}));});}}>测试</button>
  </div>
  <p role="status" className={error?'error-text':''}>{testing?'测试中…':error?operation+'失败：'+conciseError(error):dirty?'配置已修改，请先保存':config.tested?'连接成功':'尚未测试'}</p>
  {config.testPreview&&!dirty&&config.tested&&<p className="model-test-preview">{config.testPreview}</p>}
  {error&&<details className="model-error"><summary>查看详情</summary><p>{error}</p></details>}
 </fieldset>;
}
