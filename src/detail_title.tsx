import {useLayoutEffect,useRef,useState} from 'react';

export function DetailTitle({title,label,onChange}:{title:string;label:string;onChange:(title:string)=>void}){
  const [editing,setEditing]=useState(false),input=useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(()=>{
    const el=input.current;if(!el)return;
    const grow=()=>{
      el.style.height='auto';
      // 新版 WebView 原生随内容布局，缩放/滚动条改变可用宽度时也同步计算。
      if(!CSS.supports('field-sizing','content'))el.style.height=(el.scrollHeight+2)+'px';
    };
    grow();let width=el.clientWidth;
    const resize=new ResizeObserver(()=>{if(el.clientWidth!==width){width=el.clientWidth;grow();}});
    resize.observe(el);
    const theme=new MutationObserver(grow);theme.observe(document.documentElement,{attributes:true});
    window.addEventListener('resize',grow);
    return()=>{resize.disconnect();theme.disconnect();window.removeEventListener('resize',grow);};
  },[title,editing]);
  useLayoutEffect(()=>{if(editing)input.current?.focus();},[editing]);
  // 视觉换行不写入标题；结束编辑只收起控件，不触发保存。
  return editing?<textarea ref={input} rows={1} className="detail-title title-edit" aria-label={label+'标题'} value={title} placeholder={'无标题'+label}
    onChange={e=>onChange(e.target.value.replace(/[\r\n]+/g,' '))} onBlur={()=>setEditing(false)}
    onKeyDown={e=>{if(e.key==='Enter'&&!e.nativeEvent.isComposing){e.preventDefault();e.currentTarget.blur();}}}/>
    :<button className="detail-title title-display" aria-label={label+'标题'} title={title||'无标题'+label} onClick={()=>setEditing(true)}>{title||'无标题'+label}</button>;
}
