// 此处仅还原主题原色；整窗透明度由 Windows 原生层统一应用一次。
export const backgroundKeys=['canvas','surface','panel','panel-soft','tint'];
export function applyBackgroundTransparency(value:number,colors?:Record<string,string>){
 const element=document.documentElement;
 if(!colors)backgroundKeys.forEach(key=>element.style.removeProperty('--'+key));
 const style=getComputedStyle(element);
 const raw=Object.fromEntries(backgroundKeys.map(key=>[key,colors?.[key]||style.getPropertyValue('--'+key).trim()]));
 // WebView 内部保持主题原色；由原生顶层窗口对最终结果应用一次透明度。
 for(const [key,color] of Object.entries(raw)){if(!color)continue;element.style.setProperty('--'+key,color);}
 element.style.setProperty('--accent-bg',style.getPropertyValue('--accent').trim());
 element.dataset.transparency=String(value);
}
