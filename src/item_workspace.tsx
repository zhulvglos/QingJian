import {createPortal} from 'react-dom';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { emitTo } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { RichBodyEditor, ReadOnlyBody, BodyViewTools } from './rich_document';
import { CalendarPicker } from './calendar_picker';
import {useRunState} from './run_session';
import type {BodyView} from './body_view';
import {DetailTitle} from './detail_title';
import {markReminderDraft} from './reminder_drafts';

export type ItemKind = 'sticky' | 'note';
export type Item = { id: string; kind: ItemKind; title: string; body: string; bodyJson: string | null; createdAt: string; updatedAt: string; revision: number; isPinned: boolean; sortOrder: number };
export type Draft = { id?: string; kind: ItemKind; title: string; body: string; bodyJson: string | null; updatedAt?: string; revision?: number; savedTitle: string; savedBody: string; savedBodyJson: string | null };
export type ItemView = '+ 新建' | '待办' | '已完成' | '回收站';
export type Reminder = { occurrenceId: string; itemId: string; kind: ItemKind; title: string; dueAt: string; status: 'pending' | 'completed'; completedAt: string | null; isCurrent: boolean };

type Props = {
  kind: ItemKind; view: ItemView; items: Item[]; trashed: Item[]; reminders: Reminder[]; draft: Draft | null; editorSession: number;
  loading: boolean; error: string; saveError: string; status: string;
  onSelect: (item: Item) => void; onTitleChange: (title: string) => void; onBodyChange: (body: string, bodyJson: string) => void;
  onDelete: (item: Item) => void; onRestore: (item: Item) => void; onPermanent: (item: Item) => void; onReload: () => void;
  onImport:()=>void; onExport:(ids:string[],format:string,includeSettings:boolean)=>Promise<boolean>;
  onTogglePinned: (item: Item) => void; onMove: (item: Item, target: Item) => void;
  onSaveReminder: (itemId: string, dueAt: string) => Promise<boolean>; onCancelReminder: (itemId: string) => Promise<boolean>; onCompleteReminder: (occurrenceId: string) => Promise<boolean>;
};

function localTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
}
function monthDay(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : `${date.getMonth() + 1}月${date.getDate()}日`;
}

function dateInputValue(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

export function ItemWorkspace({ kind, view, items, trashed, reminders, draft, editorSession, loading, error, saveError, status, onSelect, onTitleChange, onBodyChange, onDelete, onRestore, onPermanent, onReload, onImport,onExport,onTogglePinned, onMove, onSaveReminder, onCancelReminder, onCompleteReminder }: Props) {
  const [listMenu,setListMenu]=useState(false),[exportMode,setExportMode]=useState(false),[exportIds,setExportIds]=useState<string[]>([]),[exportBusy,setExportBusy]=useState(false);
  const label = kind === 'sticky' ? '便签' : '笔记';
  const [exportFormat,setExportFormat]=useState('md'),[includeSettings,setIncludeSettings]=useState(false);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const gestureRef = useRef<{ id: string; x: number; y: number; lastX: number; swiping: boolean } | null>(null);
  const suppressClickRef = useRef(false);
  const [split, setSplit] = useRunState(kind+'.split',41);
  const [heightSplit, setHeightSplit] = useRunState(kind+'.heightSplit',63);
  const [swipeOpen, setSwipeOpen] = useState<string | null>(null);
  const [contextItem, setContextItem] = useState<{ item: Item; x: number; y: number } | null>(null);
  const [toolsTarget, setToolsTarget] = useState<HTMLDivElement | null>(null);
  const [reminderError, setReminderError] = useState('');
  const [reminderBusy, setReminderBusy] = useState(false);
  const [readSelection, setReadSelection] = useRunState<{view: string; id: string} | null>(kind+'.'+view+'.readSelection',null);
  const readOnly = view === '待办' || view === '已完成';
  const activeReminder = reminders.find((entry) => entry.itemId === draft?.id && entry.isCurrent);
  const completedHistory = reminders.filter((entry) => entry.itemId === draft?.id && entry.status === 'completed');

  const entries = useMemo<Item[]>(() => {
    if (view === '回收站') return trashed;
    if (view === '待办' || view === '已完成') {
      const status = view === '待办' ? 'pending' : 'completed';
      // 二级导航只依据本篇内容的有效提醒状态，不读取正文方框。
      const ids = new Set(reminders.filter((entry) => entry.kind === kind && entry.status === status).map((entry) => entry.itemId));
      return items.filter((item) => ids.has(item.id));
    }
    return items;
  }, [items, trashed, reminders, kind, view]);

  // 只读选择独立于编辑草稿；刷新或完成提醒后，已消失的选择回退到第一条。
  const selectedSaved = entries.find(item => readSelection?.view === kind + view && item.id === readSelection.id) ?? entries[0];
  const selectedId = readOnly ? selectedSaved?.id : draft?.id;
  const selectItem = (item: Item) => readOnly ? setReadSelection({view: kind + view, id: item.id}) : onSelect(item);
  const selectedReminder = reminders.find(entry => entry.itemId === selectedSaved?.id && entry.status === 'pending' && entry.isCurrent);

  const ownerKey=kind+'.reminder.'+(selectedId??'new');
  const currentReminder=readOnly?selectedReminder:activeReminder;
  const [mode,setMode]=useRunState<BodyView>(kind+'.'+view+'.mode.'+(selectedId??'new'),'all');
  const [reminderDraft,setReminderDraft]=useRunState(ownerKey,()=>({value:currentReminder?dateInputValue(currentReminder.dueAt):dateInputValue(new Date(Date.now()+3_600_000).toISOString()),changed:false}));
  const reminderInput=reminderDraft.value;
  const setReminderInput=(value:string)=>{setReminderDraft({value,changed:true});markReminderDraft({key:ownerKey,id:selectedId,kind},true);};
  const reminderSaved=()=>{setReminderDraft({value:reminderInput,changed:false});markReminderDraft({key:ownerKey,id:selectedId,kind},false);setReminderError('');};
  useEffect(()=>{setReminderError('');if(!reminderDraft.changed&&currentReminder)setReminderDraft({value:dateInputValue(currentReminder.dueAt),changed:false});},[ownerKey,currentReminder?.dueAt]);

  const moveDivider = (x: number, y: number) => {
    const rect = workspaceRef.current?.getBoundingClientRect();
    if (!rect) return;
    // 与 CSS 媒体查询使用同一窗口断点，不能用扣除内边距后的正文宽度判断。
    if (window.innerWidth >= 500) {
      setSplit(Math.min(58, Math.max(32, ((x - rect.left) / rect.width) * 100)));
    } else {
      const min = Math.max(220, rect.height * .3);
      const max = Math.max(min, rect.height - 160);
      setHeightSplit(Math.max(min, Math.min(max, y - rect.top)) / rect.height * 100);
    }
  };

  useEffect(() => () => dragCleanupRef.current?.(), []);

  const startDividerDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    // 鼠标交给 MouseEvent 路径；WebView2 对鼠标的 Pointer Capture 在部分环境不连续。
    if (event.pointerType === 'mouse') return;
    event.preventDefault();
    dragCleanupRef.current?.();
    draggingRef.current = true;
    moveDivider(event.clientX, event.clientY);
    // WebView2 中鼠标离开细分界线时可能丢失元素级 move；窗口级监听保持拖动连续。
    const move = (next: PointerEvent | MouseEvent) => {
      if (draggingRef.current) moveDivider(next.clientX, next.clientY);
    };
    const stop = () => { dragCleanupRef.current?.(); };
    const cleanup = () => {
      draggingRef.current = false;
      window.removeEventListener('pointermove', move);
      window.removeEventListener('mousemove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('mouseup', stop);
      window.removeEventListener('pointercancel', stop);
      window.removeEventListener('blur', stop);
      dragCleanupRef.current = null;
    };
    dragCleanupRef.current = cleanup;
    window.addEventListener('pointermove', move);
    window.addEventListener('mousemove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('mouseup', stop);
    window.addEventListener('pointercancel', stop);
    window.addEventListener('blur', stop);
    try { event.currentTarget.setPointerCapture(event.pointerId); } catch { /* 鼠标兼容路径仍由窗口监听处理。 */ }
  };

  const startDividerMouseDrag = (event: React.MouseEvent<HTMLDivElement>) => {
    // 部分 WebView2/鼠标驱动只派发 MouseEvent，保留鼠标事件兜底。
    if (event.button !== 0 || draggingRef.current) return;
    event.preventDefault();
    draggingRef.current = true;
    moveDivider(event.clientX, event.clientY);
    const move = (next: MouseEvent) => moveDivider(next.clientX, next.clientY);
    const stop = () => {
      draggingRef.current = false;
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', stop);
      window.removeEventListener('blur', stop);
      dragCleanupRef.current = null;
    };
    dragCleanupRef.current = stop;
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', stop);
    window.addEventListener('blur', stop);
  };

  const openByKeyboard = (event: React.KeyboardEvent<HTMLButtonElement>, index: number, item: Item) => {
    if (view === '回收站') return;
    if (event.key === 'Delete') { event.preventDefault(); onDelete(item); return; }
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
    event.preventDefault();
    const buttons = listRef.current?.querySelectorAll<HTMLButtonElement>('.item-row');
    const next = Math.max(0, Math.min(entries.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)));
    if (event.ctrlKey) {
      if (next !== index && entries[next].isPinned === item.isPinned) onMove(item, entries[next]);
      return;
    }
    buttons?.[next]?.focus();
    selectItem(entries[next]);
  };

  return <div ref={workspaceRef} data-native-dialog-busy={exportBusy?'true':undefined} className={'item-workspace ' + (view === '回收站' ? 'trash-workspace' : '')} style={{ '--split': `${split}%`, '--height-split': `${heightSplit}%` } as React.CSSProperties}>
    <section className="list-panel" aria-label={view === '回收站' ? label + '回收站' : label + '内容列表'}>
      <div className="item-list-toolbar"><span>{label} · {entries.length}条</span><button aria-label={label+'列表操作'} aria-expanded={listMenu} onClick={()=>setListMenu(v=>!v)}>⋯</button>
        {listMenu&&<><button className="list-menu-dismiss" aria-label="关闭列表菜单" onClick={()=>setListMenu(false)}/><div className="list-actions-menu" role="menu"><button role="menuitem" onClick={()=>{setListMenu(false);onImport();}}>导入文件…</button><button role="menuitem" disabled={view==='回收站'||!entries.length} onClick={()=>{setListMenu(false);setExportIds([]);setExportMode(true);}}>导出{label}…</button></div></>}
      </div>
      {exportMode&&<div className="list-export-toolbar export-controls"><label><input type="checkbox" disabled={exportBusy} checked={entries.length>0&&entries.every(e=>exportIds.includes(e.id))} onChange={e=>setExportIds(e.target.checked?entries.map(e=>e.id):[])}/>{view==='+ 新建'?'全选本栏目':`全选${view}筛选结果`}（{entries.length}条）</label><select aria-label="导出格式" disabled={exportBusy} value={exportFormat} onChange={e=>setExportFormat(e.target.value)}><option value="md">Markdown</option><option value="txt">TXT</option><option value="json">JSON</option><option value="docx">Word (.docx)</option><option value="qjbackup">轻笺备份 (.qjbackup)</option></select>{exportFormat==='qjbackup'&&<label><input type="checkbox" checked={includeSettings} onChange={e=>setIncludeSettings(e.target.checked)}/>包含全局设置</label>}</div>}
      {loading ? <div className="simple-list-state">正在读取…</div> : error ? <div className="list-error" role="alert">读取失败：{error}<button onClick={onReload}>重试</button></div> : entries.length === 0 ?
        <div className="simple-list-state">{view === '回收站' ? '回收站为空' : view === '待办' ? '暂无待办提醒' : view === '已完成' ? '暂无已完成提醒' : `暂无${label}`}</div> :
        <div ref={listRef} className="item-list">{entries.map((item, index) => exportMode ? <label className="export-item-row" key={item.id}><input type="checkbox" disabled={exportBusy} checked={exportIds.includes(item.id)} onChange={e=>setExportIds(ids=>e.target.checked?[...ids,item.id]:ids.filter(id=>id!==item.id))}/><strong>{item.title||'无标题'+label}</strong></label> : view === '回收站' ?
          <div className="trash-row" key={item.id}><div className="trash-row-main"><strong>{item.title.trim() || '无标题' + label}</strong><time>{monthDay(item.updatedAt)}</time></div><div className="trash-actions"><button onClick={() => onRestore(item)}>恢复</button><button onClick={() => onPermanent(item)}>永久删除</button></div></div> :
          <div className={'swipe-row ' + (swipeOpen === item.id ? 'open' : '')} key={item.id}>
            <button className="swipe-delete" type="button" tabIndex={swipeOpen === item.id ? 0 : -1} onClick={() => { setSwipeOpen(null); onDelete(item); }}>删除</button>
            <button type="button" className={'item-row ' + (selectedId === item.id ? 'selected' : '')}
              onClick={() => { if (suppressClickRef.current) { suppressClickRef.current = false; return; } if (swipeOpen) { setSwipeOpen(null); return; } selectItem(item); }}
              onKeyDown={(event) => openByKeyboard(event, index, item)}
              onContextMenu={(event) => { event.preventDefault(); setContextItem({ item, x: Math.min(event.clientX, window.innerWidth - 144), y: Math.min(event.clientY, window.innerHeight - 54) }); }}
              onPointerDown={(event) => { gestureRef.current = { id: item.id, x: event.clientX, y: event.clientY, lastX: event.clientX, swiping: false }; event.currentTarget.setPointerCapture(event.pointerId); }}
              onPointerMove={(event) => {
                const gesture = gestureRef.current;
                if (!gesture || gesture.id !== item.id) return;
                gesture.lastX = event.clientX;
                const dx = event.clientX - gesture.x;
                if (dx < -22 && Math.abs(dx) > Math.abs(event.clientY - gesture.y) * 1.2) {
                  gesture.swiping = true;
                  setSwipeOpen(gesture.id);
                }
              }}
              onPointerUp={(event) => {
                const gesture = gestureRef.current;
                if (gesture && (gesture.swiping || event.clientX - gesture.x < -36)) {
                  suppressClickRef.current = true;
                  setSwipeOpen(event.clientX - gesture.x < -36 ? gesture.id : null);
                }
                gestureRef.current = null;
                event.currentTarget.releasePointerCapture(event.pointerId);
              }}
              onPointerCancel={() => { gestureRef.current = null; }}
              title="左滑删除；右键置顶；Ctrl+方向键排序" aria-label={`${item.isPinned ? '已置顶，' : ''}${item.title.trim() || '无标题' + label}，${monthDay(item.updatedAt)}`}>
              <span className="item-row-copy"><strong>{item.isPinned && <span className="pinned-mark" aria-hidden="true">⌃ </span>}{item.title.trim() || '无标题' + label}</strong></span>
              <time>{monthDay(item.updatedAt)}</time>
            </button>
            <span className="drag-handle" draggable role="button" tabIndex={0} aria-label={`拖动固定${item.title.trim() || '无标题' + label}`} title="按住并拖到窗口外侧快捷标签"
              onDragStart={(event) => {
                event.dataTransfer.effectAllowed = 'copy';
                event.dataTransfer.setData('text/plain', item.id);
                event.dataTransfer.setData('application/x-qingjian-item', item.id);
                const preview = document.createElement('div');
                preview.className = 'quick-drag-preview';
                preview.textContent = '固定：' + (item.title.trim() || '无标题' + label);
                document.body.append(preview);
                event.dataTransfer.setDragImage(preview, 20, 18);
                window.setTimeout(() => preview.remove(), 0);
                void invoke('set_quick_drag_active', { active: true }).then(() => emitTo('quick', 'quick-drag-active', true));
              }}
              onDragEnd={() => {
                // 由 Rust 即时读取鼠标按键：Esc 取消时按键仍保持按下，松手放置时已释放。
                const finish = invoke<boolean>('complete_quick_drag', { id: item.id });
                void finish.catch((reason) => console.error('快捷标签固定失败', reason)).finally(() => {
                  void invoke('set_quick_drag_active', { active: false });
                  void emitTo('quick', 'quick-drag-active', false);
                });
              }}>⠿</span>
          </div>)}</div>}
      {exportMode&&<div className="list-export-toolbar export-footer"><small>已选 {entries.filter(e=>exportIds.includes(e.id)).length} 条</small><button disabled={exportBusy} onClick={()=>{setExportMode(false);setExportIds([]);}}>取消</button><button disabled={exportBusy||!exportIds.length} onClick={()=>void(async()=>{setExportBusy(true);try{if(await onExport(exportIds.filter(id=>entries.some(e=>e.id===id)),exportFormat,includeSettings)){setExportMode(false);setExportIds([]);}}finally{setExportBusy(false);}})()}>{exportBusy?'导出中…':'导出'}</button></div>}

    </section>
    {contextItem && <><div className="context-dismiss" onClick={() => setContextItem(null)} aria-hidden="true" /><div className="item-context-menu" role="menu" style={{ left: contextItem.x, top: contextItem.y }}>
      <button role="menuitem" onClick={() => { onTogglePinned(contextItem.item); setContextItem(null); }}>{contextItem.item.isPinned ? '取消置顶' : '置顶'}</button>
    </div></>}
    {(!readOnly || entries.length > 0) && <div className="split-handle" role="separator" aria-label="调整正文与列表的空间" aria-orientation="horizontal" tabIndex={0}
      aria-valuemin={30} aria-valuemax={80} aria-valuenow={Math.round(heightSplit)}
      onPointerDown={startDividerDrag}
      onMouseDown={startDividerMouseDrag}
      onKeyDown={(event) => {
        if (event.key === 'ArrowUp') { event.preventDefault(); setHeightSplit((value) => Math.max(30, value - 3)); }
        if (event.key === 'ArrowDown') { event.preventDefault(); setHeightSplit((value) => Math.min(80, value + 3)); }
        if (event.key === 'ArrowLeft') { event.preventDefault(); setSplit((value) => Math.max(32, value - 2)); }
        if (event.key === 'ArrowRight') { event.preventDefault(); setSplit((value) => Math.min(58, value + 2)); }
      }} />}
    {readOnly && selectedSaved && <section className="editor-panel" aria-label={label + '只读详情'}>
      <div className="detail-top"><div className="detail-heading"><h2 className="readonly-title">{selectedSaved.title.trim() || '无标题' + label}</h2><div ref={setToolsTarget} className="detail-tools-target"/></div></div>
      {mode==='reminder' && view === '待办' && selectedReminder && <div className="reminder-current"><span>{localTime(selectedReminder.dueAt)}</span><button disabled={reminderBusy} onClick={async () => {setReminderBusy(true); setReminderError(''); try {if (!await onCompleteReminder(selectedReminder.occurrenceId)) setReminderError('完成提醒失败，请重试');} finally {setReminderBusy(false);}}}>标记本次完成</button></div>}
      {reminderError && <p role="alert" className="error-text">{reminderError}</p>}
      {toolsTarget&&<BodyViewToolsPortal target={toolsTarget} mode={mode} onMode={setMode}/>}
      <ReadOnlyBody key={selectedSaved.id + ':' + selectedSaved.revision} body={selectedSaved.body} bodyJson={selectedSaved.bodyJson} mode={mode} />
      {mode==='reminder'&&<div className="reminder-editor" role="region" aria-label="此条内容的提醒"><strong>提醒与完成记录</strong><CalendarPicker value={reminderInput} onChange={setReminderInput} reminderDates={reminders.map(r=>dateInputValue(r.dueAt).slice(0,10))}/><button disabled={reminderBusy} onClick={()=>void(async()=>{const d=new Date(reminderInput);if(Number.isNaN(d.getTime())){setReminderError('请选择有效时间');return;}setReminderBusy(true);try{if(await onSaveReminder(selectedSaved.id,d.toISOString()))reminderSaved();}finally{setReminderBusy(false);}})()}>保存提醒</button>{selectedReminder&&<button disabled={reminderBusy} onClick={()=>void onCancelReminder(selectedSaved.id)}>取消提醒</button>}<details className="reminder-history"><summary>已完成记录 · {reminders.filter(r=>r.itemId===selectedSaved.id&&r.status==='completed').length} 次</summary>{reminders.filter(r=>r.itemId===selectedSaved.id&&r.status==='completed').map(r=><p key={r.occurrenceId}>{localTime(r.completedAt??r.dueAt)}</p>)}</details></div>}
    </section>}
    {view !== '回收站' && !readOnly && <section className="editor-panel" aria-label={label + '编辑区'}>
      {draft && <>
        <div className="detail-top">
          <div className="detail-heading"><DetailTitle key={draft.id??editorSession} label={label} title={draft.title} onChange={onTitleChange}/><div ref={setToolsTarget} className="detail-tools-target" /></div>
          <div className="detail-meta">{draft.updatedAt && <time className="detail-time" title="上次成功保存时间">上次修改 {localTime(draft.updatedAt)}</time>}
</div>
        </div>
        {mode==='reminder' && <div className="reminder-editor" role="region" aria-label="此条内容的提醒"><strong>提醒与完成记录</strong>        {activeReminder && <div className="reminder-current"><span>{activeReminder.status === 'pending' ? (new Date(activeReminder.dueAt).getTime() < Date.now() ? '已逾期' : '待提醒') : '本次已完成'} · {localTime(activeReminder.status==='completed'?activeReminder.completedAt??activeReminder.dueAt:activeReminder.dueAt)}</span>{activeReminder.status === 'pending' && <button type="button" disabled={reminderBusy} onClick={async () => { setReminderBusy(true); if (!await onCompleteReminder(activeReminder.occurrenceId)) setReminderError('完成提醒失败，请重试'); setReminderBusy(false); }}>标记本次完成</button>}</div>}
        {completedHistory.length > 0 && <details className="reminder-history"><summary>已完成记录 · {completedHistory.length} 次</summary><ul>{completedHistory.map((entry) => <li key={entry.occurrenceId}>{localTime(entry.completedAt ?? entry.dueAt)}</li>)}</ul></details>}
<span>提醒时间</span><CalendarPicker value={reminderInput} onChange={setReminderInput} reminderDates={reminders.map(r=>dateInputValue(r.dueAt).slice(0,10))}/><div className="reminder-actions"><button type="button" disabled={reminderBusy} onClick={async () => {
          if (!draft.id) { setReminderError('请先按 Ctrl+S 保存内容'); return; }
          if (draft.title !== draft.savedTitle || draft.body !== draft.savedBody || draft.bodyJson !== draft.savedBodyJson) { setReminderError('请先按 Ctrl+S 保存正文修改'); return; }
          const date = new Date(reminderInput);
          if (!reminderInput || Number.isNaN(date.getTime())) { setReminderError('请选择有效的提醒时间'); return; }
          setReminderBusy(true); const okay = await onSaveReminder(draft.id, date.toISOString()); setReminderBusy(false);
          if (okay) { reminderSaved(); setReminderError(''); } else setReminderError('保存提醒失败，请重试');
        }}>保存提醒</button>{activeReminder && <button type="button" disabled={reminderBusy} onClick={async () => { if (!draft.id) return; setReminderBusy(true); const okay = await onCancelReminder(draft.id); setReminderBusy(false); if (okay) { reminderSaved(); setReminderError(''); } else setReminderError('取消提醒失败，请重试'); }}>取消提醒</button>}</div>{reminderError && <small role="alert" className="error-text">{reminderError}</small>}</div>}
        {toolsTarget&&<BodyViewToolsPortal target={toolsTarget} mode={mode} onMode={setMode}/>}
        <RichBodyEditor key={editorSession} body={draft.body} bodyJson={draft.bodyJson} onChange={onBodyChange} mode={mode} />
        {(saveError || status) && <div className={'detail-feedback ' + (saveError ? 'error-text' : '')} role="status">{saveError ? `保存失败：${saveError}` : status}</div>}
      </>}
    </section>}
  </div>;
}


function BodyViewToolsPortal({target,mode,onMode}:{target:HTMLElement;mode:BodyView;onMode:(mode:BodyView)=>void}){return createPortal(<BodyViewTools mode={mode} onMode={onMode}/>,target);}
