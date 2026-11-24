(function(root,factory){
  const api=factory();
  if(typeof module==='object'&&module.exports)module.exports=api;
  if(root)root.KCServerPicker=api;
})(typeof window!=='undefined'?window:globalThis,function(){
  function validAddress(value){
    try{const u=new URL(String(value||''));return(u.protocol==='http:'||u.protocol==='https:')&&!!u.hostname&&u.port==='45820'&&!u.username&&!u.password;}catch{return false}
  }
  function chooseTarget({hostMode=false,hostAddress='',manualAddress='',discoveredAddress=''}={}){
    if(hostMode&&validAddress(hostAddress))return String(hostAddress).trim();
    if(!hostMode&&validAddress(manualAddress))return String(manualAddress).trim();
    return validAddress(discoveredAddress)?String(discoveredAddress).trim():'';
  }
  function reconcileManualSelection({manualAddress='',manualPcName='',servers=[]}={}){
    const name=String(manualPcName||'').trim().toLocaleLowerCase('id');
    const match=name&&(Array.isArray(servers)?servers:[]).find(row=>String(row?.pcName||'').trim().toLocaleLowerCase('id')===name&&validAddress(row?.address));
    return match?{address:String(match.address).trim(),pcName:String(match.pcName||'').trim(),changed:!sameServer(match.address,manualAddress)}:{address:String(manualAddress||''),pcName:String(manualPcName||''),changed:false};
  }
  function sameServer(a,b){
    try{return new URL(a).origin===new URL(b).origin}catch{return false}
  }
  return{validAddress,chooseTarget,reconcileManualSelection,sameServer};
});
