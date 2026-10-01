import {invoke} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
const style=document.createElement('style');
style.textContent='html,body{margin:0;width:100%;height:100%;overflow:hidden;background:transparent}button{width:100%;height:100%;padding:0;border:1px solid #c1844e;border-radius:6px;background:#99612f;color:#fff;font:12px sans-serif;cursor:pointer;writing-mode:vertical-rl}body.top button{writing-mode:horizontal-tb}button:hover{background:#b5763e}';document.head.append(style);
const reveal=()=>void invoke('reveal_shell');
document.querySelector('button')!.addEventListener('click',reveal);
document.querySelector('button')!.addEventListener('mouseenter',reveal);
void listen<string>('edge-side',e=>{document.body.className=e.payload;});
