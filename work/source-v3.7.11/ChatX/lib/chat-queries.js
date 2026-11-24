'use strict';
function createChatQueries(dbGetter,fallback){
 const parse=rows=>rows.map(r=>{try{return JSON.parse(r.payload_json)}catch{return null}}).filter(Boolean);
 const db=()=>dbGetter();
 function all(){const d=db();return d?parse(d.prepare('SELECT payload_json FROM chat_messages ORDER BY ts,id').all()):fallback();}
 function get(id){const d=db();if(d){const r=d.prepare('SELECT payload_json FROM chat_messages WHERE id=?').get(String(id));return r?JSON.parse(r.payload_json):null}return fallback().find(m=>m.id===id)||null}
 function page(conversationId,limit=120,beforeTs=Infinity,beforeId=''){
  limit=Math.max(20,Math.min(200,Number(limit)||120));const d=db();let items;
  if(d){let sql='SELECT payload_json FROM chat_messages WHERE conversation_id=?',args=[String(conversationId)];if(Number.isFinite(beforeTs)){sql+=beforeId?' AND (ts<? OR (ts=? AND id<?))':' AND ts<?';args.push(beforeTs);if(beforeId)args.push(beforeTs,String(beforeId));}sql+=' ORDER BY ts DESC,id DESC LIMIT ?';args.push(limit+1);items=parse(d.prepare(sql).all(...args));}
  else items=fallback().filter(m=>m.conversationId===conversationId&&(m.ts<beforeTs||(m.ts===beforeTs&&beforeId&&m.id<beforeId))).sort((a,b)=>b.ts-a.ts||b.id.localeCompare(a.id)).slice(0,limit+1);
  const hasMore=items.length>limit;items=items.slice(0,limit).reverse();return{items,hasMore,cursor:items.length?{ts:items[0].ts,id:items[0].id}:null};
 }
 function latest(id){return page(id,20).items.at(-1)||null}
 function unread(id,name,since){const d=db();if(d)return Number(d.prepare("SELECT COUNT(*) n FROM chat_messages WHERE conversation_id=? AND ts>? AND lower(trim(json_extract(payload_json,'$.sender')))<>?").get(String(id),Number(since||0),String(name).trim().toLowerCase()).n);return fallback().filter(m=>m.conversationId===id&&m.ts>since&&String(m.sender).toLowerCase()!==String(name).toLowerCase()).length}
 function attachments(){const d=db();return d?parse(d.prepare("SELECT payload_json FROM chat_messages WHERE json_extract(payload_json,'$.file.stored') IS NOT NULL ORDER BY ts DESC,id DESC").all()):fallback().filter(m=>m.file?.stored).slice().reverse()}
 function refs(stored){const d=db();return d?parse(d.prepare("SELECT payload_json FROM chat_messages WHERE json_extract(payload_json,'$.file.stored')=?").all(String(stored))):fallback().filter(m=>m.file?.stored===stored)}
 return{all,get,page,latest,unread,attachments,refs};
}
module.exports={createChatQueries};
