'use strict';
const fs=require('fs'),path=require('path'),crypto=require('crypto');
const {writeJSON}=require('./atomic-file');
function createDeviceAccess(dataDir,adminToken,options={}){
 const file=path.join(dataDir,'approved-devices.json');let devices={};try{devices=JSON.parse(fs.readFileSync(file,'utf8'))}catch(e){if(e.code!=='ENOENT')throw new Error('Registry perangkat tidak terbaca; pulihkan backup: '+e.message)}
 const pending=new Map(),sessions=new Map();const digest=s=>crypto.createHash('sha256').update(String(s||'')).digest('hex');
 function same(a,b){const x=Buffer.from(String(a||'')),y=Buffer.from(String(b||''));return x.length===y.length&&crypto.timingSafeEqual(x,y)}
 function login(raw={}){
  const deviceId=String(raw.deviceId||'').slice(0,120),name=String(raw.name||'').trim().slice(0,40),pc=String(raw.pc||'').slice(0,80);
  if(same(raw.adminToken,adminToken))return{ok:true,user:{deviceId:deviceId||'host',name:name||'HOST',pc,isHost:true},token:adminToken};
  if(!deviceId||!name)return{ok:false,error:'Identitas perangkat tidak valid.',code:'INVALID_DEVICE'};
  if(!/^[a-f0-9]{64}$/i.test(String(raw.deviceKey||''))){
   if(devices[deviceId]?.revoked)return{ok:false,error:'Akses perangkat dicabut oleh HOST.',code:'REVOKED'};
   // v1.13 clients have no device key. Keep the original zero-pairing flow;
   // revoked devices remain blocked by the HOST.
   const user={deviceId,name,pc,isHost:false,legacy:true};const token=crypto.randomBytes(32).toString('hex');sessions.set(token,{...user,expiresAt:Date.now()+24*3600000});return{ok:true,user,token};
  }
  const keyHash=digest(raw.deviceKey);let record=devices[deviceId];
  if(record?.revoked)return{ok:false,error:'Akses perangkat dicabut oleh HOST.',code:'REVOKED'};
  if(!record){
   record=devices[deviceId]={deviceId,name,pc,keyHash,registeredAt:Date.now(),revoked:false,autoRegistered:true};writeJSON(file,devices);
  }
  if(!same(record.keyHash,keyHash)){
   // A reinstalled CLIENT may generate a fresh key while retaining its stable
   // deviceId. Replace it automatically unless the HOST explicitly revoked it.
   record.keyHash=keyHash;record.name=name;record.pc=pc;record.keyUpdatedAt=Date.now();writeJSON(file,devices);
  }
  const user={deviceId,name:record.name||name,pc,isHost:false};const token=crypto.randomBytes(32).toString('hex');sessions.set(token,{...user,expiresAt:Date.now()+24*3600000});return{ok:true,user,token};
 }
 function resolve(req){const token=String(req.headers?.authorization||'').replace(/^Bearer /,'')||String(req.query?.kc_token||'');if(same(token,adminToken))return{isHost:true,name:'HOST',deviceId:'host'};const s=sessions.get(token);if(s&&s.expiresAt>=Date.now()&&!devices[s.deviceId]?.revoked)return s;if(s)sessions.delete(token);try{return options.resolveLegacyHttp?.(req)||null}catch{return null}}
 function list(){return{devices:Object.values(devices).map(({keyHash,...r})=>r),pending:[...pending.values()].map(({keyHash,...r})=>r)}}
 function decide({requestId,deviceId,approve,name,remove}){
  if(requestId){const p=pending.get(String(requestId));if(!p)throw Error('Permintaan tidak ditemukan');if(approve){devices[p.deviceId]={deviceId:p.deviceId,name:String(name||p.name).trim().slice(0,40),pc:p.pc,keyHash:p.keyHash,approvedAt:Date.now(),revoked:false};writeJSON(file,devices)}pending.delete(String(requestId));return p.deviceId;}
  const d=devices[String(deviceId)];if(!d)throw Error('Perangkat tidak ditemukan');if(remove)delete devices[String(deviceId)];else d.revoked=true;writeJSON(file,devices);for(const [token,s]of sessions)if(s.deviceId===deviceId)sessions.delete(token);return deviceId;
 }
 function prune(){for(const[k,s]of sessions)if(s.expiresAt<Date.now())sessions.delete(k);for(const[k,p]of pending)if(Date.now()-p.requestedAt>3600000)pending.delete(k)}
 return{login,resolve,list,decide,prune};
}
module.exports={createDeviceAccess,writeJSON};
