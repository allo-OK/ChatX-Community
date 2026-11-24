'use strict';
// Keep popup HTML/media in memory instead of embedding it in a multi-megabyte navigation URL.
const stores=new WeakMap();
function prepare(win,html){const session=win.webContents.session;if(!session?.protocol?.handle)throw Error('Sesi popup tidak mendukung halaman notifikasi');let store=stores.get(session);if(!store){store=new Map();session.protocol.handle('kc-notification',request=>{let key;try{const u=new URL(request.url);if(u.hostname!=='popup')return new Response('',{status:404});key=u.pathname.slice(1);}catch{return new Response('',{status:400});}const page=store.get(key);return new Response(page||'',{status:page?200:404,headers:{'Content-Type':'text/html; charset=utf-8','Cache-Control':'no-store'}});});stores.set(session,store);}const id=require('crypto').randomUUID();store.set(id,String(html));const dispose=()=>store.delete(id);win.once('closed',dispose);return{url:'kc-notification://popup/'+id,dispose};}
module.exports={prepare};
