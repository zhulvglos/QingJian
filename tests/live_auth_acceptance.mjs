// 使用已消费验证码之外的固定无效输入验证服务端限流；不记录真实验证码或会话令牌。
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {connect,wait} from './live_client.mjs';
const testEmail=process.env.QINGJIAN_TEST_EMAIL;
if(!testEmail)throw Error('请设置 QINGJIAN_TEST_EMAIL 为隔离验收邮箱');
let a=await connect(9243),b=await connect(9244);
const evidence={scope:'真实 Supabase Auth；两个客户端同邮箱；独立账号 RLS 另由回滚 SQL 验证',checks:[],limits:[]};
try{
 // Supabase 使用令牌桶，最多允许约 30 次短时突发；配置值控制补充速率。
 for(let i=0;i<36;i++){
  try{await b.invoke('cloud_verify',{email:testEmail,code:'000000'});throw Error('意外验证通过，停止测试');}
  catch(e){const message=String(e);assert.ok(/验证码错误或已过期|服务端限流/.test(message),message);evidence.limits.push(message.includes('服务端限流')?'server-rate-limited':'invalid-or-expired-rejected');if(message.includes('服务端限流'))break;}
 }
 assert.ok(evidence.limits.includes('server-rate-limited'));evidence.checks.push('直接调用服务端验证接口，无效验证码被拒绝，并触发真实服务端限流');
 await b.invoke('cloud_logout');assert.equal((await b.invoke('cloud_status')).restartRequired,true);await assert.rejects(()=>b.invoke('cloud_sync'));
 evidence.checks.push('B 退出删除本机凭据并停止同步');
 await a.invoke('cloud_sync');assert.equal((await a.invoke('cloud_status')).email,testEmail);evidence.checks.push('B 的本机会话退出不撤销 A 的独立会话');
 evidence.unverified=['真实已签发验证码静置 300 秒后的过期验证尚未单独测试','两台物理电脑及不同网络验收尚未完成'];evidence.result='passed';
}catch(e){evidence.result='failed';evidence.error=String(e);throw e;}
finally{await writeFile('D:/SOFTWARE/轻笺/isolated-test/cloud-live-evidence-20261010/live-auth-evidence.json',JSON.stringify(evidence,null,2));a.close();b.close();console.log(JSON.stringify(evidence));}
