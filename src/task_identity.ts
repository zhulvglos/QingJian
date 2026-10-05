import {Extension,type JSONContent} from '@tiptap/core';
import {Plugin} from '@tiptap/pm/state';
import type {Node} from '@tiptap/pm/model';
// 身份与完成状态存在段落上，切换列表节点后仍随内容保存，不靠行号或临时数组。
export function identifyDocument(document:JSONContent):JSONContent{
 const copy=structuredClone(document),seen=new Set<string>();
 const walk=(node:JSONContent,checked?:boolean)=>{if(node.type==='paragraph'){
  let id=node.attrs?.qjId as string|undefined;const duplicate=!!id&&seen.has(id);if(!id||duplicate)id=crypto.randomUUID();seen.add(id);
  node.attrs={...node.attrs,qjId:id,qjChecked:duplicate?false:checked??node.attrs?.qjChecked??false};
 }for(const child of node.content||[])walk(child,node.type==='taskItem'?node.attrs?.checked===true:checked);};walk(copy);return copy;
}
export const TaskIdentity=Extension.create({name:'qingjianTaskIdentity',addGlobalAttributes(){return [{types:['paragraph'],attributes:{qjId:{default:null,renderHTML:()=>({})},qjChecked:{default:false,renderHTML:()=>({})}}}];},addProseMirrorPlugins(){return [new Plugin({appendTransaction(transactions,oldState,state){
 if(!transactions.some(t=>t.docChanged))return null;
 const previous=new Map<string,{task:boolean;checked:boolean}>();
 oldState.doc.descendants((n,pos)=>{if(n.type.name==='paragraph'&&n.attrs.qjId){const parent=oldState.doc.resolve(pos).parent;previous.set(n.attrs.qjId,{task:parent.type.name==='taskItem',checked:n.attrs.qjChecked===true});}});
 const seen=new Set<string>(),tr=state.tr;const taskUpdates=new Map<number,boolean>();
 state.doc.descendants((node:Node,pos:number)=>{if(node.type.name!=='paragraph')return;const resolved=state.doc.resolve(pos),parent=resolved.parent;const task=parent.type.name==='taskItem';let id=node.attrs.qjId as string|null;const duplicate=!!id&&seen.has(id);if(!id||duplicate)id=crypto.randomUUID();seen.add(id);
  // 拆分产生的新段落获得新身份且未勾选；合并沿用存活段落身份，不复制完成状态。
  const prior=previous.get(id);const checked=duplicate?false:task?(prior?.task?parent.attrs.checked===true:node.attrs.qjChecked===true):node.attrs.qjChecked===true;
  if(node.attrs.qjId!==id||node.attrs.qjChecked!==checked)tr.setNodeMarkup(pos,undefined,{...node.attrs,qjId:id,qjChecked:checked});
  if(task&&parent.attrs.checked!==checked)taskUpdates.set(resolved.before(),checked);
 });for(const [pos,checked] of taskUpdates){const node=tr.doc.nodeAt(pos);if(node)tr.setNodeMarkup(pos,undefined,{...node.attrs,checked});}
 return tr.docChanged?tr:null;
}})];}});
