'use strict';

function validAddress(value){try{const u=new URL(String(value||''));return(u.protocol==='http:'||u.protocol==='https:')&&u.port==='45820'&&!u.username&&!u.password;}catch{return false}}
function preferPrimaryServer(servers=[]){
  const rows=(Array.isArray(servers)?servers:[]).filter(row=>validAddress(row?.address));
  const primary=rows.filter(row=>row.roleKnown===true&&row.temporary===false).sort((a,b)=>String(a.pcName||a.address).localeCompare(String(b.pcName||b.address),'id',{sensitivity:'base'}));
  return primary[0]?.address||'';
}
// v2.2.13: the application is HOST-only for server authority. CLIENT builds
// must never self-elect a temporary server, even if an old config enables it.
function mayStartTemporaryServer(){return false}
function mayMoveClientFromTemporary({connectedTemporary=false,primaryFound=false,handoverConfirmed=false}={}){return connectedTemporary===true&&primaryFound===true&&handoverConfirmed===true}
module.exports={validAddress,preferPrimaryServer,mayStartTemporaryServer,mayMoveClientFromTemporary};
