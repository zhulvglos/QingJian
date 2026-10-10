// 隔离账号标记仅测试工作区行为，不表示真实邮箱认证；只用程序自带样例转写。
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
import {mkdir,writeFile,access} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {connect,wait} from './live_client.mjs';
const root=process.env.QJ_TEST_ROOT,exe=process.env.QJ_TEST_EXE,python=process.env.QJ_TEST_PYTHON,resources=process.env.QJ_VOICE_RESOURCES_ROOT;
for(const p of [root,exe,python,resources])assert.ok(p?.replaceAll('\\','/').toLowerCase().startsWith('d:/'));
assert.ok(root.includes('isolated-test'));
try{await access(root);throw Error('测试目录必须全新，禁止覆盖');}catch(e){if(e.code!=='ENOENT')throw e;}
for(const dir of ['data','local','temp'])await mkdir(`${root}/${dir}`,{recursive:true});
const account={id:randomUUID(),email:'synthetic-voice@example.invalid',project:'a'.repeat(64)};
await writeFile(`${root}/data/active-account.json`,JSON.stringify(account));
const original={root:resources,engine:'sensevoice',sensePython:`${resources}/sensevoice/python/python.exe`,senseRuntime:`${resources}/sensevoice/runtime`,senseModels:`${resources}/sensevoice/models`};
// 原库只放合成配置；不读取、复制正式数据库或账号密钥。
execFileSync(python,['-B','-c',"import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('create table app_settings(key text primary key,value text)'); c.execute('insert into app_settings values(?,?)',('audio_config',sys.argv[2])); c.commit(); c.close()",`${root}/data/qingjian-v1.sqlite3`,JSON.stringify(original)],{windowsHide:true,env:{...process.env,PYTHONDONTWRITEBYTECODE:'1'}});
const port=process.env.QJ_TEST_PORT||'9246';
const env={...process.env,QINGJIAN_DATA_ROOT:`${root}/data`,LOCALAPPDATA:`${root}/local`,TEMP:`${root}/temp`,TMP:`${root}/temp`,QINGJIAN_TEST_MODE:'1',QINGJIAN_DISABLE_MODEL_NEWS_STARTUP:'1',QINGJIAN_TEST_NEWS_SOURCE:'http://127.0.0.1:1',QINGJIAN_TEST_NEWS_TIMEOUT_MS:'200',WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,HTTP_PROXY:'http://127.0.0.1:9',HTTPS_PROXY:'http://127.0.0.1:9',ALL_PROXY:'http://127.0.0.1:9',NO_PROXY:'localhost,127.0.0.1'};
Object.assign(env,{http_proxy:env.HTTP_PROXY,https_proxy:env.HTTPS_PROXY,all_proxy:env.ALL_PROXY,no_proxy:env.NO_PROXY});
spawn(exe,[],{env,windowsHide:true,stdio:'ignore',detached:true}).unref();
const client=await connect(port);
const evidence={scope:'本机隔离合成账号工作区，复用已有资源；不等同真实邮箱登录或干净电脑验收',checks:[]};
try{
 let bridgeReady=false;
 for(let i=0;i<80;i++){bridgeReady=await client.evaluate(`typeof window.__TAURI_INTERNALS__?.invoke==='function'`);if(bridgeReady)break;await wait(150);}
 assert.ok(bridgeReady,'等待原生调用桥初始化');
 const config=await client.invoke('audio_config');assert.equal(config.available,true);assert.equal(config.reusedLocalResources,true);
 for(const key of ['sensePython','senseRuntime','senseModels'])assert.equal(config[key],original[key]);
 assert.ok(config.root.replaceAll('\\','/').startsWith(`${root}/data/accounts/`));
 evidence.checks.push('首次进入账号自动复用完整本机资源，账号录音目录保持独立');
 for(const label of ['设置','语音']){
  let ready=false;
  for(let i=0;i<60;i++){ready=await client.evaluate(`!![...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(label)})`);if(ready)break;await wait(200);}
  assert.ok(ready,'等待导航加载：'+label);
  await client.evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(label)}).click()`);await wait(500);
 }
 const ui=await client.evaluate(`(()=>{const s=document.querySelector('.local-voice-settings');const b=[...s.querySelectorAll('button')].find(b=>b.textContent==='检查可用性');return {enabled:!b.disabled,reuse:s.textContent.includes('已复用本机已有 SenseVoice')}})()`);
 assert.deepEqual(ui,{enabled:true,reuse:true});evidence.checks.push('语音页显示复用说明，检查可用性按钮可点击');
 console.log('开始已有模型实际转写检查');
 await client.invoke('check_local_model');assert.equal((await client.invoke('audio_config')).validation.transcribed,true);
 evidence.checks.push('已有资源使用程序自带样例实际转写通过');
 console.log('开始一键安装入口无重复下载验证');
 await client.invoke('install_sensevoice');
 assert.equal((await client.invoke('audio_config')).validation.transcribed,true);
 // 检查会写本机诊断日志；只禁止重复资源或下载文件，不误把日志当成重复安装。
 for(const name of ['python','runtime','models','python-3.11.9.zip','pip.whl']){
  await assert.rejects(()=>access(`${config.root}/sensevoice/${name}`),{code:'ENOENT'});
 }
 evidence.checks.push('离线代理下调用一键安装仍检查成功，未创建重复模型或下载目录');
 evidence.result='passed';
}catch(e){evidence.result='failed';evidence.error=String(e);throw e;}
finally{
 await writeFile(`${root}/evidence.json`,JSON.stringify(evidence,null,2));console.log(JSON.stringify(evidence));
 void client.invoke('finish_leave',{action:'quit'}).catch(()=>{});await wait(700);client.close();
}
