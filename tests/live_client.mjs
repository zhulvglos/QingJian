// 仅用于本机隔离客户端验收；不提取或记录认证令牌。
export const wait=ms=>new Promise(r=>setTimeout(r,ms));
export async function connect(port){
 let target;
 for(let i=0;i<80;i++){try{target=(await(await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t=>t.url==='http://tauri.localhost/');if(target)break;}catch{}await wait(150);}
 if(!target)throw Error('客户端未启动：'+port);
 const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
 let serial=0;const pending=new Map();
 ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(p){pending.delete(m.id);m.error?p.reject(Error(m.error.message)):p.resolve(m.result);}};
 ws.onclose=()=>{for(const p of pending.values())p.reject(Error('客户端已关闭'));pending.clear();};
 async function evaluate(expression){
  const id=++serial;const r=await new Promise((resolve,reject)=>{pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));});
  if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;
 }
 return {evaluate,invoke:async(cmd,args={})=>{const r=await evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)}).then(value=>({ok:true,value}),error=>({ok:false,error:String(error)}))`);if(!r.ok)throw Error(r.error);return r.value;},close:()=>ws.close()};
}
