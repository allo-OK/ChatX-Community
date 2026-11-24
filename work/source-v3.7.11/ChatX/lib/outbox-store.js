'use strict';
const fs=require('fs'),path=require('path'),crypto=require('crypto'),{writeJSON}=require('./device-access');
function createOutboxStore(root){fs.mkdirSync(root,{recursive:true});function file(scope){return path.join(root,crypto.createHash('sha256').update(scope).digest('hex')+'.json')}function read(scope){try{const a=JSON.parse(fs.readFileSync(file(scope),'utf8'));if(!Array.isArray(a))throw Error('Format outbox rusak');return a}catch(e){if(e.code==='ENOENT')return[];throw e}}
 function mergeScopes(target,sourceScopes){
  const sources=[...new Set(sourceScopes)].filter(scope=>scope!==target),rows=read(target),byId=new Map(rows.map(row=>[row.payload.clientMessageId,row]));let migrated=0;
  for(const source of sources)for(const row of read(source)){
    const id=row?.payload?.clientMessageId;if(!id)throw Error('Pesan antrean lama tidak memiliki ID');
    const existing=byId.get(id);if(existing){if(JSON.stringify(existing.payload)!==JSON.stringify(row.payload))throw Error('ID antrean bertabrakan; data lama dipertahankan');continue;}
    rows.push(row);byId.set(id,row);migrated++;
  }
  if(rows.length>1000)throw Error('Antrean gabungan melebihi batas; data lama dipertahankan');
  if(migrated||sources.some(scope=>fs.existsSync(file(scope)))){rows.sort((a,b)=>Number(a.createdAt||0)-Number(b.createdAt||0));writeJSON(file(target),rows);for(const source of sources)try{fs.unlinkSync(file(source))}catch{}}
  return {ok:true,migrated};
 }
 return{list:read,mergeScopes,put(scope,payload){const rows=read(scope);const id=String(payload.clientMessageId||'');if(!/^[A-Za-z0-9_-]{12,100}$/.test(id))throw Error('ID pesan tidak valid');if(rows.some(r=>r.payload.clientMessageId===id))return;if(rows.length>=1000)throw Error('Outbox penuh. Selesaikan antrean dahulu.');rows.push({payload,createdAt:Date.now(),state:'waiting',attempts:0,error:''});writeJSON(file(scope),rows)},update(scope,id,patch){const rows=read(scope);const row=rows.find(r=>r.payload.clientMessageId===id);if(row)Object.assign(row,patch);writeJSON(file(scope),rows)},remove(scope,id){writeJSON(file(scope),read(scope).filter(r=>r.payload.clientMessageId!==id))}};
}
module.exports={createOutboxStore};
