import { useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { EditorContent, useEditor } from '@tiptap/react';
import type { JSONContent } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import { TaskItem, TaskList } from '@tiptap/extension-list';

type Mode = 'paragraph' | 'checklist';
type StoredDocument = { format: 'qingjian-rich-v1'; mode: Mode; document: JSONContent; checks: boolean[] };

// 提醒视图仅展示数据库快照，不注册编辑回调；切换记录时由父组件的 key 重建。
export function ReadOnlyBody({body, bodyJson}: {body: string; bodyJson: string | null}) {
  const editor = useEditor({
    extensions: [StarterKit, TaskList, TaskItem.configure({nested: true})],
    content: readDocument(body, bodyJson).document,
    editable: false,
  });
  return <div className="rich-body readonly-body"><EditorContent className="rich-editor" editor={editor} aria-label="已保存正文，只读" /></div>;
}

function paragraph(content?: JSONContent[]): JSONContent {
  return { type: 'paragraph', ...(content?.length ? { content } : {}) };
}

export function readDocument(body: string, bodyJson?: string | null): StoredDocument {
  if (bodyJson) {
    try {
      const stored = JSON.parse(bodyJson) as StoredDocument;
      if (stored.format === 'qingjian-rich-v1' && stored.document?.type === 'doc' && Array.isArray(stored.document.content)) return stored;
    } catch { /* 损坏时使用保留的纯文本副本，避免编辑器空白。 */ }
  }
  // 旧纯文本只在内存中解析；行首方框转为真实勾选项，原数据库正文不改写。
  const blocks: JSONContent[] = [];
  let tasks: JSONContent[] = [];
  const flush = () => { if (tasks.length) { blocks.push({ type: 'taskList', content: tasks }); tasks = []; } };
  for (const line of body.split('\n')) {
    if (/^[☐☑]/.test(line)) {
      const text = line.slice(1).trimStart();
      tasks.push(taskItem(paragraph(text ? [{ type: 'text', text }] : undefined), line[0] === '☑'));
    } else {
      flush();
      blocks.push(paragraph(line ? [{ type: 'text', text: line }] : undefined));
    }
  }
  flush();
  return { format: 'qingjian-rich-v1', mode: blocks.some((node) => node.type === 'taskList') ? 'checklist' : 'paragraph',
    document: { type: 'doc', content: blocks }, checks: [] };
}

function inlineText(node: JSONContent): string {
  if (node.type === 'text') return node.text ?? '';
  if (node.type === 'hardBreak') return '\n';
  return (node.content ?? []).map(inlineText).join('');
}

export function documentText(document: JSONContent): string {
  // 导入的普通列表与嵌套段落同样保留行分隔，编辑保存后仍可读、可搜索。
  const blockText = (node: JSONContent): string => {
    if (['bulletList','orderedList','taskList','listItem','taskItem','blockquote','doc'].includes(node.type ?? '')) return (node.content ?? []).map(blockText).join('\n');
    return inlineText(node);
  };
  const lines: string[] = [];
  for (const node of document.content ?? []) {
    if (node.type === 'taskList') {
      for (const item of node.content ?? []) lines.push((item.content ?? []).map(inlineText).join('\n'));
    } else {
      lines.push(blockText(node));
    }
  }
  return lines.join('\n');
}

export function taskEntries(body: string, bodyJson?: string | null): { text: string; checked: boolean; index: number }[] {
  const stored = readDocument(body, bodyJson);
  if (stored.mode === 'checklist') {
    return (stored.document.content ?? []).flatMap((node) => node.type === 'taskList' ? (node.content ?? []).map((item) => ({
      text: (item.content ?? []).map(inlineText).join('\n').trim(),
      checked: item.attrs?.checked === true,
      index: 0,
    })) : []).map((entry, index) => ({ ...entry, index })).filter((entry) => entry.text.length > 0);
  }
  // 兼容旧纯文本中已有的行首待办标记；普通段落不会被误当成任务。
  return body.split('\n').flatMap((line, index) => /^[☐☑]/.test(line) ? [{ text: line.slice(1).trim(), checked: line[0] === '☑', index }] : []);
}

function taskItem(block: JSONContent, checked: boolean): JSONContent {
  const content = block.type === 'paragraph' ? [block] : [paragraph(block.content)];
  return { type: 'taskItem', attrs: { checked }, content };
}

function asChecklist(document: JSONContent, checks: boolean[]): JSONContent {
  const items: JSONContent[] = [];
  for (const block of document.content ?? []) {
    if (block.type === 'taskList') {
      items.push(...(block.content ?? []));
    } else {
      items.push(taskItem(block, checks[items.length] ?? false));
    }
  }
  if (!items.length) items.push(taskItem(paragraph(), false));
  return { type: 'doc', content: [{ type: 'taskList', content: items }] };
}

function asParagraphs(document: JSONContent): { document: JSONContent; checks: boolean[] } {
  const blocks: JSONContent[] = [];
  const checks: boolean[] = [];
  for (const block of document.content ?? []) {
    if (block.type === 'taskList') {
      for (const item of block.content ?? []) {
        checks.push(item.attrs?.checked === true);
        blocks.push(...(item.content?.length ? item.content : [paragraph()]));
      }
    } else {
      blocks.push(block);
      checks.push(false);
    }
  }
  return { document: { type: 'doc', content: blocks.length ? blocks : [paragraph()] }, checks };
}

function listIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="4" cy="5" r="1.2" fill="currentColor"/><circle cx="4" cy="12" r="1.2" fill="currentColor"/><circle cx="4" cy="19" r="1.2" fill="currentColor"/><path d="M8 5h13M8 12h13M8 19h13" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round"/></svg>;
}

function singleListIcon() {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2.5" y="8" width="5" height="5" rx="1" fill="none" stroke="currentColor" strokeWidth="1.5"/><path d="M10 10.5h11" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round"/></svg>;
}

function checkIcon(checked: boolean) {
  return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2" fill={checked ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="1.6"/>{checked && <path d="m7 12 3.2 3.2L17 8" fill="none" stroke="var(--surface)" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"/>}</svg>;
}

export function RichBodyEditor({ body, bodyJson, onChange, toolsTarget }: { body: string; bodyJson?: string | null; onChange: (body: string, bodyJson: string) => void; toolsTarget: HTMLElement | null }) {
  const initial = useRef(readDocument(body, bodyJson)).current;
  const checksRef = useRef(initial.checks);
  const [filter, setFilter] = useState<'all' | 'checked' | 'unchecked'>('all');
  const callbackRef = useRef(onChange);
  callbackRef.current = onChange;

  const editor = useEditor({
    // 避免编辑器在列表尾部自动补段落，反复转换时累积空白勾选项。
    extensions: [StarterKit.configure({ trailingNode: false }), TaskList, TaskItem.configure({
      a11y: { checkboxLabel: (_node, checked) => checked ? '标记为未完成' : '标记为完成' },
    })],
    content: initial.document,
    onUpdate: ({ editor: current }) => {
      const document = current.getJSON();
      checksRef.current = (document.content ?? []).flatMap((node) => node.type === 'taskList' ? (node.content ?? []).map((item) => (item as JSONContent).attrs?.checked === true) : []);
      const mode: Mode = (document.content ?? []).some((node) => node.type === 'taskList') ? 'checklist' : 'paragraph';
      callbackRef.current(documentText(document), JSON.stringify({ format: 'qingjian-rich-v1', mode, document, checks: checksRef.current } satisfies StoredDocument));
    },
  });

  const convertAll = () => {
    if (!editor) return;
    const document = editor.getJSON();
    // 编辑器自动附加的尾部空段落不改变“全文已是勾选列表”的判断。
    const meaningful = (document.content ?? []).filter((node) => !(node.type === 'paragraph' && !node.content?.length));
    const allTasks = meaningful.length > 0 && meaningful.every((node) => node.type === 'taskList');
    const next = allTasks ? asParagraphs(document).document : asChecklist(document, []);
    setFilter('all');
    editor.commands.setContent(next, { emitUpdate: true });
    editor.commands.focus();
  };

  const convertCurrent = () => {
    if (!editor) return;
    // 折叠到原光标段落，已有任务仅提起当前项，不解除同组其他项。
    const at = editor.state.selection.head;
    setFilter('all');
    const chain = editor.chain().focus().setTextSelection(at);
    if (editor.isActive('taskItem')) chain.liftListItem('taskItem').run();
    else chain.toggleTaskList().run();
  };

  const tools = <div className="detail-tools" role="toolbar" aria-label="正文操作" onMouseDown={(event) => event.preventDefault()}>
    <button type="button" className={'tool-icon ' + (filter === 'checked' ? 'active' : '')} onClick={() => setFilter(filter === 'checked' ? 'all' : 'checked')} title="只显示已勾选条目；再次点击显示全部" aria-label="只显示已勾选条目" aria-pressed={filter === 'checked'}>{checkIcon(true)}</button>
    <button type="button" className={'tool-icon ' + (filter === 'unchecked' ? 'active' : '')} onClick={() => setFilter(filter === 'unchecked' ? 'all' : 'unchecked')} title="只显示未勾选条目；再次点击显示全部" aria-label="只显示未勾选条目" aria-pressed={filter === 'unchecked'}>{checkIcon(false)}</button>
    <button type="button" className="tool-icon" onClick={convertAll} title="全文：勾选列表 / 普通段落" aria-label="切换全文勾选列表">{listIcon()}</button>
    <button type="button" className="tool-icon" onClick={convertCurrent} title="当前段落：勾选条目 / 普通段落" aria-label="切换当前段落勾选条目">{singleListIcon()}</button>
  </div>;

  return <div className="rich-body">
    {toolsTarget && createPortal(tools, toolsTarget)}
    <EditorContent className={'rich-editor filter-' + filter} editor={editor} aria-label="正文" />
  </div>;
}
