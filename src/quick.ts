import { invoke } from '@tauri-apps/api/core';
import { emitTo, listen } from '@tauri-apps/api/event';
import './quick.css';

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
  document.documentElement.dataset.fontSize = fontSize;
}

try {
  applyAppearance({ theme: localStorage.getItem('qingjian.shell.theme') ?? 'warm_apricot', fontSize: localStorage.getItem('qingjian.shell.fontSize') ?? 'standard' });
} catch { applyAppearance({ theme: 'warm_apricot', fontSize: 'standard' }); }

const root = document.querySelector<HTMLElement>('#root')!;
const tab = document.querySelector<HTMLButtonElement>('#quick-tab')!;
const panel = document.querySelector<HTMLElement>('#quick-panel')!;
const close = document.querySelector<HTMLButtonElement>('#quick-close')!;
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
  list.replaceChildren();
  empty.hidden = pins.length > 0;
  for (const pin of pins) {
    const row = document.createElement('div');
    row.className = 'pin-row';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'pin-card';
    button.textContent = pin.title.trim() || (pin.kind === 'sticky' ? '无标题便签' : '无标题笔记');
    button.title = '打开' + (pin.kind === 'sticky' ? '便签' : '笔记') + '：' + button.textContent + '；右键移除固定';
    button.setAttribute('aria-label', button.title);
    button.addEventListener('click', () => void emitTo('main', 'quick-open-item', { itemId: pin.itemId }));
    button.addEventListener('keydown', (event) => { if (event.key === 'Delete') { event.preventDefault(); void unpin(pin.itemId); } });
    button.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      menuId = pin.itemId;
      menu.hidden = false;
      menu.style.top = Math.min(row.offsetTop + row.offsetHeight - list.scrollTop, root.clientHeight - 42) + 'px';
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
    menu.hidden = true;
    await refreshPins();
    if (!pins.length) await toggle(false);
  } catch (error) { showError('移除失败：' + String(error)); }
}

tab.addEventListener('click', () => void toggle(true));
close.addEventListener('click', () => void toggle(false));
remove.addEventListener('click', () => { if (menuId) void unpin(menuId); });
document.addEventListener('click', (event) => { if (!menu.contains(event.target as Node)) menu.hidden = true; });

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
