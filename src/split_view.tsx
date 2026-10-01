import { useEffect, useRef, useState, type ReactNode, type CSSProperties } from 'react';
import {useRunState} from './run_session';

export function SplitView({ list, detail, storageKey }: { list: ReactNode; detail: ReactNode; storageKey: string }) {
  const root = useRef<HTMLDivElement>(null);
  const [ratio, setRatio] = useRunState(storageKey+'.ratio',40);
  const [height, setHeight] = useRunState(storageKey+'.height',60);
  const cleanup = useRef<() => void>(() => {});
  useEffect(() => () => cleanup.current(), []);
  const move = (x: number, y: number) => {
    const r = root.current?.getBoundingClientRect(); if (!r) return;
    if (innerWidth >= 500) setRatio(Math.min(58, Math.max(32, (x-r.left)/r.width*100)));
    else setHeight(Math.min(Math.max(50, (r.height-120)/r.height*100), Math.max(Math.min(180,r.height*.45)/r.height*100, (y-r.top)/r.height*100)));
  };
  return <div ref={root} className="split-view" data-layout={storageKey} style={{'--ratio':`${ratio}%`,'--height':`${height}%`} as CSSProperties}>
    <div className="split-list">{list}</div>
    <div className="split-divider" role="separator" tabIndex={0} aria-label="调整列表与详情大小" onKeyDown={e=>{
      if(['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(e.key)){e.preventDefault();const d=['ArrowLeft','ArrowUp'].includes(e.key)?-3:3;if(innerWidth>=500)setRatio(r=>Math.max(32,Math.min(58,r+d)));else setHeight(r=>Math.max(35,Math.min(75,r+d)));}
    }} onPointerDown={e=>{
      if(e.button!==0)return;e.preventDefault();cleanup.current();
      // 窗口级监听保证越过细分界线后仍能连续拖动，卸载时清理。
      const update=(n:PointerEvent)=>move(n.clientX,n.clientY);
      const stop=()=>{window.removeEventListener('pointermove',update);window.removeEventListener('pointerup',stop);window.removeEventListener('pointercancel',stop);};
      cleanup.current=stop;window.addEventListener('pointermove',update);window.addEventListener('pointerup',stop);window.addEventListener('pointercancel',stop);
    }}/>
    <div className="split-detail">{detail}</div>
  </div>;
}
