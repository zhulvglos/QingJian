import {useLayoutEffect,useState,type Dispatch,type SetStateAction,type RefObject} from 'react';

// 只在当前 WebView 内存中保留，不使用 localStorage/sessionStorage；完整退出后自然清空。
const values=new Map<string,unknown>();
export function setRunValue(key:string,value:unknown){values.set(key,value);window.dispatchEvent(new CustomEvent('qingjian-run-state',{detail:key}));}
export function useRunState<T>(key:string,initial:T|(()=>T)):[T,Dispatch<SetStateAction<T>>]{
 const read=()=>{if(!values.has(key))values.set(key,typeof initial==='function'?(initial as ()=>T)():initial);return values.get(key) as T;};
 const [slot,setSlot]=useState(()=>({key,value:read()}));
 if(slot.key!==key)setSlot({key,value:read()});
 useLayoutEffect(()=>{const update=(event:Event)=>{if((event as CustomEvent).detail===key)setSlot({key,value:read()});};window.addEventListener('qingjian-run-state',update);return()=>window.removeEventListener('qingjian-run-state',update);},[key]);
 const set:Dispatch<SetStateAction<T>>=next=>{const previous=read();const value=typeof next==='function'?(next as (p:T)=>T)(previous):next;values.set(key,value);setSlot({key,value});};
 return [slot.key===key?slot.value:read(),set];
}
const positions=new Map<string,Record<string,{top:number;left:number}>>();
const scrollSelectors=['.split-list','.split-detail','.item-list','.editor-panel','.rich-editor','.rich-body','.readonly-body','.recording-body','.segment-list','textarea','.backup-entry-list'];
function scrollEntries(node:HTMLDivElement){return [['root',node] as const,...scrollSelectors.flatMap(s=>Array.from(node.querySelectorAll<HTMLElement>(s)).map((e,i)=>[s+':'+i,e] as const))];}
export function useRunScroll(root:RefObject<HTMLDivElement|null>,key:string){
 const captureNow=()=>{const node=root.current;if(node){const result:Record<string,{top:number;left:number}>={};for(const [id,e] of scrollEntries(node))result[id]={top:e.scrollTop,left:e.scrollLeft};positions.set(key,result);}};
 useLayoutEffect(()=>{
  const node=root.current;if(!node)return;const saved=positions.get(key)||{};
  const entries=()=>scrollEntries(node);
  let restoring=true;const apply=()=>{for(const [id,e] of entries()){const p=saved[id];if(p){e.scrollTop=p.top;e.scrollLeft=p.left;}}};
  // 缓存和编辑器可能异步加载，短暂观察 DOM；不保留隐藏页面或隐藏页面定时副作用。
  apply();const observer=new MutationObserver(apply);observer.observe(node,{childList:true,subtree:true});
  const frame=requestAnimationFrame(apply);const timer=window.setTimeout(()=>{apply();observer.disconnect();restoring=false;},600);
  const capture=()=>{if(restoring)return;const result:Record<string,{top:number;left:number}>={};for(const [id,e] of entries())result[id]={top:e.scrollTop,left:e.scrollLeft};positions.set(key,result);};
  // 用户已开始滚动或操作时，取消剩余恢复，避免延迟回填覆盖新的位置。
  const interacted=()=>{restoring=false;observer.disconnect();cancelAnimationFrame(frame);clearTimeout(timer);};
  node.addEventListener('scroll',capture,true);node.addEventListener('wheel',interacted,{passive:true});node.addEventListener('pointerdown',interacted);
  return()=>{observer.disconnect();cancelAnimationFrame(frame);clearTimeout(timer);node.removeEventListener('scroll',capture,true);node.removeEventListener('wheel',interacted);node.removeEventListener('pointerdown',interacted);};
 },[key]);
 return captureNow;
}
