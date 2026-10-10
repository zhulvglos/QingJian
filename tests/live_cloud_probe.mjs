// 真实认证验收仅连接指定的隔离客户端；不读取验证码、令牌或真实笔记。
const mode=process.argv[2]||'status';
const port=process.argv[3]||'9243';
const wait=ms=>new Promise(r=>setTimeout(r,ms));
let target;
for(let i=0;i<80;i++){
 try{target=(await(await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t=>t.url==='http://tauri.localhost/');if(target)break;}catch{}
 await wait(150);
}
if(!target)throw Error('隔离客户端未启动');
const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
let serial=0;const pending=new Map();
ws.onmessage=e=>{const m=JSON.parse(e.data);const p=pending.get(m.id);if(p){pending.delete(m.id);m.error?p.reject(Error(m.error.message)):p.resolve(m.result);}};
const call=(method,params)=>new Promise((resolve,reject)=>{const id=++serial;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
async function evaluate(expression){const r=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw Error('客户端检查失败');return r.result.value;}
try{
 if(mode==='quit'){void evaluate(`window.__TAURI_INTERNALS__.invoke('finish_leave',{action:'quit'})`).catch(()=>{});await wait(700);ws.close();process.exit(0);}
 if(mode==='smoke'){
  const result=await evaluate(`(async()=>{
   const invoke=(cmd,args={})=>window.__TAURI_INTERNALS__.invoke(cmd,args);
   const state=await invoke('cloud_status');if(!state.email||!state.sync?.linked)throw Error('账号尚未就绪');
   const all=await invoke('list_items',{kind:'note'});
   let item=all.find(i=>i.title==='云同步验收：合成笔记 A');
   if(!item)item=await invoke('save_item',{input:{id:null,kind:'note',title:'云同步验收：合成笔记 A',body:'这是自动生成的验收内容，不是真实笔记。',bodyJson:null,revision:null}});
   await invoke('cloud_sync');const after=await invoke('cloud_status');
   return {id:item.id,pending:after.sync.pending,lastSuccess:after.sync.lastSuccess,error:after.error};
  })()`);
  console.log(JSON.stringify({check:'real-cloud-synthetic-push-and-pull',...result}));
 }
 if(mode==='prepare'){
  const testEmail=process.env.QINGJIAN_TEST_EMAIL;
  if(!testEmail)throw Error('请设置 QINGJIAN_TEST_EMAIL 为隔离验收邮箱');
  for(const label of ['设置','账号']){await evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(label)});if(!b)throw Error('按钮未就绪');b.click()})()`);await wait(400);}
  await evaluate(`(()=>{const e=document.querySelector('input[type=email]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(testEmail)});e.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  // 由用户点击发送并在界面输入验证码，自动化不接触验证码。
  await evaluate(`window.__TAURI_INTERNALS__.invoke('reveal_shell')`);
 }
 const s=await evaluate(`window.__TAURI_INTERNALS__.invoke('cloud_status')`);
 console.log(JSON.stringify({configured:s.configured,email:s.email,phase:s.phase,error:s.error,restartRequired:s.restartRequired,sync:s.sync}));
}finally{ws.close();}
