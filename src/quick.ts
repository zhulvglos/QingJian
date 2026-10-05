import { invoke } from '@tauri-apps/api/core';
import { emitTo, listen } from '@tauri-apps/api/event';
import './quick.css';
import {applyBackgroundTransparency} from './background';
let backgroundTransparency=0;let appearanceTheme='warm_apricot';
function refreshBackground(){const c=palettes[appearanceTheme]??palettes.warm_apricot;applyBackgroundTransparency(backgroundTransparency,{tint:c[1],surface:c[2]});}

type Appearance = { theme: string; fontSize: string };
type Side = 'left' | 'right';
type Pin = { itemId: string; kind: 'sticky' | 'note'; title: string; sortOrder: number };

const palettes: Record<string, string[]> = {
  warm_apricot: ['#966032', '#ffebc2', '#fffdf9', '#e8dccb', '#302a24', '#776f65'],
  mint_morning: ['#24796a', '#d1f3e9', '#fcfffd', '#d5e8df', '#24342f', '#647b72'],
  new_leaf: ['#5d7332', '#e5f1c9', '#fffffb', '#e0e8cf', '#303526', '#717b61'],
  coral: ['#ac5149', '#ffded8', '#fffcfb', '#ecd9d4', '#392a29', '#836b67'],
  lilac_mist: ['#79548b', '#eadbf0', '#fefcff', '#e7dceb', '#322b36', '#786b7f'],
};

function applyAppearance({ theme, fontSize }: Appearance) {
  const colors = palettes[theme] ?? palettes.warm_apricot;
  ['accent', 'tint', 'surface', 'border', 'text', 'muted'].forEach((name, index) => {
    document.documentElement.style.setProperty('--' + name, colors[index]);
  });
  appearanceTheme=theme;document.documentElement.dataset.fontSize = fontSize;refreshBackground();
  requestAnimationFrame(()=>renderPins());
}

try {
  applyAppearance({ theme: localStorage.getItem('qingjian.shell.theme') ?? 'warm_apricot', fontSize: localStorage.getItem('qingjian.shell.fontSize') ?? 'standard' });
} catch { applyAppearance({ theme: 'warm_apricot', fontSize: 'standard' }); }

const root = document.querySelector<HTMLElement>('#root')!;
const tab = document.querySelector<HTMLButtonElement>('#quick-tab')!;
const panel = document.querySelector<HTMLElement>('#quick-panel')!;
const list = document.querySelector<HTMLElement>('#pin-list')!;
const empty = document.querySelector<HTMLElement>('#quick-empty')!;
const errorBox = document.querySelector<HTMLElement>('#quick-error')!;
const menu = document.querySelector<HTMLElement>('#pin-menu')!;
const remove = document.querySelector<HTMLButtonElement>('#pin-remove')!;
let pins: Pin[] = [];
let expanded = false;
let busy = false;
let menuId: string | null = null;
let lastMeasuredHeight = 0;
let compactWidth = 80;
let menuCard:HTMLButtonElement|null=null;
let menuBaseWidth=0;
let menuHeightReserved=false;
let renderDeferred=false;
const collapseCards=new Map<string,()=>void>();
function positionMenu(){
 if(!menuCard||menu.hidden)return;
 const r=menuCard.getBoundingClientRect(),m=menu.getBoundingClientRect(),left=document.documentElement.dataset.side==='left';
 let x=left?r.left-m.width-4:r.right+4,y=r.top;
 // 空间不足时放在标题下方；原生承载窗口已为菜单预留空间。
 if(x<0||x+m.width>root.clientWidth){
  x=Math.max(0,Math.min(r.left,root.clientWidth-m.width));y=r.bottom+4;
  if(y+m.height>root.clientHeight&&r.top>=m.height+4)y=r.top-m.height-4;
  // 通常无需增高窗口，避免底部附近的长列表因菜单而跳位；只为单行不足空间预留高度。
  if(y+m.height>root.clientHeight&&!menuHeightReserved){menuHeightReserved=true;void invoke('set_quick_menu_size',{width:m.width+4,height:m.height+4}).then(()=>requestAnimationFrame(positionMenu));}
 }
 menu.style.left=`${Math.max(0,Math.min(x,root.clientWidth-m.width))}px`;
 menu.style.top=`${Math.max(0,Math.min(y,root.clientHeight-m.height))}px`;
}
async function closeMenu(){
 if(!menuId)return;const id=menuId,card=menuCard;menuId=null;menuCard=null;menuHeightReserved=false;menu.hidden=true;
 await invoke('set_quick_menu_size',{width:0,height:0}).catch(()=>{});
 if(renderDeferred){renderDeferred=false;renderPins();}else{collapseCards.get(id)?.();if(card?.matches(":hover"))card.dispatchEvent(new MouseEvent("mouseenter"));}
}
function measureCompactWidth(){
  const sample=document.createElement('button');sample.className='pin-card';list.append(sample);
  const context=document.createElement('canvas').getContext('2d')!;
  context.font=getComputedStyle(sample).font;
  compactWidth=Math.ceil(context.measureText('四字标签…').width+18);sample.remove();
  document.documentElement.style.setProperty('--compact-width',`${compactWidth}px`);
  void invoke('set_quick_compact_width',{width:compactWidth}).catch(()=>{});
}

function reportContentHeight() {
  if (!expanded || panel.hidden) return;
  // 标签随标题换行增长，按真实卡片高度调整外侧窗口，超长列表由滚动区承接。
  const height = Math.min(540, Math.max(pins.length ? 58 : 118, [...list.querySelectorAll<HTMLElement>('.pin-row')]
    .reduce((sum, row) => sum + row.getBoundingClientRect().height + 6, 4)));
  if (Math.abs(height - lastMeasuredHeight) < 2) return;
  lastMeasuredHeight = height;
  void invoke('set_quick_content_height', { height }).catch((error) => showError(String(error)));
}

function showError(message: string) {
  errorBox.hidden = false;
  errorBox.textContent = message;
  window.setTimeout(() => { errorBox.hidden = true; }, 4500);
}

async function toggle(next: boolean) {
  if (busy) return;
  busy = true;
  try {
    await invoke('set_quick_expanded', { expanded: next });
    expanded = next;
    document.documentElement.dataset.expanded = String(expanded);
    panel.hidden = !expanded;
    tab.setAttribute('aria-expanded', String(expanded));
    menu.hidden = true;
    requestAnimationFrame(reportContentHeight);
  } catch (error) { showError(String(error)); }
  finally { busy = false; }
}

function renderPins() {
  if(menuId){renderDeferred=true;return;}
  measureCompactWidth();
  collapseCards.clear();
  list.replaceChildren();
  empty.hidden = pins.length > 0;
  for (const pin of pins) {
    const row = document.createElement('div');
    row.className = 'pin-row';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'pin-card';
    const fullTitle = pin.title.trim() || (pin.kind === 'sticky' ? '无标题便签' : '无标题笔记');
    // Segmenter 按字素簇处理组合字符、中文和 Emoji，不按 UTF-16 截断。
    const graphemes=Array.from(new (Intl as any).Segmenter('zh',{granularity:'grapheme'}).segment(fullTitle),(s:any)=>s.segment) as string[];
    const compact=graphemes.slice(0,4).join('')+(graphemes.length>4?'…':'');
    button.textContent=compact;button.dataset.fullTitle=fullTitle;
    let hoverTimer:number|undefined;
    // 快速经过不展开；点击仍只负责打开内容，窗口向外延展，不改变邻居位置。
    const collapse=()=>{clearTimeout(hoverTimer);if(menuId)return;button.classList.remove('hover-expanded');button.style.top='';button.textContent=compact;void invoke('set_quick_hover_width',{width:compactWidth}).catch(()=>{});};collapseCards.set(pin.itemId,collapse);
    const enter=()=>{if(menuId)return;clearTimeout(hoverTimer);hoverTimer=window.setTimeout(()=>{if(!button.isConnected||menuId)return;button.classList.add('hover-expanded');button.textContent=fullTitle;const context=document.createElement('canvas').getContext('2d')!;context.font=getComputedStyle(button).font;const width=Math.min(800,Math.max(compactWidth,context.measureText(fullTitle).width+18));void invoke('set_quick_hover_width',{width}).catch(e=>showError(String(e)));},300);};
    button.addEventListener('mouseenter',enter);
    button.addEventListener('mouseleave',()=>{clearTimeout(hoverTimer);if(menuId===pin.itemId)return;collapse();});
    button.title = '打开'  + (pin.kind === 'sticky' ? '便签' : '笔记') + '：' + fullTitle + '；右键移除固定';
    button.setAttribute('aria-label', button.title);
    button.addEventListener('click', () => {void closeMenu();void emitTo('main', 'quick-open-item', { itemId: pin.itemId });});
    button.addEventListener('keydown', (event) => { if (event.key === 'Delete') { event.preventDefault(); void unpin(pin.itemId); } });
    button.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      event.stopPropagation();clearTimeout(hoverTimer);
      menuId = pin.itemId;
      menuCard=button;menuBaseWidth=button.getBoundingClientRect().width;menuHeightReserved=false;
      menu.hidden = false;
      const m=menu.getBoundingClientRect();void invoke('set_quick_menu_size',{width:m.width+4,height:0}).then(()=>requestAnimationFrame(positionMenu)).catch(e=>showError(String(e)));
      requestAnimationFrame(positionMenu);
    });
    row.append(button);
    list.append(row);
  }
  requestAnimationFrame(reportContentHeight);
}

async function refreshPins() {
  try {
    pins = await invoke<Pin[]>('list_pins');
    renderPins();
    await invoke('set_quick_content_count', { count: pins.length });
  } catch (error) { showError('读取快捷标签失败：' + String(error)); }
}

async function unpin(id: string) {
  try {
    await invoke('unpin_item', { id });
    await closeMenu();
    await refreshPins();
    if (!pins.length) await toggle(false);
  } catch (error) { showError('移除失败：' + String(error)); }
}

tab.addEventListener('click', () => void toggle(true));
remove.addEventListener('click', () => { if (menuId) void unpin(menuId); });
document.addEventListener('pointerdown', (event) => { if(event.button===0&&!menu.contains(event.target as Node))void closeMenu(); });
document.addEventListener('keydown',event=>{if(event.key==='Escape'){event.preventDefault();void closeMenu();}});
window.addEventListener('blur',()=>void closeMenu());

// 跨 WebView 窗口的拖放由外侧窗口接收，数据库只记录原条目 ID。
root.addEventListener('dragenter', (event) => { event.preventDefault(); document.documentElement.dataset.dragOver = 'true'; });
root.addEventListener('dragover', (event) => { event.preventDefault(); if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'; document.documentElement.dataset.dragOver = 'true'; });
root.addEventListener('dragleave', (event) => { if (!root.contains(event.relatedTarget as Node)) document.documentElement.dataset.dragOver = 'false'; });
root.addEventListener('drop', async (event) => {
  event.preventDefault();
  document.documentElement.dataset.dragOver = 'false';
  document.documentElement.dataset.dragReady = 'false';
  const id = event.dataTransfer?.getData('application/x-qingjian-item') || event.dataTransfer?.getData('text/plain');
  if (!id) { showError('没有收到便签或笔记'); return; }
  try {
    await invoke('pin_item', { id });
    await refreshPins();
    await toggle(true);
  } catch (error) { showError('固定失败：' + String(error)); }
});

void listen<Appearance>('shell-appearance', (event) => applyAppearance(event.payload));
void listen<Side>('quick-side', (event) => { document.documentElement.dataset.side = event.payload; });
void listen<boolean>('quick-drag-active', (event) => { document.documentElement.dataset.dragReady = String(event.payload); });
new ResizeObserver(() => requestAnimationFrame(reportContentHeight)).observe(list);
void listen<boolean>('quick-items-changed', () => void refreshPins().then(() => { if (pins.length && !expanded) void toggle(true); }));
void invoke<Side>('get_quick_side').then((side) => { document.documentElement.dataset.side = side; });
void refreshPins().then(() => { if (pins.length) void toggle(true); });

void listen<number>('quick-width',e=>{document.documentElement.style.setProperty('--hover-width',`${menuId?menuBaseWidth:e.payload}px`);requestAnimationFrame(()=>{if(menuId){positionMenu();return;}const button=document.querySelector<HTMLElement>('.hover-expanded');if(button){const row=button.parentElement!;button.style.top=(-Math.min(row.offsetTop,Math.max(0,row.offsetTop+button.offsetHeight-root.clientHeight)))+'px';}});});

void listen<number>('background-transparency',e=>{backgroundTransparency=e.payload;refreshBackground();});
let backgroundAttempts=0;const readBackground=()=>void invoke<{transparency:number}>('get_shell_settings').then(s=>{backgroundTransparency=s.transparency||0;refreshBackground();}).catch(()=>{if(backgroundAttempts++<20)setTimeout(readBackground,100);});readBackground();
