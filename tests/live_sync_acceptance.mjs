// 两个真实登录、不同数据根目录的客户端连接真实 Supabase；全部内容为合成数据。
import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {connect} from './live_client.mjs';
const testEmail=process.env.QINGJIAN_TEST_EMAIL;
if(!testEmail)throw Error('请设置 QINGJIAN_TEST_EMAIL 为隔离验收邮箱');
const a=await connect(9243),b=await connect(9244);
const root='D:/SOFTWARE/轻笺/isolated-test/cloud-live-evidence-20261010';await mkdir(root,{recursive:true});
const evidence={scope:'同一 Windows 电脑，两个独立 SQLite/WebView/安全凭据范围；真实 Supabase，合成内容；不等同两台真实电脑',started:new Date().toISOString(),checks:[]};
const sync=c=>c.invoke('cloud_sync');
const get=(c,id)=>c.invoke('get_item',{id});
const save=(c,item,body)=>c.invoke('save_item',{input:{id:item.id,kind:item.kind,title:item.title,body,bodyJson:null,revision:item.revision}});
const round=async()=>{await sync(a);await sync(b);await sync(a);};
try{
 for(const c of [a,b]){const s=await c.invoke('cloud_status');assert.equal(s.email,testEmail);assert.equal(s.sync.linked,true);assert.equal(s.restartRequired,false);}
 await round();evidence.checks.push('两个独立客户端重启后使用安全凭据恢复真实会话并成功同步');
 const rich=JSON.stringify({format:'qingjian-rich-v1',mode:'paragraph',checks:[],document:{type:'doc',content:[{type:'paragraph',content:[{type:'text',text:'合成富文本',marks:[{type:'bold'}]}]}]}});
 const note=await a.invoke('save_item',{input:{id:null,kind:'note',title:'合成验收笔记 '+randomUUID().slice(0,8),body:'合成富文本',bodyJson:rich,revision:null}});
 await round();const nb=await get(b,note.id);assert.equal(nb.body,'合成富文本');assert.equal(nb.bodyJson,rich);
 await save(b,nb,'B 编辑的合成正文');await sync(b);await sync(a);assert.equal((await get(a,note.id)).body,'B 编辑的合成正文');
 evidence.checks.push('A 新增笔记及富文本上传，B 拉取、编辑，A 拉回修改');
 const sticky=await b.invoke('save_item',{input:{id:null,kind:'sticky',title:'合成便签',body:'B 新增便签',bodyJson:null,revision:null}});
 await sync(b);await sync(a);assert.equal((await get(a,sticky.id)).body,'B 新增便签');
 await a.invoke('set_item_pinned',{id:sticky.id,pinned:true});await round();assert.equal((await get(b,sticky.id)).isPinned,true);
 evidence.checks.push('B 新增便签同步到 A，A 置顶同步到 B');
 const na=await get(a,note.id);await a.invoke('trash_item',{id:note.id,revision:na.revision});await round();
 assert.ok((await b.invoke('list_trashed',{kind:'note'})).some(i=>i.id===note.id));
 await b.invoke('restore_item',{id:note.id});await sync(b);await sync(a);assert.ok(await get(a,note.id));
 evidence.checks.push('A 软删除、B 回收站可见，B 恢复后 A 恢复');
 // 两边先读取同一云端基线，再保存不同正文，验证 CAS 保留冲突副本。
 const va=await get(a,note.id),vb=await get(b,note.id);
 await save(a,va,'并发版本 A');await save(b,vb,'并发版本 B');await round();await round();
 const allA=await a.invoke('list_items',{kind:'note'}),allB=await b.invoke('list_items',{kind:'note'});
 for(const rows of [allA,allB]){assert.ok(rows.some(i=>i.body==='并发版本 A'));assert.ok(rows.some(i=>i.body==='并发版本 B'));}
 const stateB=await b.invoke('cloud_status');assert.ok(stateB.sync.conflicts.length>0);
 evidence.checks.push('同一云版本并发修改保留 A/B 两个正文，冲突副本同步到两端');
 const idsA=allA.map(i=>i.id).sort();await round();assert.deepEqual((await a.invoke('list_items',{kind:'note'})).map(i=>i.id).sort(),idsA);
 evidence.checks.push('重复同步未产生重复条目或重复冲突副本');
 const st=await get(a,sticky.id);await a.invoke('trash_item',{id:sticky.id,revision:st.revision});await sync(a);await sync(b);
 await a.invoke('delete_item_forever',{id:sticky.id});await round();
 assert.ok(!(await b.invoke('list_trashed',{kind:'sticky'})).some(i=>i.id===sticky.id));
 evidence.checks.push('永久删除墓碑同步，另一客户端移除条目');
 for(const c of [a,b])assert.equal((await c.invoke('cloud_status')).sync.pending,0);
 evidence.result='passed';evidence.completed=new Date().toISOString();
}catch(e){evidence.result='failed';evidence.error=String(e);throw e;}
finally{await writeFile(root+'/live-sync-evidence.json',JSON.stringify(evidence,null,2));a.close();b.close();console.log(JSON.stringify(evidence));}
