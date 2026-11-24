'use strict';
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const inside=(root,p)=>{const r=path.relative(path.resolve(root),path.resolve(p));return r!==''&&!r.startsWith('..'+path.sep)&&r!=='..'&&!path.isAbsolute(r);};
async function compact({userData,dryRun=true}){
 const root=path.resolve(userData),candidates=[],warnings=[];
 if(!dryRun)for(const entry of await fs.promises.readdir(root,{withFileTypes:true})){if(entry.isFile()&&/^\.kc-backup-link-[a-f0-9-]{36}$/.test(entry.name)){const file=path.join(root,entry.name);if(!(await fs.promises.lstat(file)).isSymbolicLink())try{await fs.promises.unlink(file);}catch{warnings.push('File sementara optimasi lama masih terkunci.');}}}
 for(const [folder,pattern,manifest]of [['pre-update-backups',/^ChatX-Backup-[\dTZ.-]+$/,'BACKUP_INFO.json'],['recovery',/^checkpoint-(host|client)-\d+\.\d+\.\d+$/,'CHECKPOINT.json']]){
  const base=path.join(root,folder);if(!fs.existsSync(base))continue;if((await fs.promises.lstat(base)).isSymbolicLink())throw Error('Folder backup berupa junction; optimasi dibatalkan.');
  for(const entry of await fs.promises.readdir(base,{withFileTypes:true})){if(!entry.isDirectory()||!pattern.test(entry.name))continue;const dir=path.join(base,entry.name);if((await fs.promises.lstat(dir)).isSymbolicLink())continue;
   try{const m=JSON.parse(await fs.promises.readFile(path.join(dir,manifest),'utf8'));if(!Array.isArray(m.files)||(manifest==='CHECKPOINT.json'?!m.completed:m.format!==2))continue;
    const snapshotRows=[];for(const row of m.files){if(!/^[a-f0-9]{64}$/.test(row.sha256)||!Number.isSafeInteger(row.size)||row.size<65536)continue;const file=path.resolve(dir,row.path);if(!inside(dir,file))throw Error('Path manifest tidak aman');let parent=path.dirname(file);while(parent!==dir){if((await fs.promises.lstat(parent)).isSymbolicLink())throw Error('Junction di dalam backup ditolak');parent=path.dirname(parent);}const st=await fs.promises.lstat(file);if(!st.isFile()||st.isSymbolicLink()||st.size!==row.size)throw Error('File backup berubah');snapshotRows.push({file,sha256:row.sha256,size:row.size,st});}candidates.push(...snapshotRows);
   }catch{warnings.push('Backup '+entry.name+' dilewati karena manifest atau file tidak valid.');}
  }
 }
 const groups=new Map();for(const row of candidates){const key=row.sha256+'|'+row.size+'|'+row.st.dev;if(!groups.has(key))groups.set(key,[]);groups.get(key).push(row);}
 let savedBytes=0,files=0,checked=0;
 const digest=async file=>{const h=crypto.createHash('sha256');for await(const b of fs.createReadStream(file))h.update(b);return h.digest('hex');};
 for(const rows of groups.values()){if(rows.length<2)continue;let first=null;for(const row of rows){if(first&&row.st.ino===first.st.ino&&row.st.dev===first.st.dev)continue;if(await digest(row.file)!==row.sha256){warnings.push('Checksum backup berbeda; file dilewati.');continue;}checked++;if(!first){first=row;continue;}if(row.st.nlink>1)continue;
   if(!dryRun){const tmp=path.join(root,'.kc-backup-link-'+crypto.randomUUID());try{await fs.promises.link(first.file,tmp);const current=await fs.promises.lstat(row.file);if(current.size!==row.st.size||current.mtimeMs!==row.st.mtimeMs)throw Error('Backup berubah saat optimasi');await fs.promises.rename(tmp,row.file);}catch(e){warnings.push('Sebagian file tidak bisa dihemat ('+(e.code||'berubah')+'). Salinan asli dipertahankan.');continue;}finally{try{await fs.promises.unlink(tmp);}catch{}}}
   savedBytes+=row.size;files++;
  }}
 return{ok:true,dryRun,savedBytes,files,checked,warnings:[...new Set(warnings)].slice(0,12),method:'shared-immutable-backup-files'};
}
module.exports={compact,inside};
