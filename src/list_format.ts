import type {JSONContent} from '@tiptap/core';
// 按段落身份转换，不调用会清空段落属性的 clearNodes；复杂结构原样保留。
export function formatParagraphs(document:JSONContent,ids:Set<string>,target:'taskList'|'orderedList'|'bulletList'):{document:JSONContent;skipped:boolean}{
 let skipped=false;const content:JSONContent[]=[];
 const wrap=(p:JSONContent)=>({type:target,content:[{type:target==='taskList'?'taskItem':'listItem',...(target==='taskList'?{attrs:{checked:p.attrs?.qjChecked===true}}:{}),content:[p]}]});
 for(const block of document.content||[]){
  if(block.type==='paragraph'){content.push(ids.has(block.attrs?.qjId)?wrap(block):block);continue;}
  if(!['taskList','orderedList','bulletList'].includes(block.type||'')){content.push(block);continue;}
  let group:JSONContent|null=null;
  for(const item of block.content||[]){const p=item.content?.[0],selected=p?.type==='paragraph'&&ids.has(p.attrs?.qjId);const simple=item.content?.length===1&&p?.type==='paragraph';
   if(selected&&!simple)skipped=true;
   const type=selected&&simple?target:block.type;
   const next=selected&&simple?wrap(p!).content[0]:item;
   if(!group||group.type!==type){group={...block,type,content:[]};content.push(group);}group.content!.push(next);
  }
 }
 return {document:{...document,content},skipped};
}
