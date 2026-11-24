'use strict';
const {parentPort,workerData}=require('worker_threads'),{DatabaseSync}=require('node:sqlite');
const db=new DatabaseSync(workerData.dbPath,{readOnly:true});db.exec('PRAGMA busy_timeout=5000; PRAGMA cache_size=-32768; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=268435456;');
function query(p){const terms=String(p.q||'').trim().toLowerCase().split(/\s+/).filter(Boolean).slice(0,20),where=[],args=[],scope=p.scope==='file-folder',sort=['az','za','newest','largest'].includes(p.sort)?p.sort:'az',offset=Math.max(0,Math.min(2000000,Number(p.offset)||0)),limit=Math.max(50,Math.min(500,Number(p.limit)||300));
 if(p.kind&&p.kind!=='all'){where.push('kind=?');args.push(p.kind)}for(const term of terms){const t='%'+term.replace(/[\\%_]/g,x=>'\\'+x)+'%';where.push(scope?"(search_name LIKE ? ESCAPE '\\' OR search_folder LIKE ? ESCAPE '\\')":"search_name LIKE ? ESCAPE '\\'");args.push(t);if(scope)args.push(t)}
 const w=where.length?' WHERE '+where.join(' AND '):'',order=sort==='za'?'name COLLATE NOCASE DESC,file_id':sort==='newest'?'mtime_ms DESC,file_id':sort==='largest'?'size DESC,file_id':'name COLLATE NOCASE ASC,file_id';
 const total=Number(db.prepare('SELECT COUNT(*) n FROM gallery_files'+w).get(...args).n),count=Number(db.prepare('SELECT COUNT(*) n FROM gallery_files').get().n);
 const rows=db.prepare('SELECT * FROM gallery_files'+w+' ORDER BY '+order+' LIMIT ?').all(...args,offset+limit).map(r=>({localId:r.local_id,fileId:r.file_id,name:r.name,ext:r.ext,size:Number(r.size),mtimeMs:Number(r.mtime_ms),kind:r.kind,previewable:!!r.previewable,root:r.root,relativePath:r.relative_path,searchName:r.search_name,searchFolder:r.search_folder,ownerDeviceId:r.device_id,ownerName:r.owner_name,ownerPc:r.owner_pc,sourceType:'pc'}));
 return{rows,total,count,offset,limit,sort};}
parentPort.on('message',m=>{
 if(m?.type==='close'){
  try{db.close()}catch{}
  parentPort.close();
  return;
 }
 try{parentPort.postMessage({id:m.id,ok:true,...query(m.payload||{})})}catch(e){parentPort.postMessage({id:m.id,ok:false,error:e.message})}
});
parentPort.postMessage({type:'ready'});
