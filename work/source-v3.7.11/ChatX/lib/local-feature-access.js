const fs=require('fs'),path=require('path'),http=require('http'),https=require('https');
const {cleanPermissions}=require('./feature-policy');
function createFeatureAccess({file,getConfig,isHost,onPolicy=()=>{}}){
 let session=null,cache=null,pending=null,lastRefresh=0;
 try{cache=JSON.parse(fs.readFileSync(file,'utf8'));}catch{}
 const account=()=>String(getConfig()?.name||'').trim().toLowerCase();
 function allowed(key){if(isHost())return true;return !cache||cache.name!==account()||(cache.permissions?.banned!==true&&cache.permissions?.[key]!==false);}
 function denied(key){return{ok:false,code:'FEATURE_DENIED',error:'Akses '+({designAI:'alat desain / AI',downloadFiles:'unduh file'}[key]||key)+' dibatasi HOST'};}
 function tokenFor(origin){return session?.origin===origin?session.token:'';}
 async function refresh(force=true){
  if(!session)return null;if(pending)return pending;if(!force&&Date.now()-lastRefresh<5000)return null;lastRefresh=Date.now();
  const active=session;
  pending=new Promise(resolve=>{
   const target=new URL('/permissions',active.origin),client=target.protocol==='https:'?https:http;
   const req=client.get(target,{headers:{Authorization:'Bearer '+active.token},timeout:2500},res=>{
    let body='';res.on('data',chunk=>{body+=chunk;if(body.length>64000)req.destroy(Error('Policy terlalu besar'));});
    res.on('error',()=>resolve(null));res.on('end',()=>{try{const j=JSON.parse(body);if(res.statusCode!==200||!j.ok||!j.policy||active!==session)return resolve(null);cache={origin:active.origin,name:String(j.profileName||active.name).trim().toLowerCase(),permissions:cleanPermissions(j.policy.permissions)};fs.mkdirSync(path.dirname(file),{recursive:true});const tmp=file+'.tmp';fs.writeFileSync(tmp,JSON.stringify(cache));fs.renameSync(tmp,file);onPolicy(cache.permissions);resolve(j.policy);}catch{resolve(null)}});
   });req.on('timeout',()=>req.destroy(Error('Policy timeout')));req.on('error',()=>resolve(null));
  }).finally(()=>pending=null);
  return pending;
 }
 function setSession(raw){const origin=new URL(String(raw?.origin||'')).origin;if(!/^https?:/.test(origin)||!getConfig()?.trustedHostKeys?.[origin]&&!isHost())return{ok:false,error:'HOST belum dipercaya'};const token=String(raw?.token||'');if(!token||token.length>8192)return{ok:false,error:'Sesi tidak valid'};session={origin,token,name:account()};lastRefresh=0;if(pending)pending.finally(()=>refresh());else refresh();return{ok:true};}
 async function check(key){await refresh(false);return allowed(key)?null:denied(key);}
 return{setSession,refresh,allowed,denied,check,tokenFor};
}
module.exports={createFeatureAccess};
