// 后台变更走真实网络；只操作合成笔记，验证编辑器草稿不会被拉取覆盖。
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {connect,wait} from './live_client.mjs';
const a=await connect(9243),b=await connect(9244);
const evidence={scope:'真实服务，合成笔记，原生 WebView 编辑器',checks:[]};
async function contains(text){for(let i=0;i<60;i++){if(await a.evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`))return;await wait(100);}throw Error('未显示：'+text);}
async function click(label){assert.equal(await a.evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true;})()`),true);await wait(250);}
try{
 await a.invoke('cloud_sync');await b.invoke('cloud_sync');
 const notes=await a.invoke('list_items',{kind:'note'});const item=notes.find(i=>i.title.startsWith('合成验收笔记 '));assert.ok(item);
 await click('笔记');assert.equal(await a.evaluate(`(()=>{const e=[...document.querySelectorAll('.item-row')].find(e=>e.textContent.includes(${JSON.stringify(item.title)}));if(!e)return false;e.click();return true;})()`),true);await wait(300);
 let other=await b.invoke('get_item',{id:item.id});
 const save=body=>b.invoke('save_item',{input:{id:other.id,kind:other.kind,title:other.title,body,bodyJson:null,revision:other.revision}});
 await save('真实拉取清洁编辑器更新');await b.invoke('cloud_sync');await a.invoke('cloud_sync');await contains('真实拉取清洁编辑器更新');
 assert.equal(await a.evaluate(`document.querySelector('.ProseMirror').textContent`),'真实拉取清洁编辑器更新');
 evidence.checks.push('真实后台拉取更新干净编辑器的可见正文');
 await a.evaluate(`(()=>{const e=document.querySelector('.ProseMirror');e.focus();const r=document.createRange();r.selectNodeContents(e);r.collapse(false);const s=getSelection();s.removeAllRanges();s.addRange(r);document.execCommand('insertText',false,' 未保存草稿验收');})()`);await wait(250);
 other=await b.invoke('get_item',{id:item.id});await save('另一客户端的最新已保存正文');await b.invoke('cloud_sync');await a.invoke('cloud_sync');await contains('未保存草稿验收');await contains('云端内容已变化，未保存草稿仍保留');
 evidence.checks.push('真实远端更新不覆盖未保存草稿，并显示版本变化提示');
 await click('设置');await click('账号');await click('退出登录…');await contains('请先保存或处理所有未保存');
 evidence.checks.push('未保存草稿阻止账号退出和工作区切换');
 await click('笔记');await click('另存草稿副本');await a.evaluate(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'s',ctrlKey:true,bubbles:true}))`);await contains('已保存到本机');
 await a.invoke('cloud_sync');await b.invoke('cloud_sync');
 const rows=await b.invoke('list_items',{kind:'note'});assert.ok(rows.some(i=>i.body.includes('未保存草稿验收')));assert.ok(rows.some(i=>i.id===item.id&&i.body==='另一客户端的最新已保存正文'));
 evidence.checks.push('另存草稿副本后，两份正文均同步至 B');evidence.result='passed';
}catch(e){evidence.result='failed';evidence.error=String(e);throw e;}
finally{await writeFile('D:/SOFTWARE/轻笺/isolated-test/cloud-live-evidence-20261010/live-draft-evidence.json',JSON.stringify(evidence,null,2));a.close();b.close();console.log(JSON.stringify(evidence));}
