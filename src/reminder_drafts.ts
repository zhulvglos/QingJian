// 提醒草稿仅保留本次运行；切模式、切条目、卸载栏目均不会写库或丢失修改。
export type ReminderDraftOwner={key:string;id?:string;kind:'sticky'|'note'};
const pending=new Map<string,ReminderDraftOwner>();
export function markReminderDraft(owner:ReminderDraftOwner,dirty:boolean){if(dirty)pending.set(owner.key,owner);else pending.delete(owner.key);}
export function firstReminderDraft(){return pending.values().next().value as ReminderDraftOwner|undefined;}
export function discardReminderDrafts(){pending.clear();}
