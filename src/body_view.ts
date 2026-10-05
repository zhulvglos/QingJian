import {Extension} from '@tiptap/core';
import {Plugin,PluginKey} from '@tiptap/pm/state';
import {Decoration,DecorationSet} from '@tiptap/pm/view';
import type {Node as DocNode} from '@tiptap/pm/model';

export type BodyView='all'|'checked'|'unchecked'|'reminder';
export const bodyViewKey=new PluginKey<BodyView>('qingjian-body-view');

// 完整文档始终是唯一编辑源；装饰只隐藏节点，不生成可覆盖原文的筛选副本。
function matching(node:DocNode,mode:BodyView):boolean {
  if(mode==='all')return true;
  if(node.type.name==='taskItem'&&node.attrs.checked===(mode==='checked'))return true;
  let found=false;node.forEach(child=>{if(matching(child,mode))found=true;});return found;
}
function hiddenParagraphs(doc:DocNode,mode:BodyView){
  const result=new Map<string,string>();
  const visit=(node:DocNode,visible:boolean)=>{
    if(node.type.name==='taskItem')visible=node.attrs.checked===(mode==='checked');
    if(node.type.name==='paragraph'&&!visible)result.set(node.attrs.qjId,JSON.stringify(node.toJSON()));
    node.forEach(child=>visit(child,visible));
  };visit(doc,false);return result;
}
function hiddenBlocks(doc:DocNode,mode:BodyView){
  const blocks:string[]=[];
  const visit=(node:DocNode,visible=false)=>{
    node.forEach(child=>{
      const ownVisible=child.type.name==='taskItem'?child.attrs.checked===(mode==='checked'):visible;
      if(child.isBlock&&!ownVisible&&!matching(child,mode)){blocks.push(JSON.stringify(child.toJSON()));return;}
      visit(child,ownVisible);
    });
  };visit(doc);return blocks;
}
export const BodyViewFilter=Extension.create({
  name:'bodyViewFilter',
  addProseMirrorPlugins(){return [new Plugin<BodyView>({
    key:bodyViewKey,
    state:{init:()=> 'all',apply:(tr,mode)=>tr.getMeta(bodyViewKey)??mode},
    filterTransaction(tr,state){
      const mode=bodyViewKey.getState(state)??'all';
      if(!tr.docChanged||mode==='all'||mode==='reminder')return true;
      // 多段选择、全选删除、格式转换也不能改掉未显示的段落。
      const protectedNodes=hiddenParagraphs(state.doc,mode),next=new Map<string,string>();
      tr.doc.descendants(node=>{if(node.type.name==='paragraph')next.set(node.attrs.qjId,JSON.stringify(node.toJSON()));});
      for(const [id,value] of protectedNodes)if(next.get(id)!==value)return false;
      // 非任务标题、图片、代码块等也受保护，不能被跨隐藏节点的选择删除。
      const counts=new Map<string,number>();
      tr.doc.descendants(node=>{const key=JSON.stringify(node.toJSON());counts.set(key,(counts.get(key)??0)+1);});
      for(const value of hiddenBlocks(state.doc,mode)){
        const count=counts.get(value)??0;if(!count)return false;counts.set(value,count-1);
      }
      return true;
    },
    props:{decorations(state){
      const mode=bodyViewKey.getState(state)??'all';if(mode==='all'||mode==='reminder')return null;
      const decorations:Decoration[]=[];
      const visit=(node:DocNode,pos:number,ownVisible=false)=>{
        node.forEach((child,offset)=>{
          const start=pos+offset;
          const isTask=child.type.name==='taskItem';
          const visible=isTask?child.attrs.checked===(mode==='checked'):ownVisible;
          const hasMatch=matching(child,mode);
          if(!visible&&!hasMatch){decorations.push(Decoration.node(start,start+child.nodeSize,{class:'body-view-hidden'}));return;}
          if(isTask&&!visible)decorations.push(Decoration.node(start,start+child.nodeSize,{class:'body-view-ancestor'}));
          if(child.content.size)visit(child,start+1,visible);
        });
      };visit(state.doc,0);return DecorationSet.create(state.doc,decorations);
    }}
  })];}
});
