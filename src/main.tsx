import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { emitTo, listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import {firstReminderDraft,discardReminderDrafts} from './reminder_drafts';
import { ItemWorkspace } from './item_workspace';
import { NewsPanel } from './news_panel';
import { WindowSettings } from './window_settings';
import { FileSettings } from './file_settings';
import {DocumentImportDialog} from './document_import';
import { FreeApiPanel } from './free_api_panel';
import {AudioPanel,type AudioGuard} from './audio_panel';
import {QuickSearch} from './quick_search';
import {VoiceSettings,OnlineSettings} from './model_settings';
import {readDocument,documentText} from './rich_document';
import {useRunScroll,setRunValue} from './run_session';
import type { Draft, Item, ItemKind, ItemView, Reminder } from './item_workspace';
import './style.css';
import {CloudSettings,syncLabel,type CloudStatus} from './cloud_settings';
import {NavIcon} from './ui_icons';
import {applyBackgroundTransparency} from './background';
const brandIcon = new URL('./assets/qingjian-brand.svg', import.meta.url).href;

type Section = 'news' | 'stickies' | 'notes' | 'settings';
type ThemeId = 'mint_morning' | 'new_leaf' | 'warm_apricot' | 'coral' | 'lilac_mist';
type FontSize = 'small' | 'standard' | 'large';
type LeaveAction = {type:'configure';tab:'语音'|'模型'} | { type: 'transcript'; text:string; item?:Item } | { type: 'section'; section: Section } | { type: 'tab'; tab: string } | { type: 'item' | 'pin' | 'trash'; item: Item } | { type: 'new'; kind: ItemKind } | { type: 'hide' | 'quit' };
type DeleteConfirm = { type: 'trash' | 'forever'; item: Item };
type TaskInfo={kind:string;label:string;status:string;startedAt?:string;endedAt?:string;progress?:string;canCancel?:boolean};
type TaskSnapshot={modelNews:TaskInfo;textTest:TaskInfo;formalOnline:TaskInfo;audio:TaskInfo;recording:boolean};

const appWindow = getCurrentWindow();
const resizeDirections = [
  'North', 'South', 'West', 'East',
  'NorthWest', 'NorthEast', 'SouthWest', 'SouthEast',
] as const;

const sections: { id: Section; label: string }[] = [
  { id: 'news', label: '新闻' },
  { id: 'stickies', label: '便签' },
  { id: 'notes', label: '笔记' },
  { id: 'settings', label: '设置' },
];
const themes: { id: ThemeId; label: string; seed: string }[] = [
  { id: 'mint_morning', label: '薄荷晨露', seed: '#86E3CE' },
  { id: 'new_leaf', label: '青芽', seed: '#D0E6A5' },
  { id: 'warm_apricot', label: '暖杏', seed: '#FFDD94' },
  { id: 'coral', label: '珊瑚', seed: '#FA897B' },
  { id: 'lilac_mist', label: '丁香雾', seed: '#CCABD8' },
];
const subnav: Record<Section, string[]> = {
  news: ['AI 新闻', '模型资讯'],
  stickies: ['+ 新建', '待办', '已完成', '回收站'],
  notes: ['+ 新建', '待办', '已完成', '回收站'],
  settings: ['通用', '账号', '文件', '模型', '语音'],
};
const defaultSubnav: Record<Section, string> = {
  news: 'AI 新闻', stickies: '+ 新建', notes: '+ 新建', settings: '通用',
};

function initialTheme(): ThemeId {
  try {
    const stored = window.localStorage.getItem('qingjian.shell.theme');
    if (themes.some((item) => item.id === stored)) return stored as ThemeId;
  } catch {
    // 存储不可用时仍使用 PRD 确定的默认皮肤。
  }
  return 'warm_apricot';
}

function initialFontSize(): FontSize {
  try {
    const stored = window.localStorage.getItem('qingjian.shell.fontSize');
    if (stored === 'small' || stored === 'standard' || stored === 'large') return stored;
  } catch {
    // 本地设置不可用时仍可按标准字号启动。
  }
  return 'standard';
}

function SearchIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.8" cy="10.8" r="6.4" /><path d="m15.5 15.5 4.3 4.3" /></svg>;
}
function BellIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 8a6 6 0 0 0-12 0c0 7-2.3 7.9-2.3 9h16.6C20.3 15.9 18 15 18 8Z" /><path d="M10 20h4" /></svg>;
}
function SectionNotice({ title, detail }: { title: string; detail: string }) {
  return <div className="section-notice"><span className="notice-dot" /><strong>{title}</strong><span>{detail}</span></div>;
}
function EmptyState({ icon, title, detail }: { icon: string; title: string; detail: string }) {
  return <div className="empty-state"><div className="empty-icon">{icon}</div><strong>{title}</strong><p>{detail}</p></div>;
}

function App() {
  const [maximized,setMaximized]=useState(false),[sizeChanging,setSizeChanging]=useState(false);
  useEffect(()=>{
    let live=true;const read=()=>void invoke<{maximized:boolean}>('get_shell_diagnostics').then(s=>{if(live)setMaximized(s.maximized);}).catch(()=>{});
    // 图标依据真实原生窗口状态，包含系统快捷键、最小化恢复和跨屏尺寸变化。
    read();const listener=appWindow.onResized(read);const timer=window.setInterval(read,800);
    return()=>{live=false;clearInterval(timer);void listener.then(f=>f());};
  },[]);
  const [section, setSection] = useState<Section>('news');
  const [tabs,setTabs]=useState<Record<Section,string>>({...defaultSubnav});
  const tab=tabs[section];
  const setTab=(value:string)=>setTabs(previous=>({...previous,[section]:value}));
  const navigate=(next:Section,value?:string)=>{setSection(next);if(value)setTabs(previous=>({...previous,[next]:value}));};
  const [transparency,setTransparency]=useState(0);
  const [topmost,setTopmost]=useState(false);
  useEffect(()=>{let live=true;const read=()=>void invoke<{alwaysOnTop:boolean}>('get_shell_settings').then(s=>{if(live)setTopmost(s.alwaysOnTop);}).catch(()=>{});read();const timer=setInterval(read,500);return()=>{live=false;clearInterval(timer);};},[]);
  useEffect(()=>{let live=true;let timer:number;let attempts=0;const read=()=>void invoke<{transparency:number}>('get_shell_settings').then(s=>{if(live)setTransparency(s.transparency||0);}).catch(()=>{if(live&&attempts++<20)timer=window.setTimeout(read,100);});read();const listener=listen<number>('background-transparency',e=>setTransparency(e.payload));return()=>{live=false;clearTimeout(timer);void listener.then(f=>f());};},[]);
  const [theme, setTheme] = useState<ThemeId>(initialTheme);
  const [fontSize, setFontSize] = useState<FontSize>(initialFontSize);
  useEffect(()=>{let live=true,attempts=0;const save=()=>void invoke('backup_preferences',{preferences:{theme,fontSize}}).catch(()=>{if(live&&attempts++<20)setTimeout(save,150);});save();return()=>{live=false;};},[theme,fontSize]);
  useEffect(()=>applyBackgroundTransparency(transparency),[theme,transparency]);
  const [items, setItems] = useState<Record<ItemKind, Item[]>>({ sticky: [], note: [] });
  const [trashed, setTrashed] = useState<Record<ItemKind, Item[]>>({ sticky: [], note: [] });
  const [reminders, setReminders] = useState<Reminder[]>([]);
  const [bellOpen, setBellOpen] = useState(false);
  const [audioOpen,setAudioOpen]=useState(false);
  const [recording,setRecording]=useState(false),[recordLeave,setRecordLeave]=useState<LeaveAction|null>(null);
  const [loading, setLoading] = useState(true);
  const [dataError, setDataError] = useState('');
  const [saveError, setSaveError] = useState('');
  const [actionError, setActionError] = useState('');
  const [tasks,setTasks]=useState<TaskSnapshot|null>(null);
  const [accountBusy,setAccountBusy]=useState(false);
  useEffect(()=>{const update=(e:Event)=>setAccountBusy((e as CustomEvent<boolean>).detail);window.addEventListener('cloud-transition-busy',update);return()=>window.removeEventListener('cloud-transition-busy',update);},[]);
  const [cloudStatus,setCloudStatus]=useState<CloudStatus|null>(null);
  const refreshCloud=async()=>{setCloudStatus(await invoke<CloudStatus>('cloud_status'));};
  useEffect(()=>{void refreshCloud().catch(()=>{});const timer=setInterval(()=>void refreshCloud().catch(()=>{}),1500);return()=>clearInterval(timer);},[]);
  const [status, setStatus] = useState('');
  const [saving, setSaving] = useState(false);
  const [importKind,setImportKind]=useState<ItemKind|null>(null),[importSaving,setImportSaving]=useState(false);
  useEffect(()=>{const busy=(e:Event)=>setImportSaving((e as CustomEvent<boolean>).detail);const appearance=(e:Event)=>{const p=(e as CustomEvent).detail;setTheme(p.theme);setFontSize(p.fontSize);};window.addEventListener('backup-saving',busy);window.addEventListener('backup-appearance',appearance);return()=>{window.removeEventListener('backup-saving',busy);window.removeEventListener('backup-appearance',appearance);};},[]);
  const [drafts,setDrafts]=useState<Record<ItemKind,Draft|null>>({sticky:null,note:null});
  const kind=section==='stickies'?'sticky':section==='notes'?'note':null;
  const draft=kind?drafts[kind]:null;
  const setDraft:React.Dispatch<React.SetStateAction<Draft|null>>=next=>setDrafts(previous=>{
    const value=typeof next==='function'?next(kind?previous[kind]:null):next;
    return value?{...previous,[value.kind]:value}:kind?{...previous,[kind]:null}:previous;
  });
  const [editorSession, setEditorSession] = useState(0);
  const [pending, setPending] = useState<LeaveAction | null>(null);
  const audioGuard=useRef<AudioGuard|null>(null);
  const [audioPending,setAudioPending]=useState<LeaveAction|null>(null);
  const [audioSaving,setAudioSaving]=useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<DeleteConfirm | null>(null);
  const actionRef = useRef<(action: LeaveAction) => void>(() => {});
  const saveRef = useRef<(after?: LeaveAction) => Promise<void>>(async () => {});
  const contentRef=useRef<HTMLDivElement>(null);
  const capturePosition=useRunScroll(contentRef,section+'/'+tab);
  const dirtyDraft=(d:Draft|null)=>!!d&&(d.title!==d.savedTitle||d.body!==d.savedBody||d.bodyJson!==d.savedBodyJson);
  useEffect(()=>{let live=true;const poll=()=>void invoke<TaskSnapshot>('task_status').then(next=>{if(!live)return;setTasks(next);if(!next.recording&&next.audio.status!=='running'&&next.formalOnline.status!=='running')setActionError(old=>old.includes('正在进行')? '':old);}).catch(()=>{});poll();const timer=window.setInterval(poll,1000);return()=>{live=false;window.clearInterval(timer);};},[]);

  useEffect(() => {
    // 五套主题只改变语义色；暖杏是首次启动默认项。
    document.documentElement.dataset.theme = theme;
    try { window.localStorage.setItem('qingjian.shell.theme', theme); } catch { /* 预览仍可用 */ }
    void emitTo('quick', 'shell-appearance', { theme, fontSize });
  }, [theme]);

  useEffect(() => {
    // 字号档位即时作用于导航、内容与控件；在接入 SQLite 前保存在本机。
    document.documentElement.dataset.fontSize = fontSize;
    try { window.localStorage.setItem('qingjian.shell.fontSize', fontSize); } catch { /* 当前会话仍可用 */ }
    void emitTo('quick', 'shell-appearance', { theme, fontSize });
  }, [fontSize]);

  const reloadSerial=useRef(0);
  const reload = async () => {
    const serial=++reloadSerial.current;
    setLoading(true); setDataError('');
    try {
      const [sticky, note, stickyTrash, noteTrash, allReminders] = await Promise.all([
        invoke<Item[]>('list_items', { kind: 'sticky' }), invoke<Item[]>('list_items', { kind: 'note' }),
        invoke<Item[]>('list_trashed', { kind: 'sticky' }), invoke<Item[]>('list_trashed', { kind: 'note' }),
        invoke<Reminder[]>('list_reminders'),
      ]);
      if(serial!==reloadSerial.current)return;
      setItems({ sticky, note });
      setTrashed({ sticky: stickyTrash, note: noteTrash });
      setReminders(allReminders);
      void refreshCloud().catch(()=>{});
    } catch (error) { if(serial===reloadSerial.current)setDataError(String(error)); }
    finally { if(serial===reloadSerial.current)setLoading(false); }
  };
  useEffect(() => { void reload(); const p=listen('content-collected',()=>void reload()); const sync=listen('cloud-content-changed',()=>{void reload();void refreshCloud();}); return ()=>{void p.then(f=>f());void sync.then(f=>f());}; }, []);
  useEffect(()=>{
    if(loading)return;
    // 删除或导入后安全回退；检查只随实际列表更新，不因编辑草稿自动提交或保存。
    setDrafts(previous=>{let changed=false;const next={...previous};for(const k of ['sticky','note'] as const){const d=previous[k];if(!d?.id||dirtyDraft(d))continue;const item=items[k].find(i=>i.id===d.id);if(!item){next[k]=null;changed=true;}else if(item.revision!==d.revision){next[k]={remoteGeneration:(d.remoteGeneration||0)+1,id:item.id,kind:item.kind,title:item.title,body:item.body,bodyJson:item.bodyJson,updatedAt:item.updatedAt,revision:item.revision,savedTitle:item.title,savedBody:item.body,savedBodyJson:item.bodyJson};changed=true;}}return changed?next:previous;});
  },[items,loading]);
  useEffect(() => {
    if (!status) return;
    const timer = window.setTimeout(() => setStatus(''), 1800);
    return () => window.clearTimeout(timer);
  }, [status]);

  const [reminderLeave,setReminderLeave]=useState(false);
  const doAction = async (action: LeaveAction) => {
    if(action.type==='quit'&&firstReminderDraft()){setReminderLeave(true);return;}
    setSaveError(''); setStatus(''); setActionError('');
    setBellOpen(false);
    if(action.type==='transcript'){
      // 追加前重新读取目标，保留其富文本；这里只生成草稿，仍由 Ctrl+S 落库。
      try{const item=action.item?await invoke<Item>('get_item',{id:action.item.id}):undefined;const doc=readDocument(item?.body||'',item?.bodyJson);const extra=readDocument(action.text).document.content||[];doc.document.content=[...(item?doc.document.content||[]:[]),...extra];navigate('notes','+ 新建');setDraft({id:item?.id,kind:'note',title:item?.title||'会议记录',body:documentText(doc.document),bodyJson:JSON.stringify(doc),revision:item?.revision,updatedAt:item?.updatedAt,savedTitle:item?.title||'',savedBody:item?.body||'',savedBodyJson:item?.bodyJson||null});setEditorSession(n=>n+1);}catch(e){setActionError(String(e));}return;
    }
    if (action.type === 'hide' || action.type === 'quit') {
      try { await invoke('finish_leave', { action: action.type }); }
      catch (error) { setActionError(String(error)); }
      return;
    }
    if(action.type==='configure'){navigate('settings',action.tab);return;}
    if (action.type === 'section') {
      navigate(action.section);
      // 栏目首次进入沿用默认页；随后保留独立草稿，切页不自动保存或丢弃。
      if(action.section==='stickies'||action.section==='notes'){const k=action.section==='stickies'?'sticky':'note';if(!drafts[k])setDraft({kind:k,title:'',body:'',bodyJson:null,savedTitle:'',savedBody:'',savedBodyJson:null});}
      return;
    }
    if (action.type === 'tab') {
      setTab(action.tab);
      return;
    }
    if (action.type === 'trash') { setDeleteConfirm({ type: 'trash', item: action.item }); return; }
    if (action.type === 'item' || action.type === 'pin') {
      const item = action.item;
      if (action.type === 'pin') { const next = item.kind === 'sticky' ? 'stickies' : 'notes'; navigate(next,defaultSubnav[next]); }
      setEditorSession((value) => value + 1);
      setDraft({ id: item.id, kind: item.kind, title: item.title, body: item.body, bodyJson: item.bodyJson, updatedAt: item.updatedAt, revision: item.revision, savedTitle: item.title, savedBody: item.body, savedBodyJson: item.bodyJson });
      return;
    }
    if (action.type === 'new') { setTab('+ 新建'); setEditorSession((value) => value + 1); setDraft({ kind: action.kind, title: '', body: '', bodyJson: null, savedTitle: '', savedBody: '', savedBodyJson: null }); return; }
  };
  const isDirty = dirtyDraft(draft);
  const anyDirty=Object.values(drafts).some(dirtyDraft);
  useEffect(() => {
    // 编辑焦点、草稿与对话框保护贴边隐藏；鼠标拖动由 Windows 按键状态另行保护。
    const update = () => { const focus=document.activeElement; const editing=focus instanceof HTMLElement&&(focus.isContentEditable||focus.matches('textarea,input:not([type="checkbox"]):not([type="radio"]),select')); const modal=!!document.querySelector('[role="dialog"],[data-native-dialog-busy="true"]'); void invoke('set_shell_busy',{busy:anyDirty||!!firstReminderDraft()||!!pending||!!deleteConfirm||bellOpen||saving||editing||modal}).catch(()=>{}); };
    update();const timer=window.setInterval(update,250);return()=>window.clearInterval(timer);
  },[anyDirty,pending,deleteConfirm,bellOpen,saving]);
  const requestAction = (action: LeaveAction) => {
    if (accountBusy || cloudStatus?.restartRequired || saving || importSaving || pending || audioPending) return;
    if(recording&&(action.type==='hide'||action.type==='quit')){setRecordLeave(action);return;}
    if (action.type === 'section' && action.section === section) return;
    if (action.type === 'tab' && action.tab === tab) return;
    if ((action.type === 'item' || action.type === 'pin') && action.item.id === draft?.id && section === (action.item.kind === 'sticky' ? 'stickies' : 'notes')) return;
    capturePosition();
    if(action.type==='section'||action.type==='tab'||action.type==='configure'){void doAction(action);return;}
    // 隐藏栏目中的未保存草稿仍受退出保护；逐条处理后才允许真正退出。
    if((action.type==='hide'||action.type==='quit')&&anyDirty){const d=draft&&isDirty?draft:Object.values(drafts).find(dirtyDraft);if(d){navigate(d.kind==='sticky'?'stickies':'notes','+ 新建');setPending(action);return;}}
    const replacing=action.type==='new'||action.type==='pin'||action.type==='item'||action.type==='transcript';
    const targetKind=action.type==='new'?action.kind:action.type==='pin'||action.type==='item'?action.item.kind:action.type==='transcript'?'note':kind;
    if((action.type==='pin'||action.type==='item')&&targetKind&&drafts[targetKind]?.id===action.item.id){navigate(targetKind==='sticky'?'stickies':'notes','+ 新建');return;}
    if(replacing&&targetKind&&dirtyDraft(drafts[targetKind])){navigate(targetKind==='sticky'?'stickies':'notes','+ 新建');setPending(action);return;}
    if (isDirty) { setPending(action); return; }
    if (audioGuard.current?.dirty()) { setAudioPending(action); return; }
    void doAction(action);
  };
  actionRef.current = requestAction;
  useEffect(() => {
    let active = true;
    const unlisteners: (() => void)[] = [];
    void listen<string>('shell-leave-request', (event) => {
      if (event.payload === 'hide' || event.payload === 'quit') actionRef.current({ type: event.payload });
    }).then((fn) => { if (active) unlisteners.push(fn); else fn(); });
    void listen<{ itemId: string }>('quick-open-item', async (event) => {
      try { const item = await invoke<Item>('get_item', { id: event.payload.itemId }); actionRef.current({ type: 'pin', item }); }
      catch (error) { setDataError(String(error)); }
    }).then((fn) => { if (active) unlisteners.push(fn); else fn(); });
    return () => { active = false; unlisteners.forEach((fn) => fn()); };
  }, []);
  const save = async (after?: LeaveAction) => {
    if (!draft || saving || accountBusy || cloudStatus?.restartRequired) return;
    setSaving(true); setSaveError(''); setStatus('');
    try {
      // 只有这一处调用数据库写入；编辑输入始终先停留在内存草稿。
      const item = await invoke<Item>('save_item', { input: { id: draft.id ?? null, kind: draft.kind, title: draft.title, body: draft.body, bodyJson: draft.bodyJson, revision: draft.revision ?? null } });
      setDraft({ remoteGeneration:draft.remoteGeneration, id: item.id, kind: item.kind, title: item.title, body: item.body, bodyJson: item.bodyJson, updatedAt: item.updatedAt, revision: item.revision, savedTitle: item.title, savedBody: item.body, savedBodyJson: item.bodyJson });
      await reload();
      setStatus('已保存到本机'); void refreshCloud();
      void emitTo('quick', 'quick-items-changed', true);
      if (after) {
        setPending(null);
        if (after.type === 'trash') setDeleteConfirm({ type: 'trash', item });
        else if(after.type==='quit'||after.type==='hide'){const remaining=Object.values(drafts).find(d=>d?.kind!==draft.kind&&dirtyDraft(d));if(remaining){navigate(remaining.kind==='sticky'?'stickies':'notes','+ 新建');setPending(after);}else if(audioGuard.current?.dirty())setAudioPending(after);else await doAction(after);}else await doAction(after);
      }
    } catch (error) { setSaveError(String(error)); }
    finally { setSaving(false); }
  };
  saveRef.current = save;
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (!audioOpen && !event.defaultPrevented && draft && event.ctrlKey && !event.altKey && event.key.toLowerCase() === 's') {
        event.preventDefault();
        void saveRef.current(pending ?? undefined);
      }
    };
    document.addEventListener('keydown', keydown);
    return () => document.removeEventListener('keydown', keydown);
  }, [pending, !!draft,audioOpen]);
  const resolveDiscard = () => {
    const next = pending; setPending(null);
    // 放弃草稿时还原已保存快照；托盘恢复不能重新出现被放弃的文字。
    setDraft((current) => current?.id ? { ...current, title: current.savedTitle, body: current.savedBody, bodyJson: current.savedBodyJson } : null);
    setEditorSession((value) => value + 1);
    if(next&&(next.type==='quit'||next.type==='hide')){const remaining=Object.values(drafts).find(d=>d?.kind!==draft?.kind&&dirtyDraft(d));if(remaining){navigate(remaining.kind==='sticky'?'stickies':'notes','+ 新建');setPending(next);return;}}
    if(next&&(next.type==='quit'||next.type==='hide')&&audioGuard.current?.dirty()){setAudioPending(next);return;}
    if (next) void doAction(next);
  };
  const confirmDelete = async () => {
    if (!deleteConfirm) return;
    setActionError('');
    try {
      if (deleteConfirm.type === 'trash') await invoke('trash_item', { id: deleteConfirm.item.id, revision: deleteConfirm.item.revision });
      else await invoke('delete_item_forever', { id: deleteConfirm.item.id });
      if (draft?.id === deleteConfirm.item.id) {
        setDraft({ kind: draft.kind, title: '', body: '', bodyJson: null, savedTitle: '', savedBody: '', savedBodyJson: null });
        setEditorSession((value) => value + 1);
      }
      setDeleteConfirm(null);
      await reload();
      void emitTo('quick', 'quick-items-changed', true);
    } catch (reason) { setActionError(String(reason)); }
  };
  const restore = async (item: Item) => {
    setActionError('');
    try { await invoke('restore_item', { id: item.id }); await reload(); void emitTo('quick', 'quick-items-changed', true); }
    catch (reason) { setActionError('恢复失败：' + String(reason)); }
  };
  const togglePinned = async (item: Item) => {
    setActionError('');
    try { await invoke('set_item_pinned', { id: item.id, pinned: !item.isPinned }); await reload(); }
    catch (reason) { setActionError('调整置顶失败：' + String(reason)); }
  };
  const moveItem = async (item: Item, target: Item) => {
    setActionError('');
    try { await invoke('move_item', { id: item.id, targetId: target.id }); await reload(); }
    catch (reason) { setActionError('调整顺序失败：' + String(reason)); }
  };
  const saveReminder = async (itemId: string, dueAt: string): Promise<boolean> => {
    setActionError('');
    try { await invoke('save_reminder', { itemId, dueAt }); await reload(); setStatus('提醒已保存'); return true; }
    catch (reason) { setActionError('保存提醒失败：' + String(reason)); return false; }
  };
  const cancelReminder = async (itemId: string): Promise<boolean> => {
    setActionError('');
    try { await invoke('cancel_reminder', { itemId }); await reload(); setStatus('提醒已取消'); return true; }
    catch (reason) { setActionError('取消提醒失败：' + String(reason)); return false; }
  };
  const completeReminder = async (occurrenceId: string): Promise<boolean> => {
    setActionError('');
    try { await invoke('complete_reminder', { occurrenceId }); await reload(); setStatus('本次提醒已完成'); return true; }
    catch (reason) { setActionError('完成提醒失败：' + String(reason)); return false; }
  };
  const openReminder = async (reminder: Reminder) => {
    try { const item = await invoke<Item>('get_item', { id: reminder.itemId }); requestAction({ type: 'pin', item }); }
    catch (reason) { setActionError('打开提醒来源失败：' + String(reason)); }
  };
  const showItemWorkspace = section === 'stickies' || section === 'notes';
  const sectionLabel = sections.find((item) => item.id === section)?.label ?? '';

  return <main className="app-shell">
    <div className="resize-zones" aria-hidden="true">
      {resizeDirections.map((direction) => <div key={direction} className={'resize-zone ' + direction.toLowerCase()} onMouseDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        // 自定义外框的八个热区分别交给 Windows 原生窗口缩放处理。
        void appWindow.startResizeDragging(direction);
      }} />)}
    </div>

    <header className="titlebar">
      <div className="brand-drag" onMouseDown={(event) => { if (event.button === 0&&!maximized){event.preventDefault();void invoke('begin_grip_drag').catch(e=>setActionError(String(e)));} }}>
        {/* 品牌使用完整矢量图，固定渐变，不叠加主题底框；禁用图片拖动以保留窗口拖动。 */}
        <img className="brand-icon" src={brandIcon} alt="" draggable={false} /><span className="brand-name">轻笺</span>
      </div>
      <div className="title-actions">
        {<button className={'icon-button quick-record-button '+(recording?'recording-active':'')} title={recording?'查看当前录音':'录音与转写'} aria-label="录音与转写" aria-pressed={recording} onClick={()=>{setAudioOpen(true);}}><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3M8 22h8"/></svg></button>}
        <QuickSearch items={[...items.sticky,...items.note]} onOpen={item=>requestAction({type:'pin',item})}/>
        <button className={'icon-button window-pin '+(topmost?'active':'')} title={topmost?'取消置顶':'置顶'} aria-label={topmost?'取消置顶':'置顶'} aria-pressed={topmost} onClick={()=>void invoke<{alwaysOnTop:boolean}>('set_shell_setting',{key:'alwaysOnTop',value:!topmost}).then(s=>setTopmost(s.alwaysOnTop)).catch(e=>setActionError(String(e)))}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 3h8l-1 7 4 4H5l4-4-1-7ZM12 14v7"/></svg></button>
        {reminders.some(entry=>entry.status==='pending'&&entry.isCurrent)&&<button className="icon-button" title="提醒中心" aria-label="提醒中心" aria-expanded={bellOpen} onClick={() => setBellOpen((value) => !value)}><BellIcon /></button>}
        <span className="title-separator" />
        <button className="icon-button system-button" title="最小化" aria-label="最小化" onClick={() => void appWindow.minimize()}>−</button>
        <button className="icon-button system-button" title={maximized?'还原':'最大化'} aria-label={maximized?'还原':'最大化'} disabled={sizeChanging} onClick={()=>{setSizeChanging(true);void invoke<boolean>('toggle_shell_maximize').then(setMaximized).catch(e=>setActionError(String(e))).finally(()=>setSizeChanging(false));}}>{maximized?<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 8V4h12v12h-4"/><rect x="4" y="8" width="12" height="12"/></svg>:<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="4" width="16" height="16"/></svg>}</button>
        <button className="icon-button system-button" title="关闭到托盘" aria-label="关闭到托盘" onClick={() => requestAction({ type: 'hide' })}>×</button>
      </div>
    </header>
    {actionError&&<div className="operation-error" role="alert">{actionError}<button onClick={()=>setActionError('')}>关闭提示</button></div>}
    {tasks&&[tasks.textTest,tasks.formalOnline,tasks.audio].filter(t=>t.status==='running').map(t=><div className="operation-error" role="status" key={t.kind}>{t.label}正在进行{t.startedAt?` · 开始于 ${new Date(t.startedAt).toLocaleTimeString('zh-CN')}`:''}{t.progress?` · ${t.progress}`:''}{t.canCancel&&<button onClick={()=>void invoke(t.kind==='model_news_refresh'?'cancel_model_news':'cancel_online_test',t.kind==='model_news_refresh'?{}:{kind:'text'}).then(()=>invoke<TaskSnapshot>('task_status')).then(setTasks)}>取消</button>}</div>)}
    {bellOpen && <div className="bell-panel" role="dialog" aria-label="提醒中心">
      <div className="bell-heading"><strong>提醒中心</strong><button type="button" onClick={() => setBellOpen(false)} aria-label="关闭提醒中心">×</button></div>
      {reminders.filter((entry) => entry.status === 'pending').length === 0 ? <p className="bell-empty">暂无待处理提醒</p> :
        reminders.filter((entry) => entry.status === 'pending').map((entry) => <div className="bell-row" key={entry.occurrenceId}>
          <button type="button" onClick={() => void openReminder(entry)}><strong>{entry.title.trim() || (entry.kind === 'sticky' ? '无标题便签' : '无标题笔记')}</strong><small>{new Date(entry.dueAt).toLocaleString('zh-CN')} · {new Date(entry.dueAt).getTime() < Date.now() ? '已逾期' : '待提醒'}</small></button>
          <button type="button" className="bell-complete" title="标记本次完成" onClick={() => void completeReminder(entry.occurrenceId)}>完成</button>
        </div>)}
      <p className="bell-footnote">当前可设置单次提醒；系统通知与周期规则待接入。</p>
    </div>}

    {accountBusy&&<div className="dialog-backdrop cloud-restart-block"><section className="leave-dialog" role="dialog" aria-label="正在变更会话"><p>正在变更会话，请稍候…</p></section></div>}
    {cloudStatus?.restartRequired&&<div className="dialog-backdrop cloud-restart-block"><section className="leave-dialog" role="dialog" aria-label="账号切换完成"><h2>请重启轻笺</h2><p>账号已变更，旧工作区同步已停止。重启后进入所选工作区。</p><button onClick={()=>void invoke('cloud_restart').catch(e=>setActionError(String(e)))}>重启轻笺</button>{actionError&&<p role="alert">{actionError}</p>}</section></div>}
    <nav className="primary-nav" aria-label="主导航">
      {sections.map((item) => <button key={item.id} title={item.label} aria-label={item.label} className={section === item.id ? 'active' : ''} onClick={() => requestAction({ type: 'section', section: item.id })}>{item.label}</button>)}
    </nav>
    <nav className={'secondary-nav ' + (section === 'settings' ? 'settings-tabs' : section === 'news' ? 'news-tabs' : '')} aria-label={sectionLabel + '子导航'}>
      {subnav[section].map((item) => {
        const isItem = section === 'stickies' || section === 'notes';
        const disabled = false;
        return <button key={item} className={tab === item ? 'active' : ''} disabled={disabled} title={item==='+ 新建'?'新建':item==='已完成'?'已办':item} aria-label={item==='+ 新建'?'新建':item==='已完成'?'已办':item} onClick={() => item === '+ 新建' && isItem ? requestAction({ type: 'new', kind: section === 'stickies' ? 'sticky' : 'note' }) : requestAction({ type: 'tab', tab: item })}>{item==='+ 新建'?<NavIcon name="new"/>:item==='回收站'?<NavIcon name="trash"/>:item==='已完成'?'已办':item}</button>;
      })}
    </nav>
    <div ref={contentRef} className={'content-stage '+(section==='news'?'news-content-stage':audioOpen?'recording-content-stage':'')}>
      {section === 'news' && <section className="news-stage">
        {tab==='AI 新闻'?<NewsPanel onView={id=>void invoke<Item>('get_item',{id}).then(item=>requestAction({type:'item',item}))}/>:<FreeApiPanel onView={id=>void invoke<Item>('get_item',{id}).then(item=>requestAction({type:'item',item}))}/>}
      </section>}
      {showItemWorkspace && <div className="items-stage">
        <div className="cloud-inline-status" role="status"><span>{syncLabel(cloudStatus)}</span><button onClick={()=>navigate('settings','账号')}>账号</button></div>
        {draft?.id&&isDirty&&!items[draft.kind].some(i=>i.id===draft.id&&i.revision===draft.revision)&&<div className="operation-error" role="alert">云端内容已变化，未保存草稿仍保留。<button onClick={()=>{setDraft(current=>current?{...current,id:undefined,revision:undefined,title:current.title+'（草稿副本）',savedTitle:'',savedBody:'',savedBodyJson:null}:null);setSaveError('');}}>另存草稿副本</button></div>}

        {actionError && <div className="operation-error" role="alert">{actionError}</div>}
        <ItemWorkspace key={section} kind={section === 'stickies' ? 'sticky' : 'note'} view={tab as ItemView} items={items[section === 'stickies' ? 'sticky' : 'note']} trashed={trashed[section === 'stickies' ? 'sticky' : 'note']} reminders={reminders} draft={draft?.kind === (section === 'stickies' ? 'sticky' : 'note') ? draft : null} editorSession={editorSession} loading={loading} error={dataError} saveError={saveError} status={status} onSelect={(item) => requestAction({ type: 'item', item })}
          onTitleChange={(title) => { setDraft((current) => current ? { ...current, title } : null); setSaveError(''); setStatus(''); }}
          onBodyChange={(body, bodyJson) => { setDraft((current) => current ? { ...current, body, bodyJson } : null); setSaveError(''); setStatus(''); }}
          onDelete={(item) => requestAction({ type: 'trash', item })} onRestore={(item) => void restore(item)} onPermanent={(item) => { setActionError(''); setDeleteConfirm({ type: 'forever', item }); }} onReload={() => void reload()}
          onImport={()=>{capturePosition();setImportKind(section==='stickies'?'sticky':'note');}} onExport={async (ids,format,includeSettings)=>{setActionError('');try{const directory=await invoke<string|null>('choose_export_folder');if(!directory)return false;await invoke('export_documents',{directory,format,preferences:{theme,fontSize},selectedIds:ids,includeSettings});setStatus(`已导出 ${ids.length} 条${section==='stickies'?'便签':'笔记'}`);return true;}catch(e){setActionError('导出失败：'+String(e));return false;}}} onTogglePinned={(item) => void togglePinned(item)} onMove={(item, target) => void moveItem(item, target)} onSaveReminder={saveReminder} onCancelReminder={cancelReminder} onCompleteReminder={completeReminder} />
      </div>}
      <div className="recording-overlay" hidden={!audioOpen} role={audioOpen?'dialog':undefined} aria-label="录音工作区"><div className="recording-overlay-heading"><span>录音与转写</span><button onClick={()=>setAudioOpen(false)}>收起</button></div><AudioPanel guardRef={audioGuard} open={audioOpen} onActive={setRecording} notes={items.note} onConfigure={next=>{setAudioOpen(false);requestAction({type:'configure',tab:next});}} onDraft={(text,item)=>{setAudioOpen(false);requestAction({type:'transcript',text,item});}}/></div>
      {section === 'settings' && <section className="settings-stage">
        {actionError&&<p className="operation-error" role="alert">{actionError}</p>}
        {tab === '账号' ? <CloudSettings status={cloudStatus} onRefresh={refreshCloud} canChange={()=>anyDirty||!!firstReminderDraft()||audioGuard.current?.dirty()?'请先保存或处理所有未保存的正文、提醒和转写草稿。':saving||importSaving||recording||tasks?.audio.status==='running'||tasks?.formalOnline.status==='running'?'请先结束保存、导入、录音或处理任务。':''} onOpen={id=>void invoke<Item>('get_item',{id}).then(item=>requestAction({type:'pin',item})).catch(e=>setActionError(String(e)))}/> : tab === '通用' ? <>
          <div className="settings-heading"><span className="settings-kicker">外观</span><h1>选择轻笺的色彩</h1><p>五套皮肤来自 PRD 指定色卡，首次启动使用暖杏。</p></div>
          <div className="theme-list" aria-label="皮肤选择">
            {themes.map((option) => <button key={option.id} className={'theme-option ' + (theme === option.id ? 'selected' : '')} onClick={() => setTheme(option.id)}>
              <span className="theme-swatch" style={{ backgroundColor: option.seed }} /><span className="theme-label">{option.label}</span><span className="theme-code">{option.seed}</span><span className="theme-check">{theme === option.id ? '✓' : ''}</span>
            </button>)}
          </div>
          <div className="font-setting"><div className="setting-label"><strong>界面字号</strong><span>切换后即时生效</span></div><div className="font-options" role="group" aria-label="界面字号">
            {([['small', '小'], ['standard', '标准'], ['large', '大']] as const).map(([value, label]) => <button key={value} className={fontSize === value ? 'active' : ''} aria-pressed={fontSize === value} onClick={() => setFontSize(value)}>{label}</button>)}
          </div></div>
          <WindowSettings />
        </> : tab==='文件'?<FileSettings preferences={{theme,fontSize}} onImported={settings=>{if(settings){setTheme(settings.theme as ThemeId);setFontSize(settings.fontSize as FontSize);}setDraft(null);setEditorSession(n=>n+1);void reload();}}/>:tab==='语音'?<VoiceSettings/>:tab==='模型'?<OnlineSettings/>:<EmptyState icon="⚙" title={tab + '设置尚未接入'} detail="通用设置可调整窗口与外观；文件页可备份恢复。" />}
      </section>}
    </div>

    {importKind&&<DocumentImportDialog defaultKind={importKind} preferences={{theme,fontSize}} onClose={()=>setImportKind(null)} onSaving={setImportSaving} onImported={reload} onView={id=>{
      // 打开结果仍走既有草稿保护，不因导入而替换或自动保存当前编辑内容。
      setImportKind(null);void invoke<Item>('get_item',{id}).then(item=>requestAction({type:'pin',item})).catch(e=>setActionError(String(e)));
    }}/ >}

    {/* 高窗口从底部握柄上拖，中心也能进入顶部区域；不改变中心判定规则。 */}
    <button className="window-drag-grip" title="拖动窗口；贴边松手 0.5 秒后隐藏" aria-label="拖动窗口" disabled={maximized} onMouseDown={event=>{if(event.button===0&&!maximized){event.preventDefault();void invoke("begin_grip_drag");}}}><span /></button>

    {reminderLeave&&<div className="dialog-backdrop"><section className="leave-dialog" role="dialog" aria-modal="true" aria-label="未保存的提醒修改"><h2>有未保存的提醒修改</h2><p>提醒草稿在本次运行中保留，完全退出后将丢失。</p><div className="dialog-actions"><button onClick={()=>{setReminderLeave(false);const owner=firstReminderDraft();if(owner){setRunValue(owner.kind+'.+ 新建.mode.'+(owner.id??'new'),'reminder');navigate(owner.kind==='sticky'?'stickies':'notes','+ 新建');if(owner.id){const item=items[owner.kind].find(i=>i.id===owner.id);if(item)void doAction({type:'item',item});}}}}>继续编辑</button><button onClick={()=>{discardReminderDrafts();setReminderLeave(false);void doAction({type:'quit'});}}>放弃提醒修改并退出</button><button onClick={()=>setReminderLeave(false)}>取消</button></div></section></div>}
    {recordLeave&&<div className="dialog-backdrop"><section className="leave-dialog" role="dialog" aria-label="正在录音"><h2>录音仍在进行</h2><p>停止后音频会临时保留，可稍后转写；也可继续录音。</p>{saveError&&<p role="alert">{saveError}</p>}<div className="dialog-actions"><button onClick={()=>void(async()=>{try{await invoke('stop_recording');setRecording(false);const a=recordLeave;setRecordLeave(null);if(anyDirty){const d=Object.values(drafts).find(dirtyDraft);if(d)navigate(d.kind==='sticky'?'stickies':'notes','+ 新建');setPending(a);}else void doAction(a);}catch(e){setSaveError(String(e));}})()}>停止并暂存音频后继续</button>{recordLeave.type==='hide'&&<button onClick={()=>{const a=recordLeave;setRecordLeave(null);if(anyDirty){const d=Object.values(drafts).find(dirtyDraft);if(d)navigate(d.kind==='sticky'?'stickies':'notes','+ 新建');setPending(a);}else void doAction(a);}}>继续录音并进入托盘</button>}<button onClick={()=>setRecordLeave(null)}>取消</button></div></section></div>}
    {audioPending&&<div className="dialog-backdrop"><section className="leave-dialog" role="dialog" aria-modal="true" aria-label="未保存的转写校对"><h2>有未保存的校对</h2><p>保存当前转写或纪要的修改？</p><div className="dialog-actions"><button disabled={audioSaving} onClick={()=>void(async()=>{setAudioSaving(true);try{if(await audioGuard.current?.save()){const next=audioPending;setAudioPending(null);await doAction(next);}}finally{setAudioSaving(false);}})()}>{audioSaving?'保存中…':'保存'}</button><button disabled={audioSaving} onClick={()=>{audioGuard.current?.discard();const next=audioPending;setAudioPending(null);void doAction(next);}}>放弃</button><button disabled={audioSaving} onClick={()=>setAudioPending(null)}>继续编辑</button></div></section></div>}
    {pending && <div className="dialog-backdrop"><div className="leave-dialog" role="dialog" aria-modal="true" aria-labelledby="leave-title">
      <h2 id="leave-title">有未保存的内容</h2><p>要保存这次{draft?.kind === 'note' ? '笔记' : '便签'}修改吗？</p>
      {saveError && <p className="error-text" role="alert">保存失败：{saveError}。草稿仍在编辑区。</p>}
      <div className="dialog-actions"><button className="save-button" disabled={saving} onClick={() => void save(pending)}>{saving ? '保存中…' : '保存'}</button><button onClick={resolveDiscard} disabled={saving}>放弃</button><button onClick={() => { setPending(null); setSaveError(''); }} disabled={saving}>继续编辑</button></div>
    </div></div>}
    {deleteConfirm && !pending && <div className="dialog-backdrop"><div className="leave-dialog" role="dialog" aria-modal="true" aria-labelledby="delete-title">
      <h2 id="delete-title">{deleteConfirm.type === 'trash' ? '移入回收站？' : '永久删除？'}</h2>
      <p>{deleteConfirm.type === 'trash' ? '内容可以从回收站恢复。' : '永久删除后无法恢复，关联的快捷固定也会移除。'}</p>
      {actionError && <p role="alert" className="error-text">{actionError}</p>}
      <div className="dialog-actions"><button className="danger-button" onClick={() => void confirmDelete()}>{deleteConfirm.type === 'trash' ? '删除' : '永久删除'}</button><button onClick={() => setDeleteConfirm(null)}>取消</button></div>
    </div></div>}
  </main>;
}

createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
