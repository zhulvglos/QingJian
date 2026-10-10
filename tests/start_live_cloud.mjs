// 使用全新 D 盘目录启动真实云端验收，禁止覆盖已有测试库或安装版数据。
import {mkdir,writeFile,access} from 'node:fs/promises';
import {spawn} from 'node:child_process';
// 连接配置从环境传入，禁止将维护者项目或个人账号固定在仓库。
const cloudUrl=process.env.QINGJIAN_TEST_SUPABASE_URL,publicKey=process.env.QINGJIAN_TEST_PUBLIC_KEY;
if(!cloudUrl||!publicKey)throw Error('请设置 QINGJIAN_TEST_SUPABASE_URL 与 QINGJIAN_TEST_PUBLIC_KEY');
const client=process.argv[2]||'a';if(!['a','b'].includes(client))throw Error('仅支持隔离客户端 a / b');
const root=`D:/SOFTWARE/轻笺/isolated-test/cloud-live-${client}-20261010`;
const port=client==='a'?'9243':'9244';
const resume=process.argv.includes('--resume'),offline=process.argv.includes('--offline');
try{await access(`${root}/data/qingjian-v1.sqlite3`);if(!resume)throw Error('测试库已存在，禁止覆盖');}catch(e){if(e.code!=='ENOENT')throw e;if(resume)throw Error('不能恢复不存在的测试库');}
for(const p of ['data','local','temp'])await mkdir(`${root}/${p}`,{recursive:true});
if(!resume)await writeFile(`${root}/data/cloud-config.json`,JSON.stringify({url:cloudUrl,publicKey}));
// 仅给此测试子进程设置不可达代理，模拟 Rust 网络请求失败；不改变系统网络或真实应用。
const proxyEnv=offline?{HTTPS_PROXY:'http://127.0.0.1:9',HTTP_PROXY:'http://127.0.0.1:9',ALL_PROXY:'http://127.0.0.1:9',NO_PROXY:'',https_proxy:'http://127.0.0.1:9',http_proxy:'http://127.0.0.1:9',all_proxy:'http://127.0.0.1:9',no_proxy:''}:{};
const child=spawn('D:/SOFTWARE/轻笺/source/src-tauri/target/x86_64-pc-windows-gnu/release/qingjian_shell.exe',[],{detached:true,windowsHide:true,stdio:'ignore',env:{...process.env,...proxyEnv,QINGJIAN_DATA_ROOT:`${root}/data`,LOCALAPPDATA:`${root}/local`,TEMP:`${root}/temp`,TMP:`${root}/temp`,QINGJIAN_TEST_MODE:'1',QINGJIAN_DISABLE_MODEL_NEWS_STARTUP:'1',QINGJIAN_TEST_NEWS_SOURCE:'http://127.0.0.1:1',QINGJIAN_TEST_NEWS_TIMEOUT_MS:'200',WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
child.on('error',()=>{console.error('隔离客户端启动失败');process.exitCode=1;});child.unref();console.log(JSON.stringify({pid:child.pid,root}));
