// 隔离原生客户端验收。合成账号标记仅验证缓存隔离，绝不伪造认证或真实云同步成功。
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
import {mkdir,writeFile} from 'node:fs/promises';
import {openSync,closeSync} from 'node:fs';
import {createHash,randomUUID} from 'node:crypto';
const root=process.env.QJ_TEST_ROOT;
const exe=process.env.QJ_TEST_EXE;
assert.ok(root?.replaceAll('\\','/').toLowerCase().startsWith('d:/')&&root.includes('isolated-test'));
assert.ok(exe?.toLowerCase().startsWith('d:'));
const port=process.env.QJ_TEST_PORT||'9241';
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const a={id:randomUUID(),email:'synthetic-a@example.invalid',project:createHash('sha256').update('').digest('hex')};
const b={...a,id:randomUUID(),email:'synthetic-b@example.invalid'};
let child,ws,serial=0;const pending=new Map(),errors=[];
for(const p of ['data','local','temp'])await mkdir(`${root}/${p}`,{recursive:true});
const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++serial;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));});
async function evaluate(expression){const r=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description||r.exceptionDetails.text);return r.result.value;}
const invoke=(cmd,args={})=>evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
async function click(label){assert.equal(await evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(x=>x.getAttribute('aria-label')===${JSON.stringify(label)}||x.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true;})()`),true,`可点击：${label}`);await wait(180);}
async function contains(text){for(let i=0;i<60;i++){if(await evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`))return;await wait(100);}throw Error('未显示：'+text);}
async function capture(name){const r=await call('Page.captureScreenshot',{format:'png'});await writeFile(`${root}/${name}`,Buffer.from(r.data,'base64'));}
async function start(){
 const log=openSync(`${root}/native.log`,'a');
 child=spawn(exe,[],{windowsHide:true,stdio:['ignore',log,log],env:{...process.env,QINGJIAN_DATA_ROOT:`${root}/data`,LOCALAPPDATA:`${root}/local`,TEMP:`${root}/temp`,TMP:`${root}/temp`,QINGJIAN_TEST_MODE:'1',QINGJIAN_DISABLE_MODEL_NEWS_STARTUP:'1',QINGJIAN_TEST_NEWS_SOURCE:'http://127.0.0.1:1',QINGJIAN_TEST_NEWS_TIMEOUT_MS:'200',WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});closeSync(log);
 let target;for(let i=0;i<100;i++){try{target=(await(await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t=>t.url==='http://tauri.localhost/');if(target)break;}catch{}await wait(100);}
 assert.ok(target,'原生主窗口启动');ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
 ws.onmessage=e=>{const m=JSON.parse(e.data);if(m.method==='Runtime.exceptionThrown')errors.push(m.params);const p=pending.get(m.id);if(p){pending.delete(m.id);m.error?p.reject(m.error):p.resolve(m.result);}};
 await call('Runtime.enable');for(let i=0;i<80;i++){if(await evaluate(`!!document.querySelector('.primary-nav')`)){try{await invoke('cloud_status');return;}catch{}}await wait(100);}throw Error('前端未就绪');
}
async function quit(){void invoke('finish_leave',{action:'quit'}).catch(()=>{});for(let i=0;i<80&&child.exitCode===null;i++)await wait(100);assert.notEqual(child.exitCode,null);ws.close();await wait(350);}
const marker=account=>writeFile(`${root}/data/active-account.json`,JSON.stringify(account));
const accountDb=account=>`${root}/data/accounts/${account.project}/${account.id}/data/qingjian-v1.sqlite3`;
function sql(path,statement){execFileSync(process.env.QJ_TEST_PYTHON||'python',['-B','-c','import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.executescript(sys.argv[2]);c.commit();c.close()',path,statement],{windowsHide:true,env:process.env});}
const evidence={scope:'原生隔离测试；合成账号；无真实认证、发信、云数据库或第二台电脑',checks:[]};
try{
 await start();assert.equal((await invoke('cloud_status')).configured,false);await click('设置');await click('账号');await contains('尚未配置云端');
 assert.equal(await evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent==='获取 / 重新发送验证码').disabled`),true);
 await assert.rejects(()=>invoke('cloud_verify',{email:'test@qq.com',code:'123456'}));assert.equal((await invoke('cloud_status')).email,null);
 const item=await invoke('save_item',{input:{id:null,kind:'note',title:'本机测试笔记',body:'本机原始正文',bodyJson:null,revision:null}});
 const sticky=await invoke('save_item',{input:{id:null,kind:'sticky',title:'本机测试便签',body:'便签正文',bodyJson:null,revision:null}});
 await invoke('save_reminder',{itemId:item.id,dueAt:'2030-10-10T12:00:00Z'});
 await invoke('set_item_pinned',{id:sticky.id,pinned:true});await invoke('trash_item',{id:sticky.id,revision:sticky.revision});await invoke('restore_item',{id:sticky.id});
 const backup=`${root}/local-content.qjbackup`;await invoke('export_backup',{path:backup,preferences:{theme:'warm_apricot',fontSize:'standard'}});assert.equal((await invoke('preview_backup',{path:backup})).items,2);
 evidence.checks.push('无配置不伪造登录；本地保存、置顶、删除恢复、提醒、备份预览正常');await capture('local-account.png');await quit();
 await marker(a);await start();assert.equal((await invoke('list_items',{kind:'note'})).length,0);await click('设置');await click('账号');await contains('首次关联本机内容');
 assert.equal((await invoke('cloud_status')).sync.pending,0);await capture('first-link.png');
 await click('备份并加入此账号…');await contains('确认备份并加入');await click('确认备份并加入');await contains('已关联 2 条');
 assert.equal((await invoke('list_items',{kind:'note'})).length,1);assert.equal((await invoke('list_reminders')).length,0);assert.equal((await invoke('cloud_status')).sync.linked,true);
 assert.equal((await invoke('cloud_link_local',{include:true})).alreadyLinked,true);
 await assert.rejects(()=>invoke('cloud_sync'));assert.equal((await invoke('get_item',{id:item.id})).body,'本机原始正文');
 evidence.checks.push('首次关联确认、实际一致性快照、重复关联去重；提醒不复制；同步失败不影响本地保存');
 await click('笔记');await evaluate(`document.querySelector('.item-row').click()`);await wait(180);
 sql(accountDb(a),`BEGIN;UPDATE sync_control SET applying=1;UPDATE items SET body='合成远端清洁更新',body_json=NULL,revision=revision+1 WHERE id='${item.id}';UPDATE sync_control SET applying=0;COMMIT;`);
 await invoke('plugin:event|emit',{event:'cloud-content-changed',payload:null});await contains('合成远端清洁更新');
 assert.equal(await evaluate(`document.querySelector('.ProseMirror').textContent`),'合成远端清洁更新');

 await evaluate(`(()=>{const e=document.querySelector('.ProseMirror');e.focus();const r=document.createRange();r.selectNodeContents(e);r.collapse(false);const s=getSelection();s.removeAllRanges();s.addRange(r);document.execCommand('insertText',false,' 未保存草稿');})()`);await wait(180);
 sql(accountDb(a),`BEGIN;UPDATE sync_control SET applying=1;UPDATE items SET body='合成远端修改',body_json=NULL,revision=revision+1 WHERE id='${item.id}';UPDATE sync_control SET applying=0;COMMIT;`);
 await invoke('plugin:event|emit',{event:'cloud-content-changed',payload:null});await contains('云端内容已变化，未保存草稿仍保留');await contains('未保存草稿');
 await click('设置');await click('账号');await click('退出登录…');await contains('请先保存或处理所有未保存');await capture('draft-guard.png');
 await click('笔记');await click('另存草稿副本');await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'s',ctrlKey:true,bubbles:true}))`);await contains('已保存到本机');
 const notes=await invoke('list_items',{kind:'note'});assert.equal(notes.length,2);assert.ok(notes.some(n=>n.body.includes('未保存草稿')));assert.ok(notes.some(n=>n.body==='合成远端修改'));
 evidence.checks.push('后台变更刷新干净编辑器、保留未保存草稿；退出被草稿保护拦截；另存副本保留两个正文');
 await click('设置');await click('账号');await click('退出登录…');await click('退出并停止同步');await contains('请重启轻笺');assert.equal((await invoke('cloud_status')).restartRequired,true);await quit();
 await start();assert.equal((await invoke('cloud_status')).email,null);assert.equal((await invoke('list_items',{kind:'note'})).length,1);assert.equal((await invoke('get_item',{id:item.id})).body,'本机原始正文');await quit();
 await marker(b);await start();assert.equal((await invoke('list_items',{kind:'note'})).length,0);assert.equal((await invoke('list_reminders')).length,0);assert.equal((await invoke('cloud_status')).sync.pending,0);await invoke('cloud_link_local',{include:false});assert.equal((await invoke('list_items',{kind:'note'})).length,0);await quit();
 await marker(a);await start();assert.equal((await invoke('list_items',{kind:'note'})).length,2);assert.ok((await invoke('cloud_status')).sync.pending>=1);await quit();
 evidence.checks.push('退出后本地原稿不变；合成账号 B 不见 A 的缓存、提醒及队列；A 重启后队列仍存在');
 assert.deepEqual(errors,[]);evidence.result='passed';
}catch(e){evidence.result='failed';evidence.error=String(e);try{await capture('failure.png');await quit();}catch{}throw e;}finally{await writeFile(`${root}/native-cloud-evidence.json`,JSON.stringify(evidence,null,2));}
console.log(JSON.stringify(evidence));
