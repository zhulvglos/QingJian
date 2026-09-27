import {useEffect,useLayoutEffect,useRef,useState} from 'react';
import {createPortal} from 'react-dom';

export function TimeSelect({label,value,count,onChange}:{label:string;value:string;count:number;onChange:(v:string)=>void}){
  const [open,setOpen]=useState(false),[cursor,setCursor]=useState(Number(value)),[position,setPosition]=useState({left:0,top:0,height:192});
  const trigger=useRef<HTMLButtonElement>(null),panel=useRef<HTMLDivElement>(null);
  useLayoutEffect(()=>{if(!open)return;const r=trigger.current!.getBoundingClientRect();const h=Math.min(194,innerHeight-16);setPosition({left:Math.max(8,Math.min(innerWidth-80,r.left)),top:Math.max(8,Math.min(innerHeight-h-8,r.bottom+3)),height:h});panel.current?.focus();},[open]);
  useEffect(()=>{if(open)panel.current?.querySelector(`[data-index="${cursor}"]`)?.scrollIntoView({block:'nearest'});},[open,cursor]);
  useEffect(()=>{if(!open)return;const close=(e:PointerEvent)=>{if(!panel.current?.contains(e.target as Node)&&!trigger.current?.contains(e.target as Node))setOpen(false);};const resize=()=>setOpen(false);window.addEventListener('pointerdown',close);window.addEventListener('resize',resize);return()=>{window.removeEventListener('pointerdown',close);window.removeEventListener('resize',resize);};},[open]);
  const finish=(n?:number)=>{if(n!==undefined)onChange(String(n).padStart(2,'0'));setOpen(false);trigger.current?.focus();};
  return <><button ref={trigger} type="button" className="time-trigger" aria-label={label} aria-haspopup="listbox" aria-expanded={open} onClick={()=>{setCursor(Number(value));setOpen(!open);}}>{value} ▾</button>{open&&createPortal(<div ref={panel} className="time-options" role="listbox" aria-label={label+'选项'} tabIndex={0} aria-activedescendant={`time-${label}-${cursor}`} style={{left:position.left,top:position.top,maxHeight:position.height}} onKeyDown={e=>{
    // 方向键只移动候选值；Enter提交，Esc退出，Tab按正常焦点顺序离开。
    if(['ArrowUp','ArrowDown','Home','End','PageUp','PageDown'].includes(e.key)){e.preventDefault();setCursor(n=>Math.max(0,Math.min(count-1,e.key==='Home'?0:e.key==='End'?count-1:n+({ArrowUp:-1,ArrowDown:1,PageUp:-6,PageDown:6}[e.key]||0))));}
    else if(e.key==='Enter'||e.key===' '){e.preventDefault();finish(cursor);}else if(e.key==='Escape'){e.preventDefault();e.stopPropagation();finish();}else if(e.key==='Tab')setOpen(false);
  }}>{Array.from({length:count},(_,n)=><button key={n} id={`time-${label}-${n}`} data-index={n} type="button" role="option" tabIndex={-1} aria-selected={Number(value)===n} className={cursor===n?'active':''} onMouseEnter={()=>setCursor(n)} onClick={()=>finish(n)}>{String(n).padStart(2,'0')}</button>)}</div>,document.body)}</>;
}
