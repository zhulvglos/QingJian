export function NavIcon({name}:{name:string}) {
 const paths:Record<string,string>={news:'M4 4h16v16H4zM7 8h10M7 12h4M7 16h10M15 12h2',stickies:'M4 4h16v11l-5 5H4zM15 15h5M15 15v5',notes:'M5 3h14v18H5zM8 7h8M8 11h8M8 15h6',settings:'M12 3v3M12 18v3M3 12h3M18 12h3M6 6l2 2M16 16l2 2M6 18l2-2M16 8l2-2',new:'M12 4v16M4 12h16',trash:'M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7',bell:'M5 17h14l-2-3V9a5 5 0 0 0-10 0v5zM10 20h4',all:'M4 5h16M4 12h16M4 19h16'};
 return <svg viewBox="0 0 24 24" aria-hidden="true"><path d={paths[name]||paths.all} fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"/>{name==='settings'&&<circle cx="12" cy="12" r="4" fill="none" stroke="currentColor" strokeWidth="1.7"/>}</svg>;
}
