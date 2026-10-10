// 断网模拟只影响 B 子进程代理；重启后实际联网补同步，不改系统网络。
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {writeFile} from 'node:fs/promises';
import {connect,wait} from './live_client.mjs';
const root='D:/SOFTWARE/轻笺/isolated-test/cloud-live-evidence-20261010';
const evidence={scope:'真实服务，子进程不可达代理模拟断网；合成内容',checks:[]};
const a=await connect(9243);let b=await connect(9244);
async function quit(){void b.invoke('finish_leave',{action:'quit'}).catch(()=>{});await wait(1500);b.close();}
function start(offline){execFileSync(process.execPath,['tests/start_live_cloud.mjs','b','--resume',...(offline?['--offline']:[])],{windowsHide:true,stdio:'pipe'});}
try{
 await quit();start(true);b=await connect(9244);
 const item=await b.invoke('save_item',{input:{id:null,kind:'note',title:'合成离线重启补同步',body:'断网时已保存到 B 的 SQLite',bodyJson:null,revision:null}});
 await assert.rejects(()=>b.invoke('cloud_sync'),/网络连接失败/);
 assert.ok((await b.invoke('cloud_status')).sync.pending>0);assert.equal((await b.invoke('get_item',{id:item.id})).body,'断网时已保存到 B 的 SQLite');
 evidence.checks.push('Rust 真实网络请求失败，本地保存成功且队列待上传');
 await quit();start(true);b=await connect(9244);assert.equal((await b.invoke('get_item',{id:item.id})).body,'断网时已保存到 B 的 SQLite');assert.ok((await b.invoke('cloud_status')).sync.pending>0);
 evidence.checks.push('仍离线重启，内容和待同步队列未丢失');
 await quit();start(false);b=await connect(9244);await b.invoke('cloud_sync');await a.invoke('cloud_sync');assert.equal((await a.invoke('get_item',{id:item.id})).body,'断网时已保存到 B 的 SQLite');assert.equal((await b.invoke('cloud_status')).sync.pending,0);
 evidence.checks.push('恢复联网并重启，安全凭据恢复会话，补同步到真实云端和 A');evidence.result='passed';
}catch(e){evidence.result='failed';evidence.error=String(e);throw e;}
finally{await writeFile(root+'/live-offline-evidence.json',JSON.stringify(evidence,null,2));a.close();b.close();console.log(JSON.stringify(evidence));}
